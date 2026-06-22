use chrono::Utc;
use rusqlite::{params, Connection, Row};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State};
use uuid::Uuid;

struct Db(Mutex<Connection>);

/// 一条自动抓取到的 AI 提问。
#[derive(Debug, Serialize)]
struct CapturedQuestion {
    id: String,
    window_id: Option<String>,
    source: String,
    session_id: Option<String>,
    cwd: Option<String>,
    text: String,
    asked_at: String,
}

/// 前台窗口探测结果。window_id 为 None 表示前台不是终端。
#[derive(Debug, Serialize)]
struct FrontStatus {
    window_id: Option<String>,
    app_name: Option<String>,
    message: String,
}

/// 用户偏好（外观 + 贴靠），存在 settings 表，两个窗口共享。
#[derive(Debug, Serialize)]
struct Prefs {
    theme: String,
    font_face: String,
    font_size: String,
    glass: bool,
    dock_mode: String,
    attach_side: String,
    dock_width: i32,
    dock_height: i32,
    follow: bool,
}

#[derive(Debug, Clone)]
struct TerminalWindow {
    app_name: String,
    window_id: i64,
    rect: Rect,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// 从会话文件里解析出的一条原始提问（未归类）。
struct ParsedQuestion {
    dedup_key: String,
    text: String,
    asked_at: String,
    cwd: Option<String>,
    session_id: Option<String>,
}

const TERMINAL_APP_NAMES: &[&str] = &[
    "Terminal",
    "iTerm",
    "iTerm2",
    "Ghostty",
    "Warp",
    "Alacritty",
    "WezTerm",
    "Otty",
    "kitty",
    "Tabby",
    "Hyper",
    "Rio",
    "WaveTerm",
];

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let db = init_database(&app.handle())?;
            app.manage(Db(Mutex::new(db)));
            setup_tray(app.handle())?;
            if let Some(window) = app.get_webview_window("main") {
                window.set_position(PhysicalPosition::new(80, 80))?;
                window.set_size(PhysicalSize::new(360, 620))?;
                let glass_on = {
                    let db = app.state::<Db>();
                    let conn = db.0.lock().unwrap();
                    get_i32_setting(&conn, "glass", 0) != 0
                };
                apply_glass(&window, glass_on);
                window.show()?;
                window.set_focus()?;
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || run_capture_loop(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            poll_front_window,
            get_window_questions,
            delete_question,
            clear_window,
            get_prefs,
            set_prefs,
            open_settings,
            show_main_window,
            hide_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running AskDock");
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示浮窗", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏浮窗", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    TrayIconBuilder::new()
        .tooltip("AskDock")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                let _ = show_window(app);
            }
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { .. }) {
                let app = tray.app_handle();
                let _ = show_window(app);
            }
        })
        .build(app)?;
    Ok(())
}

fn show_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

fn init_database(app: &AppHandle) -> tauri::Result<Connection> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| tauri::Error::Anyhow(error.into()))?;
    std::fs::create_dir_all(&app_dir).map_err(|error| tauri::Error::Anyhow(error.into()))?;
    let db_path = app_dir.join("askdock.sqlite3");
    let conn = Connection::open(db_path).map_err(|error| tauri::Error::Anyhow(error.into()))?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS questions (
          id TEXT PRIMARY KEY,
          window_id TEXT,
          source TEXT NOT NULL,
          session_id TEXT,
          cwd TEXT,
          text TEXT NOT NULL,
          asked_at TEXT NOT NULL,
          created_at TEXT NOT NULL,
          dedup_key TEXT NOT NULL UNIQUE
        );

        CREATE INDEX IF NOT EXISTS idx_questions_window ON questions(window_id);

        CREATE TABLE IF NOT EXISTS settings (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL
        );

        INSERT OR IGNORE INTO settings (key, value) VALUES
          ('theme', 'linen'),
          ('font_face', 'system'),
          ('font_size', 'medium'),
          ('glass', '0'),
          ('dock_mode', 'terminal'),
          ('attach_side', 'right'),
          ('dock_width', '360'),
          ('dock_height', '620'),
          ('dock_gap', '8'),
          ('follow_terminal', '1');
        "#,
    )
    .map_err(|error| tauri::Error::Anyhow(error.into()))?;

    Ok(conn)
}

