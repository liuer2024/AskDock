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

The dock list is a timeline grouped by **day** (`dayLabel`). But when one window's
questions span **more than one working directory** (`cwd`) it can switch to grouping by
**project** instead (`projectLabel` = the cwd's last path segment, `multiProject` flag in
`Dock`). This is the mitigation for the tab caveat below: it can't follow tab switches
(macOS exposes no signal for the active tab), but it keeps each project's prompts visually
separated within a shared window list. Single-project windows keep the plain day timeline.

Whether grouping kicks in is gated by **two prefs**, chosen by the current window's terminal
type (`appName` vs `NATIVE_TAB_TERMINALS = Ghostty/Terminal`):
- `group_drawn_tabs` (default **on**) — app-drawn-tab terminals (iTerm2/Otty/Warp…), where
  every tab shares one CGWindowID, so one window legitimately mixes projects → grouping is
  useful.
- `group_native_windows` (default **off**) — native-tab terminals (Ghostty/Terminal), where
  each tab is its own window/CGWindowID, so a window's multiple cwds are just one tab `cd`-ing
  around → grouping is usually noise, off by default.
Both are toggles in Settings → 外观 → 按项目分组, persisted via `set_prefs`/`get_prefs`.

**Notch-style collapse** (titlebar `PanelRightClose` button): the dock can hide to a thin
handle pinned at the **screen right edge** (notch-like), and slide out **on hover**. Driven
by the `dock_collapse(state)` command — `"handle"` (shrink to a `NOTCH_HANDLE_W`×`H` bar at
the right edge, vertically centered; temporarily relaxes the window's min-size below the
config floor of 240×480), `"peek"` (grow to the full dock at the right edge — fired by the
handle's `onMouseEnter`), `"normal"` (restore min-size + `reposition_dock` — fired by the
exit button). Grow uses set-position-then-size, shrink uses size-then-position, to avoid
overflowing the right edge. Frontend state: `collapsed`/`peeking` in `Dock`; while `collapsed`, the poll loop passes
`reposition: false` so it doesn't fight the handle geometry. Collapse is **session-only**
(not persisted) — restarting AskDock comes back expanded.

The handle's **expand behavior** is a pref `notch_expand` (`hover`|`click`, default `hover`,
Settings → 外观 → 收起到边缘):
- `hover`: handle `onMouseEnter` → `peek` (temporary slide-out); dock `onMouseLeave` →
  `peekEnd` (~180ms debounce to avoid flicker at the resize boundary) → back to handle.
  `collapsed` stays true; the exit button restores the normal dock.
- `click`: handle `onClick` directly exits collapse (→ full normal dock); there's no `peek`
  state — it's a clean handle⇄dock toggle via the handle and the titlebar collapse button.

### Edge mascot (desktop pet)

The collapsed handle can show a **mascot** instead of the thin grip, reusing the **Codex
Pets** spritesheet format. Pref `notch_mascot` (`off` = plain grip, or a pet id; default
`sprout`), Settings → 外观 → 收起到边缘. Pets live in `$APPDATA/pets/<id>/` as `pet.json`
(`id`/`displayName`/`description`/`spritesheetPath`) + a spritesheet; the built-in
**小芽/sprout** is embedded (`include_bytes!`) and written there on first run by
`ensure_default_pet` (regenerate via `tools/gen_sprout.py`). `list_pets` scans that dir and
returns absolute spritesheet paths → frontend `convertFileSrc` (asset protocol, since
`$APPDATA/**` is in scope); `open_pets_dir` opens it in Finder so community pets can be
dropped in + 刷新. The sheet is a fixed **8 cols × 9 rows of 192×208 cells**; the 9 rows +
per-frame timings are the universal `PET_ROWS` table in `main.tsx`, animated by `PetSprite`
(a JS frame-ticker, not CSS `steps()`).

Display & states (frontend `Dock`):
- **Right-edge half-peek** (not dragged): `dock_collapse("handle")` sizes the window to
  `NOTCH_MASCOT_W`×`H` at the right edge with a **transparent** background
  (`set_background_color` + `data-petnotch` on `<html>` so html/body go transparent — else
  the theme bg covers it; `apply_window_surface` restores the opaque surface on
  `peek`/`normal`/`pet-expand`). CSS `translateX` pushes the mascot's right half off the
  screen edge (clipped), leaving the left half + its 3/4-left-facing face peeking in.
- **Desktop pet** (`petDragged` → `.free`): press-and-drag the mascot (pointer move > 4px →
  `startDragging`; window `onMoved` sets `petDragged`+`freeMode`) to anywhere on the desktop,
  shown full-body. Left-click → `pet-expand` (full dock in place); titlebar collapse →
  `pet-collapse` (shrink back to the mascot in place); the Zap button (turning `freeMode`
  off) leaves pet mode and re-attaches. `pet-expand`/`pet-collapse`/`pet-menu`/`pet-menu-end`
  transform at the **current** window position (clamped on-screen), unlike `handle`/`peek`
  which pin to the right edge. While in pet mode the poll loop passes `reposition: false`.
