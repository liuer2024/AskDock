use base64::Engine;
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
    image_path: Option<String>,
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
struct PurgeStats {
    rows: usize,
    images: usize,
}

#[derive(Debug, Serialize)]
struct Prefs {
    theme: String,
    font_face: String,
    font_size: String,
    glass: String,
    corner_radius: String,
    filter_rules: String,
    retention_days: i32,
    dock_mode: String,
    attach_side: String,
    dock_width: i32,
    dock_height: i32,
    follow: bool,
    group_drawn_tabs: bool,
    group_native_windows: bool,
    notch_expand: String,
    notch_mascot: String,
}

/// 一只边缘小人（Codex Pets 格式）：pet.json + 精灵图。
#[derive(Debug, Serialize)]
struct PetInfo {
    id: String,
    display_name: String,
    description: String,
    /// 精灵图绝对路径，前端用 convertFileSrc 走 asset 协议加载（$APPDATA 已在白名单内）。
    spritesheet: String,
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

/// 提问里附带的图片来源：Claude 是内嵌 base64，Codex 是临时文件路径。
enum CapturedImage {
    Base64 { ext: String, data: String },
    Path(String),
}

/// 从会话文件里解析出的一条原始提问（未归类）。
struct ParsedQuestion {
    dedup_key: String,
    text: String,
    asked_at: String,
    cwd: Option<String>,
    session_id: Option<String>,
    images: Vec<CapturedImage>,
}

/// 纯图片消息的占位文本（清理后无文字时用它，避免丢掉这条）。
const IMAGE_MARKER: &str = "🖼 图片";

// 刘海式收起：把手的尺寸（贴屏幕右缘、纵向居中的小竖条）。
const NOTCH_HANDLE_W: i32 = 14;
const NOTCH_HANDLE_H: i32 = 120;

// 收起时若选了边缘小人（mascot），把手放大到能放下一只精灵的小窗口。
// 前端 PetSprite 的显示尺寸需与之大致吻合（见 main.tsx 的 PET_DISPLAY_H）。
const NOTCH_MASCOT_W: i32 = 120;
const NOTCH_MASCOT_H: i32 = 172;
// 右键小人时窗口临时增高，把菜单显示在小人下方（不盖住小人）。
const PET_MENU_H: i32 = 176;

// 内置默认小人“小芽”：精灵图随二进制内嵌，首启写入 $APPDATA/pets/sprout/。
const SPROUT_SPRITESHEET: &[u8] = include_bytes!("../assets/pets/sprout.png");
const SPROUT_PET_JSON: &str = r#"{
  "id": "sprout",
  "displayName": "小芽",
  "description": "AskDock 内置的小芽，收起时扒在屏幕边看着你。",
  "spritesheetPath": "spritesheet.png"
}
"#;

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
            ensure_default_pet(&app.handle());
            setup_tray(app.handle())?;
            if let Some(window) = app.get_webview_window("main") {
                window.set_position(PhysicalPosition::new(80, 80))?;
                window.set_size(PhysicalSize::new(360, 620))?;
                let (theme, glass_mode, radius) = {
                    let db = app.state::<Db>();
                    let conn = db.0.lock().unwrap();
                    (
                        get_str_setting(&conn, "theme", "linen"),
                        get_str_setting(&conn, "glass", "off"),
                        radius_max(&get_str_setting(&conn, "corner_radius", "0,0,0,0")),
                    )
                };
                // 初始背景按主题色（开玻璃时用透明让 vibrancy 透出），消掉加载色变闪屏。
                let bg = if glass_mode == "off" {
                    theme_bg(&theme)
                } else {
                    tauri::window::Color(0, 0, 0, 0)
                };
                let _ = window.set_background_color(Some(bg));
                apply_glass(&window, &glass_mode, radius);
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
            purge_cache_now,
            dock_collapse,
            get_prefs,
            set_prefs,
            list_pets,
            open_pets_dir,
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
          dedup_key TEXT NOT NULL UNIQUE,
          image_path TEXT
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
          ('glass', 'off'),
          ('corner_radius', '0,0,0,0'),
          ('filter_rules', '继续' || char(10) || '你好'),
          ('retention_days', '3'),
          ('dock_mode', 'terminal'),
          ('attach_side', 'right'),
          ('dock_width', '360'),
          ('dock_height', '620'),
          ('dock_gap', '8'),
          ('follow_terminal', '1');
        "#,
    )
    .map_err(|error| tauri::Error::Anyhow(error.into()))?;

    // 兼容旧库：补上 image_path 列（已存在时忽略报错）。
    let _ = conn.execute("ALTER TABLE questions ADD COLUMN image_path TEXT", []);

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
        image_path: row.get(7)?,
    })
}

const QUESTION_COLUMNS: &str = "id, window_id, source, session_id, cwd, text, asked_at, image_path";

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