fn map_question(row: &Row) -> rusqlite::Result<CapturedQuestion> {
    Ok(CapturedQuestion {
        id: row.get(0)?,
        window_id: row.get(1)?,
        source: row.get(2)?,
        session_id: row.get(3)?,
        cwd: row.get(4)?,
        text: row.get(5)?,
        asked_at: row.get(6)?,
    })
}

const QUESTION_COLUMNS: &str = "id, window_id, source, session_id, cwd, text, asked_at";

/// 前端轮询：探测当前前台终端窗口，按需贴靠，并记下它供抓取时归类用。
#[tauri::command]
fn poll_front_window(app: AppHandle, db: State<'_, Db>, reposition: bool) -> FrontStatus {
    let detected = detect_front_terminal();
    if reposition {
        // 终端模式：传入终端矩形贴过去；固定屏幕边模式：reposition_dock 内部忽略它。
        let terminal = detected
            .as_ref()
            .ok()
            .and_then(|opt| opt.as_ref())
            .map(|win| win.rect);
        let _ = reposition_dock(&app, &db, terminal);
    }
    match detected {
        Ok(Some(win)) => FrontStatus {
            window_id: Some(win.window_id.to_string()),
            app_name: Some(win.app_name.clone()),
            message: format!("已贴靠 {}", win.app_name),
        },
        Ok(None) => FrontStatus {
            window_id: None,
            app_name: None,
            message: "前台不是终端，保留上一个窗口".to_string(),
        },
        Err(error) => FrontStatus {
            window_id: None,
            app_name: None,
            message: error,
        },
    }
}

