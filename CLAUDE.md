# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

AskDock (package `askdock`, repo dir `askDock`) is a Tauri 2 desktop app: a small
always-on-top frameless dock that sits next to the active terminal window. **The core
idea: it auto-captures the questions you ask AI coding tools (Claude Code, Codex) in the
terminal and groups them by terminal window.** When you switch back to a terminal window,
the dock shows the questions you asked AI *in that window* — so you remember what you were
doing there. **There is no manual input** — the dock is a read-only per-window log; you
never type into it.

It does **not** read terminal contents. It reads two things: (1) window geometry/identity
(to position itself and key questions by window), and (2) the AI tools' own session
transcript files on disk (to harvest your prompts). macOS is the first-class platform;
window detection is macOS-only.

UI strings are in Chinese. The original (now superseded) product spec is
`questiondock-tauri-mvp.md` — note it describes an earlier manual-notes concept; the app
has since pivoted to automatic AI-question capture.

## Commands

```bash
npm run dev          # Vite dev server only — browser preview at http://127.0.0.1:1420
npm run tauri dev    # Full app: Rust backend + webview (use this to exercise native features)
npm run build        # tsc typecheck + vite build → dist/
npm run tauri build  # Production app bundle

cd src-tauri && cargo test   # Rust unit tests (e.g. calculate_dock_rect)
cargo test calculate_dock_rect   # run a single Rust test by name
```

There is no JS test runner and no linter configured. `npm run build` (which runs `tsc`)
is the only frontend check.

## Architecture

Two halves talk over Tauri's `invoke` bridge:

- **Frontend** — `src/main.tsx` (one `App` component, no router/state library) +
  `src/styles.css`. Icons from `lucide-react`. React 19 + Vite 7.
- **Backend** — `src-tauri/src/lib.rs` holds all logic; `main.rs` only calls
  `askdock_lib::run()`. Rust + Tauri 2, SQLite via `rusqlite` (bundled).

### Dual runtime — the most important thing to know

`src/main.tsx` runs in **two** environments and abstracts them behind one `command<T>()`
helper:

- Inside Tauri (`window.__TAURI_INTERNALS__.invoke` exists) → real `invoke()` to Rust.
- In a plain browser (`npm run dev` opened in a browser) → `browserCommand()`, a
  **localStorage-backed mock** that reimplements every backend command in JS.

**When you add or change a Tauri command, update BOTH sides** — the Rust
`#[tauri::command]` in `lib.rs` *and* its branch in `browserCommand()` — or browser
preview silently diverges from the real app.

### Backend commands (registered in `invoke_handler!` in lib.rs)

`poll_front_window`, `get_window_questions`, `delete_question`, `clear_window`,
`set_layout`, `show_main_window`, `hide_main_window`.

The frontend Settings view (gear button in the footer) holds theme/typography prefs in
**localStorage** (`askdock.theme` / `.fontFace` / `.fontSize`), applied via `data-theme` /
`data-face` / `data-size` attributes on `<html>` against CSS-variable themes in
`styles.css`. **Dock placement** is the part that hits the backend: `set_layout(side,
width, height, follow)` persists `attach_side` / `dock_width` / `dock_height` /
`follow_terminal` to the `settings` table and re-runs `reposition_dock`. `side` is
`right|left|top|bottom`; `follow` makes the dimension along the attach edge track the
terminal (height for left/right, width for top/bottom) — `calculate_dock_rect` (pure,
unit-tested) computes the final rect and clamps it on-screen. The frontend debounces
`set_layout` (~250ms) so dragging the width/height number fields isn't jumpy.

The frontend is **read-only** and runs one **poll loop** (`useEffect` in `App`, every
~1200ms): it calls `poll_front_window({ reposition })`. The returned `window_id` is the
macOS CGWindowID of the frontmost terminal window; it also repositions the dock and
records the window as the "last seen terminal" (for capture attribution). When the
`window_id` changes, the frontend calls `get_window_questions(window_id)` to show that
window's captured questions. It also listens for the backend's `questions-updated`
Tauri **event** to refresh the list the instant a new question is captured.

### AI-question capture (the heart of the app)

A background thread (`run_capture_loop`, spawned in `setup`) polls every ~800ms over the
AI tools' session transcripts and harvests new user prompts:

- **Sources** (`capture_sources`): Claude Code `~/.claude/projects/**/*.jsonl` and Codex
  `~/.codex/sessions/**/*.jsonl`. Both are JSONL; add more sources here.