/// 立即按当前保留天数清一次缓存（用户在设置里手动点「立即清理」）。
#[tauri::command]
fn purge_cache_now(app: AppHandle, db: State<'_, Db>) -> Result<PurgeStats, String> {
    let stats = {
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        let days = get_i32_setting(&conn, "retention_days", 3);
        purge_cache(&app, &conn, days)
    };
    // 清掉过期记录后让浮窗刷新列表。
    let _ = app.emit("questions-updated", ());
    Ok(stats)
}

#[tauri::command]
fn get_prefs(db: State<'_, Db>) -> Result<Prefs, String> {
    let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
    Ok(Prefs {
        theme: get_str_setting(&conn, "theme", "linen"),
        font_face: get_str_setting(&conn, "font_face", "system"),
        font_size: get_str_setting(&conn, "font_size", "medium"),
        glass: match get_str_setting(&conn, "glass", "off").as_str() {
            "frosted" | "liquid" => get_str_setting(&conn, "glass", "off"),
            "1" | "true" => "frosted".to_string(),
            _ => "off".to_string(),
        },
        corner_radius: get_str_setting(&conn, "corner_radius", "0,0,0,0"),
        filter_rules: get_str_setting(&conn, "filter_rules", "继续\n你好"),
        retention_days: get_i32_setting(&conn, "retention_days", 3),
        dock_mode: get_str_setting(&conn, "dock_mode", "terminal"),
        attach_side: get_str_setting(&conn, "attach_side", "right"),
        dock_width: get_i32_setting(&conn, "dock_width", 360),
        dock_height: get_i32_setting(&conn, "dock_height", 620),
        follow: get_i32_setting(&conn, "follow_terminal", 1) != 0,
        // 自绘 tab 终端（多 tab 挤一个窗口）默认按项目分组；原生 tab / 真窗口默认不分组。
        group_drawn_tabs: get_i32_setting(&conn, "group_drawn_tabs", 1) != 0,
        group_native_windows: get_i32_setting(&conn, "group_native_windows", 0) != 0,
        // 刘海收起后把手的展开方式：hover(悬停滑出) / click(点击展开)。
        notch_expand: get_str_setting(&conn, "notch_expand", "hover"),
        // 收起把手上的边缘小人：off(细握把) 或某只 pet 的 id，默认内置“小芽”。
        notch_mascot: get_str_setting(&conn, "notch_mascot", "sprout"),
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
    glass: String,
    corner_radius: String,
    filter_rules: String,
    retention_days: i32,
    mode: String,
    side: String,
    width: i32,
    height: i32,
    follow: bool,
    group_drawn_tabs: bool,
    group_native_windows: bool,
    notch_expand: String,
    notch_mascot: String,
) -> Result<(), String> {
    let mode = if mode == "screen" { "screen" } else { "terminal" }.to_string();
    let glass = match glass.as_str() {
        "frosted" | "liquid" => glass,
        _ => "off".to_string(),
    };
    // 规整圆角字符串为 4 个 0..=40 的整数 "tl,tr,bl,br"
    let corner_radius = {
        let mut v: Vec<i32> = corner_radius
            .split(',')
            .map(|s| s.trim().parse::<i32>().unwrap_or(0).clamp(0, 40))
            .collect();
        v.resize(4, 0);
        format!("{},{},{},{}", v[0], v[1], v[2], v[3])
    };
    let side = match side.as_str() {
        "left" | "right" | "top" | "bottom" => side,
        _ => "right".to_string(),
    };
    let width = width.clamp(180, 900);
    let height = height.clamp(120, 1600);
    let notch_expand = if notch_expand == "click" { "click" } else { "hover" }.to_string();
    // 小人 id 只允许 [a-z0-9_-]，空/非法 → off（细握把）。
    let notch_mascot = {
        let s: String = notch_mascot
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if s.is_empty() { "off".to_string() } else { s }
    };
    {
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        set_setting(&conn, "theme", &theme)?;
        set_setting(&conn, "font_face", &font_face)?;
        set_setting(&conn, "font_size", &font_size)?;
        set_setting(&conn, "glass", &glass)?;
        set_setting(&conn, "corner_radius", &corner_radius)?;
        set_setting(&conn, "filter_rules", filter_rules.trim())?;
        set_setting(&conn, "retention_days", &retention_days.clamp(0, 365).to_string())?;
        set_setting(&conn, "dock_mode", &mode)?;
        set_setting(&conn, "attach_side", &side)?;
        set_setting(&conn, "dock_width", &width.to_string())?;
        set_setting(&conn, "dock_height", &height.to_string())?;
        set_setting(&conn, "follow_terminal", if follow { "1" } else { "0" })?;
        set_setting(&conn, "group_drawn_tabs", if group_drawn_tabs { "1" } else { "0" })?;
        set_setting(
            &conn,
            "group_native_windows",
            if group_native_windows { "1" } else { "0" },
        )?;
        set_setting(&conn, "notch_expand", &notch_expand)?;
        set_setting(&conn, "notch_mascot", &notch_mascot)?;
    }
    // 改了保留天数 → 立即清理一次
    if let Ok(conn) = db.0.lock() {
        purge_cache(&app, &conn, retention_days.clamp(0, 365));
    }
    if let Some(window) = app.get_webview_window("main") {
        apply_glass(&window, &glass, radius_max(&corner_radius));
    }
    let terminal = detect_front_terminal().ok().flatten().map(|win| win.rect);
    let _ = reposition_dock(&app, &db, terminal);
    let _ = app.emit("prefs-changed", ());
    Ok(())
}

/// $APPDATA/pets 目录：每只边缘小人一个子目录（pet.json + 精灵图）。
fn pets_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("pets");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// 首启把内置“小芽”落到 $APPDATA/pets/sprout/（精灵图存在则不覆盖，pet.json 始终刷新）。
fn ensure_default_pet(app: &AppHandle) {
    let Ok(base) = pets_dir(app) else { return };
    let dir = base.join("sprout");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    // 内置资源随版本更新 → 始终覆盖（用户想改应另建 pet，不要动内置 sprout）。
    let _ = fs::write(dir.join("spritesheet.png"), SPROUT_SPRITESHEET);
    let _ = fs::write(dir.join("pet.json"), SPROUT_PET_JSON);
}

/// 列出 $APPDATA/pets 下所有合法小人（pet.json 可解析且精灵图存在）。
/// 沿用 Codex Pets 的清单字段，所以社区现成宠物拖进来即可用。
#[tauri::command]
fn list_pets(app: AppHandle) -> Result<Vec<PetInfo>, String> {
    let base = pets_dir(&app)?;
    let mut pets: Vec<PetInfo> = Vec::new();
    let Ok(entries) = fs::read_dir(&base) else {
        return Ok(pets);
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(dir.join("pet.json")) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let folder = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let id = json
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&folder)
            .to_string();
        let display_name = json
            .get("displayName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();
        let description = json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sprite_rel = json
            .get("spritesheetPath")
            .and_then(|v| v.as_str())
            .unwrap_or("spritesheet.webp");
        let sprite = dir.join(sprite_rel);
        if !sprite.exists() {
            continue;
        }
        pets.push(PetInfo {
            id,
            display_name,
            description,
            spritesheet: sprite.to_string_lossy().to_string(),
        });
    }
    // 内置“小芽”排最前，其余按显示名排序。
    pets.sort_by(|a, b| {
        let rank = |p: &PetInfo| u8::from(p.id != "sprout");
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
    Ok(pets)
}

/// 在访达里打开 pets 目录，方便用户把社区小人（pet.json + 精灵图）拖进去。
#[tauri::command]
fn open_pets_dir(app: AppHandle) -> Result<(), String> {
    let dir = pets_dir(&app)?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 给窗口加/去 macOS 毛玻璃(NSVisualEffectView)。需要窗口透明 + CSS 半透明背景才看得到。
/// 主题对应的表面底色（= CSS --bg），用作窗口初始背景，避免加载时色变闪屏。
fn theme_bg(theme: &str) -> tauri::window::Color {
    let (r, g, b) = match theme {
        "midnight" => (14, 16, 20),
        "graphite" => (22, 22, 24),
        "ember" => (21, 15, 11),
        "indigo" => (19, 21, 43),
        "slate" => (241, 243, 246),
        _ => (244, 240, 231), // linen / 默认
    };
    tauri::window::Color(r, g, b, 255)
}

/// 主题表面色的十六进制（用于在内容绘制前注入 html 背景，盖住 webview 白底）。
fn theme_hex(theme: &str) -> &'static str {
    match theme {
        "midnight" => "#0e1014",
        "graphite" => "#161618",
        "ember" => "#150f0b",
        "indigo" => "#13152b",
        "slate" => "#f1f3f6",
        _ => "#f4f0e7", // linen / 默认
    }
}

/// 从 "tl,tr,bl,br" 取最大圆角(原生 vibrancy 只能整体一个半径，取最大值近似)。
fn radius_max(corner_radius: &str) -> f64 {
    corner_radius
        .split(',')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .fold(0.0_f64, f64::max)
        .clamp(0.0, 40.0)
}

/// 给浮窗加 macOS 玻璃：frosted=磨砂；liquid=磨砂+CSS 高光(模拟液态玻璃)；其它=去掉。
/// radius 为原生 vibrancy 的整体圆角(0 表示矩形)。
#[cfg(target_os = "macos")]
fn apply_glass(window: &tauri::WebviewWindow, mode: &str, radius: f64) {
    use window_vibrancy::{apply_vibrancy, clear_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
    // 先清掉旧的，避免切换模式/半径时叠加多层效果。
    let _ = clear_vibrancy(window);
    if mode != "frosted" && mode != "liquid" {
        return;
    }
    let radius = if radius > 0.0 { Some(radius) } else { None };
    let _ = apply_vibrancy(window, NSVisualEffectMaterial::Sidebar, Some(NSVisualEffectState::Active), radius);
}

#[cfg(not(target_os = "macos"))]
fn apply_glass(_window: &tauri::WebviewWindow, _mode: &str, _radius: f64) {}

/// 打开（或聚焦）独立的设置窗口。它加载同一前端，按窗口 label 渲染设置页。
#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    let (theme, font_face, font_size, glass) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        (
            get_str_setting(&conn, "theme", "linen"),
            get_str_setting(&conn, "font_face", "system"),
            get_str_setting(&conn, "font_size", "medium"),
            get_str_setting(&conn, "glass", "off"),
        )
    };
    // 绘制前就把真实外观设上去（data-* + html 底色），并注入全局给 React 读初始值，
    // 避免首帧按默认(亚麻)画一帧再变回真实主题的闪动。
    let bg = if glass == "off" {
        theme_hex(&theme).to_string()
    } else {
        "transparent".to_string()
    };
    let init = format!(
        "window.__ASKDOCK_INIT__={{theme:'{theme}',font_face:'{font_face}',font_size:'{font_size}',glass:'{glass}'}};\
         var r=document.documentElement;r.setAttribute('data-theme','{theme}');\
         r.setAttribute('data-face','{font_face}');r.setAttribute('data-size','{font_size}');\
         r.setAttribute('data-glass','{glass}');r.style.background='{bg}';"
    );
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
    .background_color(theme_bg(&theme))
    .initialization_script(init)
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

/// 按当前主题/玻璃设置恢复窗口表面（背景色 + vibrancy）。收起小人时会临时设透明，
/// 退出（normal）或滑出（peek）时用它还原，避免“小人装在不透明卡片里”的盒子感。
fn apply_window_surface(window: &tauri::WebviewWindow, conn: &Connection) {
    let theme = get_str_setting(conn, "theme", "linen");
    let glass_mode = get_str_setting(conn, "glass", "off");
    let radius = radius_max(&get_str_setting(conn, "corner_radius", "0,0,0,0"));
    let bg = if glass_mode == "off" {
        theme_bg(&theme)
    } else {
        tauri::window::Color(0, 0, 0, 0)
    };
    let _ = window.set_background_color(Some(bg));
    apply_glass(window, &glass_mode, radius);
}

/// 刘海式收起浮窗。`state`：
/// - `"handle"`：收成屏幕右缘纵向居中的小把手（选了小人时透明、放大）；
/// - `"peek"`：悬停时滑出全尺寸浮窗（仍贴右缘）；
/// - `"normal"`：退出收起，恢复正常贴靠。
#[tauri::command]
fn dock_collapse(app: AppHandle, db: State<'_, Db>, state: String) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?;

    if state == "normal" {
        // 恢复配置里的最小尺寸下限 + 正常窗口背景，再按正常逻辑重新贴靠。
        let _ = window.set_min_size(Some(PhysicalSize::new(240u32, 480u32)));
        if let Ok(conn) = db.0.lock() {
            apply_window_surface(&window, &conn);
        }
        let terminal = detect_front_terminal().ok().flatten().map(|win| win.rect);
        return reposition_dock(&app, &db, terminal);
    }

    // 桌面宠物模式（小人在桌面任意位置）下，在「当前窗口位置」就地变换，不钉屏幕右缘：
    // pet-expand=就地展开完整浮窗；pet-collapse=缩回小人；
    // pet-menu=临时增高、在小人下方腾出右键菜单的位置；pet-menu-end=菜单关闭后缩回小人。
    if matches!(
        state.as_str(),
        "pet-expand" | "pet-collapse" | "pet-menu" | "pet-menu-end"
    ) {
        let cur = window.outer_position().map_err(|error| error.to_string())?;
        let (w, h) = match state.as_str() {
            "pet-expand" => {
                let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
                apply_window_surface(&window, &conn); // 恢复不透明背景
                let _ = window.set_min_size(Some(PhysicalSize::new(240u32, 480u32)));
                (
                    get_i32_setting(&conn, "dock_width", 360),
                    get_i32_setting(&conn, "dock_height", 620),
                )
            }
            "pet-menu" => {
                let _ = window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
                let _ = window.set_min_size(Some(PhysicalSize::new(
                    NOTCH_MASCOT_W as u32,
                    NOTCH_MASCOT_H as u32,
                )));
                (NOTCH_MASCOT_W, NOTCH_MASCOT_H + PET_MENU_H)
            }
            _ => {
                // pet-collapse | pet-menu-end：缩回小人尺寸，透明背景
                let _ = window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
                let _ = window.set_min_size(Some(PhysicalSize::new(
                    NOTCH_MASCOT_W as u32,
                    NOTCH_MASCOT_H as u32,
                )));
                (NOTCH_MASCOT_W, NOTCH_MASCOT_H)
            }
        };
        // 保持当前左上角，按当前显示器把新尺寸 clamp 进屏幕内。
        let (mut x, mut y) = (cur.x, cur.y);
        if let Some(m) = window
            .current_monitor()
            .ok()
            .flatten()
            .or_else(|| window.primary_monitor().ok().flatten())
        {
            let mp = m.position();
            let ms = m.size();
            x = x.clamp(mp.x, (mp.x + ms.width as i32 - w).max(mp.x));
            y = y.clamp(mp.y, (mp.y + ms.height as i32 - h).max(mp.y));
        }
        // 先改尺寸再定位（clamp 后的左上角保证不越出屏幕）。
        window
            .set_size(PhysicalSize::new(w as u32, h as u32))
            .map_err(|error| error.to_string())?;
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(|| "无法读取显示器信息".to_string())?;
    let pos = monitor.position();
    let size = monitor.size();

    let peek = state == "peek";
    let (w, h) = if peek {
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        // 从透明把手滑出完整浮窗：恢复正常背景。
        apply_window_surface(&window, &conn);
        (
            get_i32_setting(&conn, "dock_width", 360),
            get_i32_setting(&conn, "dock_height", 620),
        )
    } else {
        let conn = db.0.lock().map_err(|_| "数据库繁忙".to_string())?;
        // 选了边缘小人 → 把手放大到能放下精灵，且背景透明让小人像直接趴在屏幕边；
        // 否则细握把：维持正常不透明背景。
        let has_mascot = get_str_setting(&conn, "notch_mascot", "sprout") != "off";
        if has_mascot {
            let _ = window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
        } else {
            apply_window_surface(&window, &conn);
        }
        let (hw, hh) = if has_mascot {
            (NOTCH_MASCOT_W, NOTCH_MASCOT_H)
        } else {
            (NOTCH_HANDLE_W, NOTCH_HANDLE_H)
        };
        // 把手比配置里的 minWidth/minHeight 小，先放开最小尺寸下限。
        let _ = window.set_min_size(Some(PhysicalSize::new(hw as u32, hh as u32)));
        (hw, hh)
    };

    let x = pos.x + size.width as i32 - w; // 贴屏幕右缘
    let y = pos.y + (size.height as i32 - h) / 2; // 纵向居中
    let new_pos = PhysicalPosition::new(x, y);
    let new_size = PhysicalSize::new(w as u32, h as u32);

    // 放大（peek）先移位再变大、缩小（handle）先变小再移位，避免窗口越出屏幕右缘。
    if peek {
        window.set_position(new_pos).map_err(|e| e.to_string())?;
        window.set_size(new_size).map_err(|e| e.to_string())?;
    } else {
        window.set_size(new_size).map_err(|e| e.to_string())?;
        window.set_position(new_pos).map_err(|e| e.to_string())?;
    }
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

    // 一次性回填：给历史 Codex 记录补上 cwd（这些行入库时还没存 cwd），消除"未归类"。
    backfill_codex_cwd(&app);

    // 缓存每个 Codex 会话文件的元数据（来源是否终端 + 会话 cwd），首次有新行时读一次。
    let mut codex_meta: HashMap<PathBuf, CodexMeta> = HashMap::new();
    let mut purge_counter: u32 = 0;
    loop {
        // 每 ~1 小时清理一次过期缓存（首轮也清）。
        if purge_counter == 0 {
            if let Some(db) = app.try_state::<Db>() {
                if let Ok(conn) = db.0.lock() {
                    let days = get_i32_setting(&conn, "retention_days", 3);
                    purge_cache(&app, &conn, days);
                }
            }
        }
        purge_counter = (purge_counter + 1) % 4500; // 4500 * 800ms ≈ 1 小时

        let mut captured_any = false;
        for (root, source) in &sources {
            let mut files = Vec::new();
            collect_jsonl(root, &mut files, 0);
            for file in files {
                let mut parsed_list = read_new_questions(&file, &mut offsets, source);
                if parsed_list.is_empty() {
                    continue;
                }
                if source == "codex" {
                    let meta = codex_meta
                        .entry(file.clone())
                        .or_insert_with(|| read_codex_meta(&file));
                    // Codex 桌面 app / IDE 扩展的提问不属于任何终端窗口，丢弃它们，
                    // 免得被错误归到捕获时恰好在前台的终端。
                    if !meta.is_terminal {
                        continue;
                    }
                    // 用会话 cwd 补上 Codex 的工作目录（前端据此按项目分组）。
                    if let Some(cwd) = &meta.cwd {
                        for p in &mut parsed_list {
                            if p.cwd.is_none() {
                                p.cwd = Some(cwd.clone());
                            }
                        }
                    }
                }
                for parsed in parsed_list {
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

/// 用户自定义过滤：整条提问（去掉首尾空白和末尾标点、忽略大小写）正好等于
/// 某条规则时返回 true。规则按行分隔。用于过滤“继续”“你好”这类催状态的短消息。
fn is_filtered(text: &str, rules: &str) -> bool {
    let normalize = |s: &str| {
        s.trim()
            .trim_end_matches(|c: char| {
                c.is_whitespace() || ",.。!！?？、;；:：…~ ".contains(c)
            })
            .trim()
            .to_lowercase()
    };
    let t = normalize(text);
    if t.is_empty() {
        return false;
    }
    rules
        .lines()
        .map(normalize)
        .any(|rule| !rule.is_empty() && rule == t)
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
    // Claude 的图片是内嵌 base64 块；一条消息可能有多张，全部收集。
    let images: Vec<CapturedImage> = content
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) != Some("image") {
                        return None;
                    }
                    let src = b.get("source")?;
                    let data = src.get("data").and_then(|d| d.as_str())?;
                    let media = src
                        .get("media_type")
                        .and_then(|m| m.as_str())
                        .unwrap_or("image/png");
                    let ext = media.rsplit('/').next().unwrap_or("png");
                    let ext = if ext == "jpeg" { "jpg" } else { ext };
                    Some(CapturedImage::Base64 {
                        ext: ext.to_string(),
                        data: data.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let raw = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(_) => join_text_blocks(content, "text", "type"),
        _ => return None,
    };
    let mut text = clean_prompt(&raw).trim().to_string();
    if text.is_empty() && !images.is_empty() {
        text = IMAGE_MARKER.to_string();
    }
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
        images,
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
    // Codex 把图片写成 <image … path="…"> ；一条消息可能有多张，抽出全部路径复制持久化
    let images: Vec<CapturedImage> = extract_codex_image_paths(&raw)
        .into_iter()
        .map(CapturedImage::Path)
        .collect();
    let has_image = !images.is_empty()
        || raw.contains("<image")
        || content.as_array().map_or(false, |arr| {
            arr.iter().any(|b| {
                matches!(
                    b.get("type").and_then(|t| t.as_str()),
                    Some("input_image") | Some("image")
                )
            })
        });
    let mut text = clean_prompt(&raw).trim().to_string();
    if text.is_empty() && has_image {
        text = IMAGE_MARKER.to_string();
    }
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
        images,
    })
}

/// Codex 会话是不是「在终端里跑」的。会话文件首行 `session_meta` 里带 `source`：
/// `cli`(codex-tui)/`exec`(codex exec)是终端来源 → 捕获；`vscode` 等是
/// Codex 桌面 app / IDE 扩展 → 不是在终端里问的，必须排除，否则它的提问会被
/// 错误归到捕获那一刻恰好在前台的某个终端窗口。读不到/老格式则按终端处理（兼容）。
/// Codex 会话文件首行 `session_meta` 里的两项信息：是否终端来源、会话工作目录。
#[derive(Debug, Clone)]
struct CodexMeta {
    is_terminal: bool,
    cwd: Option<String>,
}

fn read_codex_meta(path: &Path) -> CodexMeta {
    use std::io::BufRead;
    let Ok(file) = fs::File::open(path) else {
        return CodexMeta { is_terminal: true, cwd: None };
    };
    let mut line = String::new();
    if std::io::BufReader::new(file).read_line(&mut line).is_err() {
        return CodexMeta { is_terminal: true, cwd: None };
    }
    parse_codex_meta(&line)
}

/// 从 `session_meta` 首行 JSON 解析（是否终端来源, cwd）。`source` 为 `cli`/`exec`
/// 才是终端来源；读不到首行、首行不是 meta、或没有 `source` 字段（老格式）按终端处理。
fn parse_codex_meta(line: &str) -> CodexMeta {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return CodexMeta { is_terminal: true, cwd: None };
    };
    if v.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
        return CodexMeta { is_terminal: true, cwd: None };
    }
    let payload = v.get("payload");
    let is_terminal = match payload.and_then(|p| p.get("source")).and_then(|s| s.as_str()) {
        Some(s) => s == "cli" || s == "exec",
        None => true,
    };
    let cwd = payload
        .and_then(|p| p.get("cwd"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(|c| c.to_string());
    CodexMeta { is_terminal, cwd }
}

/// 一次性回填历史 Codex 记录的 cwd：早先入库的 Codex 提问没有 cwd（前端会把它们
/// 归到"未归类"组）。每条记录都存了 `session_id`（= 会话文件名 stem），据此找回
/// 会话文件、读出 `session_meta.cwd` 补写回去。找不到文件或文件里没 cwd 的保持 NULL。
fn backfill_codex_cwd(app: &AppHandle) {
    let Some(db) = app.try_state::<Db>() else {
        return;
    };
    let Ok(conn) = db.0.lock() else {
        return;
    };

    // 哪些会话还缺 cwd
    let session_ids: Vec<String> = {
        let Ok(mut stmt) = conn.prepare(
            "SELECT DISTINCT session_id FROM questions
             WHERE source = 'codex' AND cwd IS NULL AND session_id IS NOT NULL",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
            return;
        };
        rows.flatten().collect()
    };
    if session_ids.is_empty() {
        return;
    }

    // 建 文件名stem → 路径 映射
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let mut files = Vec::new();
    collect_jsonl(&PathBuf::from(home).join(".codex/sessions"), &mut files, 0);
    let by_stem: HashMap<String, PathBuf> = files
        .into_iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| (s.to_string(), p.clone()))
        })
        .collect();

    let mut filled = 0usize;
    for sid in session_ids {
        if let Some(path) = by_stem.get(&sid) {
            if let Some(cwd) = read_codex_meta(path).cwd {
                filled += conn
                    .execute(
                        "UPDATE questions SET cwd = ?1
                         WHERE source = 'codex' AND session_id = ?2 AND cwd IS NULL",
                        params![cwd, sid],
                    )
                    .unwrap_or(0);
            }
        }
    }
    drop(conn);
    if filled > 0 {
        let _ = app.emit("questions-updated", ());
    }
}

/// 从 Codex 的 `<image … path="…">` 里抽出所有文件路径（一条消息可能多张）。
fn extract_codex_image_paths(raw: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find("<image") {
        let after = &rest[start + "<image".len()..];
        // 该 <image 标签的结束位置，路径只在标签内部找，避免跨标签误抓
        let tag_end = after.find('>').unwrap_or(after.len());
        let tag = &after[..tag_end];
        if let Some(p) = tag.find("path=\"") {
            let val = &tag[p + "path=\"".len()..];
            if let Some(end) = val.find('"') {
                let path = &val[..end];
                if !path.is_empty() {
                    paths.push(path.to_string());
                }
            }
        }
        rest = &after[tag_end..];
    }
    paths
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

/// 缓存清理：删掉超过保留天数的会话记录，并清掉 images 目录里没人引用的孤儿图片。
/// retention_days <= 0 表示永久保留（只清孤儿图片）。返回本次清掉的记录数和图片数。
fn purge_cache(app: &AppHandle, conn: &Connection, retention_days: i32) -> PurgeStats {
    let mut rows = 0usize;
    if retention_days > 0 {
        let cutoff = (Utc::now() - chrono::Duration::days(retention_days as i64)).to_rfc3339();
        rows = conn
            .execute(
                "DELETE FROM questions WHERE datetime(created_at) < datetime(?1)",
                params![cutoff],
            )
            .unwrap_or(0);
    }

    // 删掉 images 目录里不再被任何会话引用的图片文件（含用户手动删/清空留下的）。
    let Ok(dir) = app.path().app_data_dir().map(|d| d.join("images")) else {
        return PurgeStats { rows, images: 0 };
    };
    let used: std::collections::HashSet<String> = {
        let mut set = std::collections::HashSet::new();
        if let Ok(mut stmt) =
            conn.prepare("SELECT image_path FROM questions WHERE image_path IS NOT NULL")
        {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                for joined in rows.flatten() {
                    // image_path 可能是多张图换行连起来的，逐一登记。
                    for path in joined.split('\n').filter(|p| !p.is_empty()) {
                        set.insert(path.to_string());
                    }
                }
            }
        }
        set
    };
    let mut images = 0usize;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && !used.contains(&path.to_string_lossy().to_string()) {
                if fs::remove_file(&path).is_ok() {
                    images += 1;
                }
            }
        }
    }
    PurgeStats { rows, images }
}

/// 归类并入库；返回是否真的新插入了一条（去重命中则返回 false）。
///
/// 归类规则：归到**写入那一刻的前台终端窗口**。因为我们只抓用户真正手敲
/// 的提问（合成消息、工具结果、agent-history 都已过滤），它必然发生在当前
/// 聚焦的窗口里，所以当下前台即正确归属。**不再做会话黏定** —— 黏定会把一
/// 个会话永久锁死在它第一次出现的窗口上，导致同一应用的多窗口分不开。
fn store_question(app: &AppHandle, source: &str, parsed: ParsedQuestion) -> bool {
    let Some(db) = app.try_state::<Db>() else {
        return false;
    };

    // 命中用户过滤规则（“继续/你好”等催状态短消息）则直接跳过，不入库。
    {
        let Ok(conn) = db.0.lock() else {
            return false;
        };
        let rules = get_str_setting(&conn, "filter_rules", "");
        if is_filtered(&parsed.text, &rules) {
            return false;
        }
    }

    let window_id = detect_front_terminal()
        .ok()
        .flatten()
        .map(|win| win.window_id.to_string());

    // 把附带的图片（可能多张）持久化到 AskDock 自己的目录，多张路径用换行连起来存。
    let paths: Vec<String> = parsed
        .images
        .iter()
        .filter_map(|img| persist_image(app, img))
        .collect();
    let image_path = if paths.is_empty() {
        None
    } else {
        Some(paths.join("\n"))
    };

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let Ok(conn) = db.0.lock() else {
        return false;
    };
    conn.execute(
        "INSERT OR IGNORE INTO questions
           (id, window_id, source, session_id, cwd, text, asked_at, created_at, dedup_key, image_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            window_id,
            source,
            parsed.session_id,
            parsed.cwd,
            parsed.text,
            parsed.asked_at,
            now,
            parsed.dedup_key,
            image_path
        ],
    )
    .map(|changed| changed > 0)
    .unwrap_or(false)
}

/// 把抓到的图片落到 `app_data_dir/images/<uuid>.<ext>`，返回保存后的绝对路径。
/// Claude 是内嵌 base64(解码写入)；Codex 是临时文件路径(复制一份)。
fn persist_image(app: &AppHandle, image: &CapturedImage) -> Option<String> {
    let dir = app.path().app_data_dir().ok()?.join("images");
    fs::create_dir_all(&dir).ok()?;
    let id = Uuid::new_v4().to_string();
    match image {
        CapturedImage::Base64 { ext, data } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.as_bytes())
                .ok()?;
            let path = dir.join(format!("{id}.{ext}"));
            fs::write(&path, bytes).ok()?;
            Some(path.to_string_lossy().to_string())
        }
        CapturedImage::Path(src) => {
            let src_path = Path::new(src);
            if !src_path.exists() {
                return None;
            }
            let ext = src_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png");
            let path = dir.join(format!("{id}.{ext}"));
            fs::copy(src_path, &path).ok()?;
            Some(path.to_string_lossy().to_string())
        }
    }
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
    use super::{calculate_dock_rect, is_real_prompt, parse_codex_meta, Rect};

    #[test]
    fn codex_terminal_sources_kept_gui_excluded() {
        let meta = |source: &str| {
            format!(r#"{{"type":"session_meta","payload":{{"id":"x","source":"{source}"}}}}"#)
        };
        // 终端来源 → 捕获
        assert!(parse_codex_meta(&meta("cli")).is_terminal);
        assert!(parse_codex_meta(&meta("exec")).is_terminal);
        // Codex 桌面 app / IDE → 排除
        assert!(!parse_codex_meta(&meta("vscode")).is_terminal);
        // 老格式 / 异常 → 兼容地按终端处理
        assert!(parse_codex_meta(r#"{"type":"session_meta","payload":{"id":"x"}}"#).is_terminal);
        assert!(parse_codex_meta(r#"{"type":"response_item"}"#).is_terminal);
        assert!(parse_codex_meta("not json").is_terminal);
    }

    #[test]
    fn codex_meta_extracts_cwd() {
        let line = r#"{"type":"session_meta","payload":{"source":"cli","cwd":"/Users/x/proj"}}"#;
        assert_eq!(parse_codex_meta(line).cwd.as_deref(), Some("/Users/x/proj"));
        // 没有 cwd / 空 cwd → None
        let line2 = r#"{"type":"session_meta","payload":{"source":"cli","cwd":""}}"#;
        assert_eq!(parse_codex_meta(line2).cwd, None);
        assert_eq!(parse_codex_meta(r#"{"type":"session_meta","payload":{}}"#).cwd, None);
    }

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
    fn filter_rules_match_whole_message_only() {
        let rules = "继续\n你好\nok";
        assert!(super::is_filtered("继续", rules));
        assert!(super::is_filtered("继续。", rules)); // 末尾标点忽略
        assert!(super::is_filtered("  你好  ", rules)); // 首尾空白忽略
        assert!(super::is_filtered("OK", rules)); // 大小写忽略
        assert!(!super::is_filtered("继续优化这个函数", rules)); // 不是整条匹配
        assert!(!super::is_filtered("你好，帮我看下报错", rules));
        assert!(!super::is_filtered("继续", "")); // 无规则不过滤
    }

    #[test]
    fn cleans_image_noise_from_prompt() {
        let raw = "<image name=[Image #1] path=\"/tmp/codex-clipboard-x.png\"></image>[Image #1] 这页面太粗糙了，设计一下。";
        assert_eq!(super::clean_prompt(raw).trim(), "这页面太粗糙了，设计一下。");
        // 纯图片消息清理后为空 → 不算有效提问
        assert!(super::clean_prompt("<image path=\"/tmp/a.png\"></image>").trim().is_empty());
    }

    #[test]
    fn extracts_all_codex_image_paths() {
        // 一条消息里多张图：全部抽出，且路径只在各自标签内取。
        let raw = "<image name=[Image #1] path=\"/tmp/a.png\"></image>看下\
                   <image name=[Image #2] path=\"/tmp/b.jpg\"></image>这两张";
        assert_eq!(
            super::extract_codex_image_paths(raw),
            vec!["/tmp/a.png".to_string(), "/tmp/b.jpg".to_string()]
        );
        // 没有图片时返回空。
        assert!(super::extract_codex_image_paths("纯文字").is_empty());
        // 空 path 跳过。
        assert!(super::extract_codex_image_paths("<image path=\"\">").is_empty());
    }
}