#[tauri::command]
fn get_window_questions(
    db: State<'_, Db>,
    window_id: String,
) -> Result<Vec<CapturedQuestion>, String> {
    let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {QUESTION_COLUMNS} FROM questions
             WHERE window_id = ?1
             ORDER BY datetime(asked_at) DESC, created_at DESC
             LIMIT 200"
        ))
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![window_id], map_question)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
fn delete_question(db: State<'_, Db>, id: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
    conn.execute("DELETE FROM questions WHERE id = ?1", params![id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn clear_window(db: State<'_, Db>, window_id: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
    conn.execute("DELETE FROM questions WHERE window_id = ?1", params![window_id])
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_prefs(db: State<'_, Db>) -> Result<Prefs, String> {
    let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
    Ok(Prefs {
        theme: get_str_setting(&conn, "theme", "linen"),
        font_face: get_str_setting(&conn, "font_face", "system"),
        font_size: get_str_setting(&conn, "font_size", "medium"),
        glass: get_i32_setting(&conn, "glass", 0) != 0,
        dock_mode: get_str_setting(&conn, "dock_mode", "terminal"),
        attach_side: get_str_setting(&conn, "attach_side", "right"),
        dock_width: get_i32_setting(&conn, "dock_width", 360),
        dock_height: get_i32_setting(&conn, "dock_height", 620),
        follow: get_i32_setting(&conn, "follow_terminal", 1) != 0,
    })
}

/// 写入所有偏好，立即重新贴靠，并广播 prefs-changed 让各窗口同步。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn set_prefs(
    app: AppHandle,
    db: State<'_, Db>,
    theme: String,
    font_face: String,
    font_size: String,
    glass: bool,
    mode: String,
    side: String,
    width: i32,
    height: i32,
    follow: bool,
) -> Result<(), String> {
    let mode = if mode == "screen" { "screen" } else { "terminal" }.to_string();
    let side = match side.as_str() {
        "left" | "right" | "top" | "bottom" => side,
        _ => "right".to_string(),
    };
    let width = width.clamp(180, 900);
    let height = height.clamp(120, 1600);
    {
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        set_setting(&conn, "theme", &theme)?;
        set_setting(&conn, "font_face", &font_face)?;
        set_setting(&conn, "font_size", &font_size)?;
        set_setting(&conn, "glass", if glass { "1" } else { "0" })?;
        set_setting(&conn, "dock_mode", &mode)?;
        set_setting(&conn, "attach_side", &side)?;
        set_setting(&conn, "dock_width", &width.to_string())?;
        set_setting(&conn, "dock_height", &height.to_string())?;
        set_setting(&conn, "follow_terminal", if follow { "1" } else { "0" })?;
    }
    if let Some(window) = app.get_webview_window("main") {
        apply_glass(&window, glass);
    }
    let terminal = detect_front_terminal().ok().flatten().map(|win| win.rect);
    let _ = reposition_dock(&app, &db, terminal);
    let _ = app.emit("prefs-changed", ());
    Ok(())
}

/// 给窗口加/去 macOS 毛玻璃(NSVisualEffectView)。需要窗口透明 + CSS 半透明背景才看得到。
#[cfg(target_os = "macos")]
fn apply_glass(window: &tauri::WebviewWindow, on: bool) {
    use window_vibrancy::{apply_vibrancy, clear_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
    if on {
        let _ = apply_vibrancy(
            window,
            NSVisualEffectMaterial::Sidebar,
            Some(NSVisualEffectState::Active),
            None,
        );
    } else {
        let _ = clear_vibrancy(window);
    }
}

#[cfg(not(target_os = "macos"))]
fn apply_glass(_window: &tauri::WebviewWindow, _on: bool) {}

/// 打开（或聚焦）独立的设置窗口。它加载同一前端，按窗口 label 渲染设置页。
#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    let settings = tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("AskDock 设置")
    .inner_size(720.0, 520.0)
    .min_inner_size(620.0, 440.0)
    .resizable(true)
    .visible(false)
    .build()
    .map_err(|error| error.to_string())?;

    // 居中到「浮窗所在的那块屏幕」，避免跑到主屏/别的屏上。
    if let Some(main) = app.get_webview_window("main") {
        if let Ok(Some(monitor)) = main.current_monitor() {
            let pos = monitor.position();
            let size = monitor.size();
            let scale = monitor.scale_factor();
            let win_w = (720.0 * scale) as i32;
            let win_h = (520.0 * scale) as i32;
            let x = pos.x + (size.width as i32 - win_w).max(0) / 2;
            let y = pos.y + (size.height as i32 - win_h).max(0) / 2;
            let _ = settings.set_position(PhysicalPosition::new(x, y));
        }
    }
    settings.show().map_err(|error| error.to_string())?;
    settings.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn show_main_window(app: AppHandle) -> Result<(), String> {
    show_window(&app).map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn reposition_dock(app: &AppHandle, db: &State<'_, Db>, terminal: Option<Rect>) -> Result<(), String> {
    let (mode, side, dock_width, dock_height, dock_gap, follow) = {
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        (
            get_str_setting(&conn, "dock_mode", "terminal"),
            get_str_setting(&conn, "attach_side", "right"),
            get_i32_setting(&conn, "dock_width", 360),
            get_i32_setting(&conn, "dock_height", 620),
            get_i32_setting(&conn, "dock_gap", 8),
            get_i32_setting(&conn, "follow_terminal", 1) != 0,
        )
    };

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(|| "无法读取显示器信息".to_string())?;
    let pos = monitor.position();
    let size = monitor.size();
    let screen = Rect {
        x: pos.x,
        y: pos.y,
        width: size.width as i32,
        height: size.height as i32,
    };

    let dock = if mode == "screen" {
        // 像 Mac Dock 一样钉在浮窗所在屏幕的某条边，不跟终端动。
        screen_edge_rect(screen, &side, dock_width, dock_height, dock_gap)
    } else {
        match terminal {
            Some(rect) => {
                calculate_dock_rect(rect, screen, &side, dock_width, dock_height, dock_gap, follow)
            }
            None => return Ok(()), // 终端模式但前台不是终端：保持原位
        }
    };

    window
        .set_position(PhysicalPosition::new(dock.x, dock.y))
        .map_err(|error| error.to_string())?;
    window
        .set_size(PhysicalSize::new(dock.width as u32, dock.height as u32))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// 固定屏幕边模式：把浮窗钉在屏幕指定边、沿该边居中。
fn screen_edge_rect(screen: Rect, side: &str, width: i32, height: i32, gap: i32) -> Rect {
    let w = width.max(180);
    let h = height.max(120);
    let (x, y) = match side {
        "left" => (screen.x + gap, screen.y + (screen.height - h) / 2),
        "top" => (screen.x + (screen.width - w) / 2, screen.y + gap),
        "bottom" => (
            screen.x + (screen.width - w) / 2,
            screen.y + screen.height - h - gap,
        ),
        _ => (
            screen.x + screen.width - w - gap,
            screen.y + (screen.height - h) / 2,
        ), // right
    };
    Rect { x, y, width: w, height: h }
}

fn get_i32_setting(conn: &Connection, key: &str, fallback: i32) -> i32 {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value| value.parse::<i32>().ok())
    .unwrap_or(fallback)
}

fn get_str_setting(conn: &Connection, key: &str, fallback: &str) -> String {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|_| fallback.to_string())
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// 按贴靠方向和尺寸算出浮窗位置。沿贴靠边的尺寸在 follow 为真时跟随终端。
fn calculate_dock_rect(
    terminal: Rect,
    screen: Rect,
    side: &str,
    width: i32,
    height: i32,
    gap: i32,
    follow: bool,
) -> Rect {
    let vertical = side == "left" || side == "right";
    let (w, h) = if vertical {
        (width, if follow { terminal.height } else { height })
    } else {
        (if follow { terminal.width } else { width }, height)
    };
    let w = w.max(180);
    let h = h.max(120);

    let (mut x, mut y) = match side {
        "left" => (terminal.x - gap - w, terminal.y),
        "top" => (terminal.x, terminal.y - gap - h),
        "bottom" => (terminal.x, terminal.y + terminal.height + gap),
        _ => (terminal.x + terminal.width + gap, terminal.y), // right
    };

    // 保持浮窗在屏幕可见区域内
    x = x.clamp(screen.x, (screen.x + screen.width - w).max(screen.x));
    y = y.clamp(screen.y, (screen.y + screen.height - h).max(screen.y));

    Rect { x, y, width: w, height: h }
}

// ===== AI 提问抓取 =====

/// 后台线程：轮询 Claude / Codex 的会话 JSONL，把新出现的用户提问
/// 按「当下前台终端窗口」归类入库。只抓 AskDock 启动后新增的内容。
fn run_capture_loop(app: AppHandle) {
    let sources = capture_sources();
    // 启动时把已有文件的偏移设到末尾，避免把历史会话整段导入。
    let mut offsets: HashMap<PathBuf, u64> = HashMap::new();
    for (root, _) in &sources {
        let mut files = Vec::new();
        collect_jsonl(root, &mut files, 0);
        for file in files {
            if let Ok(meta) = fs::metadata(&file) {
                offsets.insert(file, meta.len());
            }
        }
    }

    loop {
        let mut captured_any = false;
        for (root, source) in &sources {
            let mut files = Vec::new();
            collect_jsonl(root, &mut files, 0);
            for file in files {
                for parsed in read_new_questions(&file, &mut offsets, source) {
                    if store_question(&app, source, parsed) {
                        captured_any = true;
                    }
                }
            }
        }
        if captured_any {
            let _ = app.emit("questions-updated", ());
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

/// 各 AI 工具的会话目录及来源标记。
fn capture_sources() -> Vec<(PathBuf, String)> {
    let home = match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home),
        Err(_) => return Vec::new(),
    };
    vec![
        (home.join(".claude/projects"), "claude".to_string()),
        (home.join(".codex/sessions"), "codex".to_string()),
    ]
    .into_iter()
    .filter(|(path, _)| path.exists())
    .collect()
}

/// 递归收集 .jsonl 文件（限制深度，避免无谓深挖）。
fn collect_jsonl(root: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl(&path, out, depth + 1);
        } else if path.extension().map_or(false, |ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// 从上次偏移读到末尾，按完整行解析出新提问，并推进偏移。
fn read_new_questions(
    path: &Path,
    offsets: &mut HashMap<PathBuf, u64>,
    source: &str,
) -> Vec<ParsedQuestion> {
    let mut out = Vec::new();
    let Ok(mut file) = fs::File::open(path) else {
        return out;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut start = offsets.get(path).copied().unwrap_or(0);
    if len < start {
        start = 0; // 文件被截断/轮换
    }
    if len <= start {
        offsets.insert(path.to_path_buf(), len);
        return out;
    }
    if file.seek(SeekFrom::Start(start)).is_err() {
        return out;
    }
    let mut buf = Vec::new();
    if file.take(len - start).read_to_end(&mut buf).is_err() {
        return out;
    }
    // 只处理到最后一个换行符为止，剩余不完整的留到下次。
    let last_nl = buf.iter().rposition(|b| *b == b'\n');
    let consumed = match last_nl {
        Some(index) => index + 1,
        None => {
            // 没有完整行，等下次
            return out;
        }
    };
    let session_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_string());

    let text = String::from_utf8_lossy(&buf[..consumed]);
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed = match source {
            "claude" => parse_claude_line(line),
            "codex" => parse_codex_line(line, session_id.as_deref()),
            _ => None,
        };
        if let Some(parsed) = parsed {
            out.push(parsed);
        }
    }

    offsets.insert(path.to_path_buf(), start + consumed as u64);
    out
}

/// 判断一段文本是不是用户真正的提问（排除系统包装/图片引用等噪音）。
fn is_real_prompt(text: &str) -> bool {
    let t = text.trim_start();
    if t.is_empty() {
        return false;
    }
    let noise_prefixes = [
        "<command-message>",
        "<command-name>",
        "<local-command",
        "<bash-",
        "<environment_context>",
        "<user_instructions>",
        "<system-reminder>",
        "[Image: source:",
        "[Request interrupted",
        "Caveat:",
        // Codex 注入的“代理历史/审查”合成消息（里面含工具与助手内容，不是用户提问）
        "The following is the Codex agent history",
    ];
    !noise_prefixes.iter().any(|prefix| t.starts_with(prefix))
}

/// 清掉提问里的图片噪音：去掉 `<image …></image>` 标签与 `[Image #N]` 占位。
fn clean_prompt(text: &str) -> String {
    // 1) 去掉 <image …></image>（或自闭合 <image …>）
    let mut stage1 = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<image") {
        stage1.push_str(&rest[..start]);
        let after = &rest[start..];
        rest = if let Some(end) = after.find("</image>") {
            &after[end + "</image>".len()..]
        } else if let Some(end) = after.find('>') {
            &after[end + 1..]
        } else {
            ""
        };
    }
    stage1.push_str(rest);

    // 2) 去掉 [Image #N] 占位标记
    let mut out = String::with_capacity(stage1.len());
    let mut rest = stage1.as_str();
    while let Some(start) = rest.find("[Image #") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        rest = match after.find(']') {
            Some(end) => &after[end + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    // 去掉清理后残留在开头的分隔标点/空白（如 "[Image #1]," → ","）
    out.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | '，' | '、' | '.' | '。' | ':' | '：' | ';' | '；' | '-')
    })
    .to_string()
}

/// 从数组型 content 里拼接所有 text 块，跳过 tool_result 等。
fn join_text_blocks(content: &serde_json::Value, text_key: &str, kind_key: &str) -> String {
    let mut parts = Vec::new();
    if let Some(array) = content.as_array() {
        for block in array {
            if block.get(kind_key).and_then(|v| v.as_str()) == Some("text")
                || block.get(kind_key).and_then(|v| v.as_str()) == Some("input_text")
            {
                if let Some(text) = block.get(text_key).and_then(|v| v.as_str()) {
                    parts.push(text);
                }
            }
        }
    }
    parts.join("\n")
}

fn parse_claude_line(line: &str) -> Option<ParsedQuestion> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("user") {
        return None;
    }
    let content = v.get("message").and_then(|m| m.get("content"))?;
    let raw = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(_) => join_text_blocks(content, "text", "type"),
        _ => return None,
    };
    let text = clean_prompt(&raw).trim().to_string();
    if !is_real_prompt(&text) {
        return None;
    }
    let uuid = v.get("uuid").and_then(|u| u.as_str());
    let asked_at = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .map(|t| t.to_string())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    Some(ParsedQuestion {
        dedup_key: dedup_key("claude", uuid, &asked_at, &text),
        text,
        asked_at,
        cwd: v.get("cwd").and_then(|c| c.as_str()).map(|c| c.to_string()),
        session_id: v
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_codex_line(line: &str, session_id: Option<&str>) -> Option<ParsedQuestion> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
        return None;
    }
    let payload = v.get("payload")?;
    if payload.get("type").and_then(|t| t.as_str()) != Some("message")
        || payload.get("role").and_then(|r| r.as_str()) != Some("user")
    {
        return None;
    }
    let content = payload.get("content")?;
    let raw = join_text_blocks(content, "text", "type");
    let text = clean_prompt(&raw).trim().to_string();
    if !is_real_prompt(&text) {
        return None;
    }
    let asked_at = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .map(|t| t.to_string())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    Some(ParsedQuestion {
        dedup_key: dedup_key("codex", None, &asked_at, &text),
        text,
        asked_at,
        cwd: None,
        session_id: session_id.map(|s| s.to_string()),
    })
}

fn dedup_key(source: &str, uuid: Option<&str>, asked_at: &str, text: &str) -> String {
    match uuid {
        Some(uuid) => format!("{source}:{uuid}"),
        None => {
            let mut hasher = DefaultHasher::new();
            (source, asked_at, text).hash(&mut hasher);
            format!("{source}:h{:x}", hasher.finish())
        }
    }
}

/// 归类并入库；返回是否真的新插入了一条（去重命中则返回 false）。
///
/// 归类规则：归到**写入那一刻的前台终端窗口**。因为我们只抓用户真正手敲
/// 的提问（合成消息、工具结果、agent-history 都已过滤），它必然发生在当前
/// 聚焦的窗口里，所以当下前台即正确归属。**不再做会话黏定** —— 黏定会把一
/// 个会话永久锁死在它第一次出现的窗口上，导致同一应用的多窗口分不开。
fn store_question(app: &AppHandle, source: &str, parsed: ParsedQuestion) -> bool {
    let window_id = detect_front_terminal()
        .ok()
        .flatten()
        .map(|win| win.window_id.to_string());

    let Some(db) = app.try_state::<Db>() else {
        return false;
    };
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let Ok(conn) = db.0.lock() else {
        return false;
    };
    conn.execute(
        "INSERT OR IGNORE INTO questions
           (id, window_id, source, session_id, cwd, text, asked_at, created_at, dedup_key)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            window_id,
            source,
            parsed.session_id,
            parsed.cwd,
            parsed.text,
            parsed.asked_at,
            now,
            parsed.dedup_key
        ],
    )
    .map(|changed| changed > 0)
    .unwrap_or(false)
}

/// 用 Core Graphics 的窗口列表找出当前最前的普通窗口。
/// 返回 Ok(Some) 表示前台是终端窗口；Ok(None) 表示前台不是终端。
#[cfg(target_os = "macos")]
fn detect_front_terminal() -> Result<Option<TerminalWindow>, String> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowNumber,
        kCGWindowOwnerName,
    };

    unsafe {
        let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
        let windows = copy_window_info(options, kCGNullWindowID)
            .ok_or_else(|| "无法读取窗口列表".to_string())?;

        let get = |dict: &CFDictionary<CFString, CFType>, key: CFStringRef| -> Option<CFType> {
            let k = CFString::wrap_under_get_rule(key);
            dict.find(&k).map(|item| (*item).clone())
        };

        for item in windows.iter() {
            let dict =
                CFDictionary::<CFString, CFType>::wrap_under_get_rule(*item as CFDictionaryRef);

            let layer = get(&dict, kCGWindowLayer)
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_i32())
                .unwrap_or(-1);
            if layer != 0 {
                continue;
            }

            let owner = get(&dict, kCGWindowOwnerName)
                .and_then(|v| v.downcast::<CFString>())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if owner.eq_ignore_ascii_case("questiondock") || owner.eq_ignore_ascii_case("askdock") {
                continue;
            }

            let is_terminal = TERMINAL_APP_NAMES
                .iter()
                .any(|name| owner.eq_ignore_ascii_case(name));
            if !is_terminal {
                return Ok(None);
            }

            let window_id = get(&dict, kCGWindowNumber)
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_i64())
                .ok_or_else(|| "窗口缺少编号".to_string())?;

            let bounds = get(&dict, kCGWindowBounds).ok_or_else(|| "窗口缺少位置".to_string())?;
            let bounds = CFDictionary::<CFString, CFType>::wrap_under_get_rule(
                bounds.as_CFTypeRef() as CFDictionaryRef,
            );
            let read = |key: &str| -> i32 {
                let k = CFString::new(key);
                bounds
                    .find(&k)
                    .and_then(|v| (*v).clone().downcast::<CFNumber>())
                    .and_then(|n| n.to_i64())
                    .unwrap_or(0) as i32
            };

            return Ok(Some(TerminalWindow {
                app_name: owner,
                window_id,
                rect: Rect {
                    x: read("X"),
                    y: read("Y"),
                    width: read("Width"),
                    height: read("Height"),
                },
            }));
        }

        Ok(None)
    }
}