- **Only going forward**: at startup, every existing file's read offset is set to its
  current EOF, so historical conversations are *not* imported. Each loop reads only newly
  appended **complete** lines (tracked per-file byte offset; a trailing partial line waits
  for next pass).
- **Parsing/filtering** (`parse_claude_line`, `parse_codex_line`, `is_real_prompt`):
  extracts the user's text (string content, or joined `text`/`input_text` blocks) and
  drops noise — slash-command wrappers (`<command-message>`…), `tool_result` blocks,
  `<environment_context>`, image-reference lines, etc.
- **Window attribution** (`store_question`): a question binds to a terminal window's
  CGWindowID. **Session stickiness is primary** — if any prior question with the same
  `session_id` already has a window, the new one reuses it (so a session's questions stay
  with the terminal it actually runs in, even if focus moved by the time the ~800ms poll
  captures). Only a session's *first* question falls back to "whichever recognized
  terminal is frontmost right now" (`detect_front_terminal`); if none is frontmost it's
  stored with `window_id = NULL` (not shown). There is deliberately **no** "last terminal"
  fallback — that previously leaked questions from an unrecognized terminal onto the last
  recognized one. So `TERMINAL_APP_NAMES` must list every terminal the user uses, or its
  questions can't be attributed; keep it broad (Ghostty, iTerm, Terminal, Warp, Alacritty,
  WezTerm, Otty, kitty, Tabby, Hyper, Rio, WaveTerm, …).
- **Dedup**: `questions.dedup_key` is `UNIQUE`; inserts are `INSERT OR IGNORE`. Key is
  `claude:<uuid>` (Claude has per-message uuids) or `codex:h<hash>` (Codex has none, so
  hash of source+timestamp+text). On a real insert the thread emits `questions-updated`.

### Data model

SQLite file at `app_data_dir/askdock.sqlite3` (identifier `com.askdock.desktop`),
created in `init_database`. Two tables: `questions` and `settings` (key/value).

- `questions` columns: `id`, `window_id` (CGWindowID as text, may be NULL if no terminal
  was frontmost at capture), `source` (`claude`|`codex`), `session_id`, `cwd`, `text`,
  `asked_at` (transcript timestamp), `created_at` (capture time), `dedup_key` UNIQUE.
- `get_window_questions(window_id)` returns that window's rows, newest `asked_at` first.

> Caveat: CGWindowID distinguishes separate **windows**, not **tabs** within one Ghostty
> window (tabs share one CGWindowID). Questions asked in different tabs of the same window
> are grouped together.

### Terminal detection & attaching (macOS only)

`detect_front_terminal` uses Core Graphics `CGWindowListCopyWindowInfo`
(deps `core-graphics` / `core-foundation`, macOS-only in Cargo.toml) — **not** osascript.
It walks the front-to-back on-screen window list, takes the first `layer == 0` normal
window (skipping our own AskDock window by owner name — matched case-insensitively, since
the dev binary's owner is lowercase `askdock`), and if its owner app is in
`TERMINAL_APP_NAMES` (Terminal, iTerm, Ghostty, Warp, Alacritty, WezTerm) returns its
CGWindowID + bounds. This needs **no Accessibility or Screen Recording permission**
(window number + bounds are available without them; only window *titles* would need Screen
Recording, and we don't use titles).

- Returns `Ok(Some(win))` if the frontmost window is a terminal, `Ok(None)` if it's
  something else (browser/editor → dock keeps showing the last terminal's questions and
  does not move), `Err(msg)` on failure / non-macOS.
- `poll_front_window` repositions the dock beside the terminal (via the pure, unit-tested
  `calculate_dock_rect`: right of the terminal, else left, else overlay) **only when
  `reposition` is true** (auto-attach on). With auto-attach off, it still reports the
  window id so the question list keeps swapping, but does not move the window ("manual
  positioning"), toggled by the titlebar rotate button.

### Window & tray

Configured in `src-tauri/tauri.conf.json`: frameless (`decorations: false`),
`alwaysOnTop`, draggable via the `data-tauri-drag-region` titlebar. A tray icon
(set up in `setup_tray`) offers show/hide/quit. Window permissions are allowlisted in
`src-tauri/capabilities/default.json` — new `core:window:*` operations must be added there.

App icons live in `src-tauri/icons/` and are generated with `npx tauri icon app-icon.png`
(a source 1024×1024 PNG); `bundle.icon` in `tauri.conf.json` lists them. A corrupt/empty
`icon.png` makes Tauri panic at launch with `invalid icon … pixels supplied … (0)` — if
that happens, regenerate the icon set.