- **Right-click menu**: `onContextMenu` → `setPetMenu` + `dock_collapse("pet-menu")` grows
  the window down by `PET_MENU_H` so a self-drawn HTML menu (`.petMenu`) sits **below** the
  mascot (去屏幕右缘 / 展开列表 / 贴靠终端 / 设置…); closing → `pet-menu-end` shrinks back.
  (A native `popup_menu` was tried first but its click events didn't route back reliably.)
- **Idle micro-actions**: while the mascot shows, every ~7–16s it randomly plays
  jumping/waving/waiting then returns to idle (`petAction`); the right-edge half-peek uses
  only `waving` (a front-facing pose would show half a face). A new captured question
  (`questions-updated` while collapsed) plays `waving` (`greet`) for ~2.6s, taking priority.

### AI-question capture (the heart of the app)

A background thread (`run_capture_loop`, spawned in `setup`) polls every ~800ms over the
AI tools' session transcripts and harvests new user prompts:

- **Sources** (`capture_sources`): Claude Code `~/.claude/projects/**/*.jsonl` and Codex
  `~/.codex/sessions/**/*.jsonl`. Both are JSONL; add more sources here.
- **Codex session meta** (`read_codex_meta` → `parse_codex_meta`, returns `CodexMeta {
  is_terminal, cwd }`): a Codex session's first line is a `session_meta` whose
  `payload.source` is `cli` (codex-tui), `exec` (codex exec), or `vscode` (**Codex Desktop /
  IDE extension**), and whose `payload.cwd` is the session's working directory.
  - *Terminal-only filter*: only terminal sources (`cli`/`exec`) are captured; `vscode`
    sessions are skipped — their prompts aren't typed in a terminal, so the "frontmost
    terminal window at capture time" attribution would wrongly pin them to whatever terminal
    the user happened to switch back to (this caused Desktop questions to bleed into
    Ghostty). Missing/old `source` field → treated as terminal (back-compat).
  - *cwd backfill*: `parse_codex_line` itself yields `cwd: None` (the per-message rows have
    no cwd), so the capture loop fills each Codex prompt's `cwd` from the session's
    `session_meta.cwd`. Claude prompts already carry their own per-line `cwd`.
  - Meta is read only for files that produced a new prompt, cached per-file in
    `run_capture_loop` (`codex_meta`).
- **Only going forward**: at startup, every existing file's read offset is set to its
  current EOF, so historical conversations are *not* imported. Each loop reads only newly
  appended **complete** lines (tracked per-file byte offset; a trailing partial line waits
  for next pass).
- **Parsing/filtering** (`parse_claude_line`, `parse_codex_line`, `is_real_prompt`):
  extracts the user's text (string content, or joined `text`/`input_text` blocks) and
  drops noise — slash-command wrappers (`<command-message>`…), `tool_result` blocks,
  `<environment_context>`, image-reference lines, etc.
- **Window attribution** (`store_question`): a question binds to **whichever recognized
  terminal window is frontmost at capture time** (`detect_front_terminal`'s CGWindowID);
  if none is frontmost it's stored with `window_id = NULL` (not shown). This is correct
  *because we only capture real user-typed prompts* — synthetic user-role messages (tool
  results, `<image>` blocks, Codex "agent history" deltas) are filtered out in parsing, and
  a genuinely-typed prompt is always written while its own terminal window is focused. So
  there is deliberately **no** session-stickiness and **no** "last terminal" fallback: both
  were tried and both caused bugs — stickiness permanently pinned a session to its
  first-seen window (so two AI tools in two windows of the same app couldn't separate), and
  the last-terminal fallback leaked an unrecognized terminal's questions onto the last
  recognized one. Consequence: `TERMINAL_APP_NAMES` must list every terminal the user uses
  or its questions can't be attributed — keep it broad (Ghostty, iTerm, Terminal, Warp,
  Alacritty, WezTerm, Otty, kitty, Tabby, Hyper, Rio, WaveTerm, …). CGWindowID reliably
  distinguishes separate windows of the same app (verified for multi-window Otty).
- **Dedup**: `questions.dedup_key` is `UNIQUE`; inserts are `INSERT OR IGNORE`. Key is
  `claude:<uuid>` (Claude has per-message uuids) or `codex:h<hash>` (Codex has none, so
  hash of source+timestamp+text). On a real insert the thread emits `questions-updated`.

### Data model

SQLite file at `app_data_dir/askdock.sqlite3` (identifier `com.askdock.desktop`),
created in `init_database`. Two tables: `questions` and `settings` (key/value).

- `questions` columns: `id`, `window_id` (CGWindowID as text, may be NULL if no terminal
  was frontmost at capture), `source` (`claude`|`codex`), `session_id`, `cwd`, `text`,
  `asked_at` (transcript timestamp), `created_at` (capture time), `dedup_key` UNIQUE,
  `image_path`. **`image_path` is newline-joined** — one message can carry several images
  (`parse_*` collect all blocks into `ParsedQuestion.images`; `store_question` persists each
  and joins the paths with `\n`). The frontend splits on `\n` and renders one `<img>` each;
  `purge_cache`'s orphan sweep splits too. A single-path value is just the 1-element case.
- `get_window_questions(window_id)` returns that window's rows, newest `asked_at` first.

> Caveat — tabs depend on the terminal's tab implementation, not on AskDock:
> - **Native macOS tabs** (NSWindow tabbing, e.g. **Ghostty**): each tab is its own
>   `NSWindow` with its own CGWindowID (verified: 3 Ghostty tabs = winNums 1266/208/406, all
>   with identical overlapping bounds; the non-active tabs are off-screen windows, only
>   visible with `OnScreenOnly` dropped). Switching tabs changes the frontmost CGWindowID,
>   so AskDock separates them automatically — same as separate windows.
> - **App-drawn tabs** (WebKit/Electron-style wrappers, e.g. **Otty**, bundle
>   `io.appmakes.otty`): the whole window is one `NSWindow` / one CGWindowID; the tabs are
>   painted inside the web view, so macOS exposes no signal for "which tab is active"
>   (verified: an Otty window with 4 tabs is a single winNum 223). AskDock cannot tell these
>   tabs apart without reading terminal contents / Accessibility, which it deliberately
>   avoids. Questions from all tabs of such a window are grouped together; the workaround is
>   to use separate windows (Otty multi-window *is* distinguished: winNums 223 vs 1247).

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