#[cfg(not(target_os = "macos"))]
fn detect_front_terminal() -> Result<Option<TerminalWindow>, String> {
    Err("当前平台暂未实现窗口识别，可手动定位浮窗".to_string())
}

#[cfg(test)]
mod tests {
    use super::{calculate_dock_rect, is_real_prompt, Rect};

    #[test]
    fn attaches_to_right_following_terminal_height() {
        let dock = calculate_dock_rect(
            Rect { x: 100, y: 50, width: 500, height: 700 },
            Rect { x: 0, y: 0, width: 2000, height: 1200 },
            "right",
            280,
            400,
            8,
            true,
        );
        assert_eq!(dock.x, 608); // 100 + 500 + 8
        assert_eq!(dock.y, 50);
        assert_eq!(dock.width, 280);
        assert_eq!(dock.height, 700); // follows terminal
    }

    #[test]
    fn attaches_to_left_with_custom_height() {
        let dock = calculate_dock_rect(
            Rect { x: 650, y: 50, width: 500, height: 700 },
            Rect { x: 0, y: 0, width: 2000, height: 1200 },
            "left",
            280,
            400,
            8,
            false,
        );
        assert_eq!(dock.x, 362); // 650 - 8 - 280
        assert_eq!(dock.height, 400); // custom, not following
    }

    #[test]
    fn attaches_to_top_following_terminal_width() {
        let dock = calculate_dock_rect(
            Rect { x: 200, y: 500, width: 600, height: 400 },
            Rect { x: 0, y: 0, width: 2000, height: 1200 },
            "top",
            280,
            300,
            8,
            true,
        );
        assert_eq!(dock.x, 200);
        assert_eq!(dock.y, 192); // 500 - 8 - 300
        assert_eq!(dock.width, 600); // follows terminal width
        assert_eq!(dock.height, 300);
    }

    #[test]
    fn screen_edge_pins_right_and_centers_vertically() {
        let dock = super::screen_edge_rect(
            Rect { x: 0, y: 0, width: 1920, height: 1080 },
            "right",
            300,
            600,
            8,
        );
        assert_eq!(dock.x, 1920 - 300 - 8); // 钉右边
        assert_eq!(dock.y, (1080 - 600) / 2); // 垂直居中
        assert_eq!(dock.width, 300);
        assert_eq!(dock.height, 600);
    }

    #[test]
    fn filters_command_and_noise_prompts() {
        assert!(is_real_prompt("帮我修一下登录页"));
        assert!(!is_real_prompt("<command-message>init</command-message>"));
        assert!(!is_real_prompt("[Image: source: /tmp/x.png]"));
        assert!(!is_real_prompt("<environment_context>"));
        assert!(!is_real_prompt("   "));
        assert!(!is_real_prompt(
            "The following is the Codex agent history whose request action you are assessing."
        ));
    }

    #[test]
    fn cleans_image_noise_from_prompt() {
        let raw = "<image name=[Image #1] path=\"/tmp/codex-clipboard-x.png\"></image>[Image #1] 这页面太粗糙了，设计一下。";
        assert_eq!(super::clean_prompt(raw).trim(), "这页面太粗糙了，设计一下。");
        // 纯图片消息清理后为空 → 不算有效提问
        assert!(super::clean_prompt("<image path=\"/tmp/a.png\"></image>").trim().is_empty());
    }
}
