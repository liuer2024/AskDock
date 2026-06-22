import React from "react";
import ReactDOM from "react-dom/client";
import { Check, Copy, Filter, Info, Palette, PanelRight, Settings, Type, X, Zap } from "lucide-react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./styles.css";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke?: unknown;
    };
  }
}

type CapturedQuestion = {
  id: string;
  window_id: string | null;
  source: string;
  session_id: string | null;
  cwd: string | null;
  text: string;
  asked_at: string;
  image_path: string | null;
};

const IMAGE_MARKER = "🖼 图片";

function imageSrc(path: string) {
  return isTauriRuntime() ? convertFileSrc(path) : path;
}

type FrontStatus = {
  window_id: string | null;
  app_name: string | null;
  message: string;
};

type Prefs = {
  theme: string;
  font_face: string;
  font_size: string;
  glass: string;
  corner_radius: string;
  filter_rules: string;
  retention_days: number;
  dock_mode: string;
  attach_side: string;
  dock_width: number;
  dock_height: number;
  follow: boolean;
};

const DEFAULT_PREFS: Prefs = {
  theme: "linen",
  font_face: "system",
  font_size: "medium",
  glass: "off",
  corner_radius: "0,0,0,0",
  filter_rules: "继续\n你好",
  retention_days: 3,
  dock_mode: "terminal",
  attach_side: "right",
  dock_width: 360,
  dock_height: 620,
  follow: true
};

const THEMES: { id: string; name: string; bg: string; a: string; b: string }[] = [
  { id: "midnight", name: "午夜", bg: "#0e1014", a: "#3fcf8e", b: "#d97757" },
  { id: "graphite", name: "石墨", bg: "#161618", a: "#57d4a1", b: "#e08a63" },
  { id: "ember", name: "暖炭", bg: "#150f0b", a: "#5fcf8e", b: "#ff8c5a" },
  { id: "indigo", name: "靛蓝", bg: "#13152b", a: "#5be0a8", b: "#ff9b6b" },
  { id: "linen", name: "亚麻", bg: "#f4f0e7", a: "#18895a", b: "#bf552f" },
  { id: "slate", name: "石板", bg: "#f1f3f6", a: "#0f9b69", b: "#c2562f" }
];

const APPEARANCE_KEY = "askdock.appearance";

// 首帧用的外观：优先后端注入(设置窗口)，否则用 localStorage 缓存(主窗口)。
// 让 React 第一次渲染就拿到正确主题，避免按 DEFAULT_PREFS(亚麻) 先画一帧再变回去的闪动。
function initialAppearance(): Partial<Prefs> {
  try {
    const injected = (window as unknown as { __ASKDOCK_INIT__?: Partial<Prefs> }).__ASKDOCK_INIT__;
    if (injected && injected.theme) {
      return {
        theme: injected.theme,
        font_face: injected.font_face,
        font_size: injected.font_size,
        glass: injected.glass
      };
    }
    const a = JSON.parse(localStorage.getItem(APPEARANCE_KEY) || "{}");
    const out: Partial<Prefs> = {};
    if (a.theme) out.theme = a.theme;
    if (a.face) out.font_face = a.face;
    if (a.size) out.font_size = a.size;
    if (a.glass) out.glass = a.glass;
    return out;
  } catch {
    return {};
  }
}

function initialPrefs(): Prefs {
  return { ...DEFAULT_PREFS, ...initialAppearance() };
}

const browserKey = "askdock.browser.questions";
const BROWSER_WINDOW_ID = "browser-preview";
const isTauriRuntime = () => typeof window !== "undefined" && typeof window.__TAURI_INTERNALS__?.invoke === "function";

function isSettingsWindow() {
  if (isTauriRuntime()) {
    try {
      return getCurrentWindow().label === "settings";
    } catch {
      return false;
    }
  }
  return typeof location !== "undefined" && location.hash === "#settings";
}

async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauriRuntime()) {
    return invoke<T>(name, args);
  }
  return browserCommand<T>(name, args ?? {});
}

// 浏览器预览：抓取/设置是后端能力，这里塞假数据让布局可见。
async function browserCommand<T>(name: string, args: Record<string, unknown>): Promise<T> {
  if (name === "poll_front_window") {
    return { window_id: BROWSER_WINDOW_ID, app_name: "Browser Preview", message: "浏览器预览模式" } as T;
  }
  if (name === "get_window_questions") {
    return readBrowserQuestions() as T;
  }
  if (name === "delete_question") {
    writeBrowserQuestions(readBrowserQuestions().filter((q) => q.id !== args.id));
    return undefined as T;
  }
  if (name === "clear_window") {
    writeBrowserQuestions([]);
    return undefined as T;
  }
  if (name === "get_prefs") {
    const t = new URLSearchParams(location.search).get("theme");
    return { ...DEFAULT_PREFS, ...(t ? { theme: t } : {}) } as T;
  }
  if (name === "open_settings") {
    location.hash = "#settings";
    location.reload();
    return undefined as T;
  }
  if (name === "hide_main_window" || name === "set_prefs") {
    return undefined as T;
  }
  throw new Error(`Unsupported browser command: ${name}`);
}

function readBrowserQuestions(): CapturedQuestion[] {
  try {
    const stored = localStorage.getItem(browserKey);
    if (stored) return JSON.parse(stored) as CapturedQuestion[];
  } catch {
    /* ignore */
  }
  const now = Date.now();
  const mk = (id: string, source: string, text: string, ms: number): CapturedQuestion => ({
    id, window_id: BROWSER_WINDOW_ID, source, session_id: null, cwd: null, text, asked_at: new Date(now - ms).toISOString(), image_path: null
  });
  const sample: CapturedQuestion[] = [
    mk("1", "codex", "你开一个新的分支，将这些问题，用去控件化处理一下。", 2 * 60000),
    mk("2", "codex", "现在的元素都包了边框、底色、阴影、强调色块，控件感太强，于是显得“重”。看看是不是有这个问题？", 6 * 60000),
    mk("3", "claude", "把时间戳和来源都换成等宽字体，整体节奏会更像终端日志，也更耐看。", 90 * 60000),
    mk("4", "claude", "左侧用一条连续的时间轴串起所有提问，最新的一条让它像光标一样在跳。", 110 * 60000),
    mk("5", "codex", "删除按钮先去掉。归档面板的重点是回看和复用，不是管理，操作越少越安静。", 26 * 3600000),
    mk("6", "claude", "内容直接占满整行，不要卡片。让文字本身成为主角，留白来分隔，而不是边框。", 30 * 3600000)
  ];
  writeBrowserQuestions(sample);
  return sample;
}

function writeBrowserQuestions(items: CapturedQuestion[]) {
  localStorage.setItem(browserKey, JSON.stringify(items));
}

function sourceLabel(source: string) {
  if (source === "codex") return "Codex";
  if (source === "claude") return "Claude Code";
  return source;
}

function timeLabel(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }).format(date);
}

// 时间线按天分隔：今天 / 昨天 / MM / DD
function dayLabel(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const startOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOf(new Date()) - startOf(date)) / 86400000);
  if (days === 0) return "今天";
  if (days === 1) return "昨天";
  const mm = String(date.getMonth() + 1).padStart(2, "0");
  const dd = String(date.getDate()).padStart(2, "0");
  return `${mm} / ${dd}`;
}

function useApplyAppearance(prefs: Prefs) {
  React.useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", prefs.theme);
    root.setAttribute("data-face", prefs.font_face);
    root.setAttribute("data-size", prefs.font_size);
    root.setAttribute("data-glass", prefs.glass || "off");
    const [tl, tr, bl, br] = (prefs.corner_radius || "0,0,0,0").split(",").map((n) => Number(n) || 0);
    root.style.setProperty("--radius", `${tl}px ${tr}px ${br}px ${bl}px`); // CSS 顺序: 左上 右上 右下 左下
    // 缓存外观，供下次开窗在绘制前(index.html 内联脚本)套上，避免首帧闪默认主题。
    try {
      const glassOn = prefs.glass && prefs.glass !== "off";
      const bg = glassOn ? "transparent" : THEMES.find((t) => t.id === prefs.theme)?.bg || "";
      localStorage.setItem(
        APPEARANCE_KEY,
        JSON.stringify({ theme: prefs.theme, face: prefs.font_face, size: prefs.font_size, glass: prefs.glass, bg })
      );
      root.style.background = bg;
    } catch {
      /* ignore */
    }
  }, [prefs.theme, prefs.font_face, prefs.font_size, prefs.glass, prefs.corner_radius]);
}

type Option = [value: string, label: string];

function Segmented({ value, onChange, options }: { value: string; onChange: (v: string) => void; options: Option[] }) {
  return (
    <div className="segmented">
      {options.map(([val, label]) => (
        <button key={val} type="button" className={value === val ? "on" : ""} onClick={() => onChange(val)}>
          {label}
        </button>
      ))}
    </div>
  );
}

/* ===================== 浮窗（主窗口） ===================== */

function Dock() {
  const [windowId, setWindowId] = React.useState<string | null>(null);
  const [appName, setAppName] = React.useState<string | null>(null);
  const [questions, setQuestions] = React.useState<CapturedQuestion[]>([]);
  const [message, setMessage] = React.useState("");
  const [freeMode, setFreeMode] = React.useState(false);
  const [prefs, setPrefs] = React.useState<Prefs>(initialPrefs);
  const [copiedId, setCopiedId] = React.useState<string | null>(null);
  const [lightbox, setLightbox] = React.useState<string | null>(null);

  const windowIdRef = React.useRef<string | null>(null);
  windowIdRef.current = windowId;
  const freeModeRef = React.useRef(false);
  freeModeRef.current = freeMode;
  const holdingTitle = React.useRef(false);

  useApplyAppearance(prefs);

  // 偏好来自后端（设置窗口写、prefs-changed 广播），保证两个窗口一致。
  React.useEffect(() => {
    const load = async () => {
      try {
        setPrefs(await command<Prefs>("get_prefs"));
      } catch {
        /* ignore */
      }
    };
    load();
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    listen("prefs-changed", load).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

  const refresh = React.useCallback(async (wid: string | null) => {
    if (!wid) return;
    try {
      const items = await command<CapturedQuestion[]>("get_window_questions", { windowId: wid });
      if (windowIdRef.current === wid) setQuestions(items);
    } catch (error) {
      setMessage(String(error));
    }
  }, []);

  React.useEffect(() => {
    let disposed = false;
    const tick = async () => {
      try {
        const status = await command<FrontStatus>("poll_front_window", { reposition: !freeModeRef.current });
        if (disposed) return;
        setMessage(freeModeRef.current ? "自由位置（已停止贴靠）" : status.message);
        if (status.window_id) {
          if (status.app_name) setAppName(status.app_name);
          if (status.window_id !== windowIdRef.current) {
            windowIdRef.current = status.window_id; // 立即同步，保证下面 refresh 的归属判断正确
            setWindowId(status.window_id);
            await refresh(status.window_id);
          }
        }
      } catch (error) {
        if (!disposed) setMessage(String(error));
      }
    };
    tick();
    const id = window.setInterval(tick, 1200);
    return () => {
      disposed = true;
      window.clearInterval(id);
    };
  }, [refresh]);

  React.useEffect(() => {
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    listen("questions-updated", () => refresh(windowIdRef.current)).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [refresh]);

  // 拖动标题栏移动后进入“自由位置”，停止贴靠回弹。
  React.useEffect(() => {
    if (!isTauriRuntime()) return;
    const onUp = () => {
      holdingTitle.current = false;
    };
    window.addEventListener("pointerup", onUp);
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onMoved(() => {
        if (holdingTitle.current) setFreeMode(true);
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      window.removeEventListener("pointerup", onUp);
      unlisten?.();
    };
  }, []);

  async function clearAll() {
    if (!windowId) return;
    await command("clear_window", { windowId });
    await refresh(windowId);
  }

  async function hideWindow() {
    try {
      await command("hide_main_window");
    } catch (error) {
      setMessage(String(error));
    }
  }

  const copyTimer = React.useRef<number | undefined>(undefined);
  async function copy(item: CapturedQuestion) {
    const imageOnly = item.image_path && (!item.text || item.text === IMAGE_MARKER);
    try {
      if (imageOnly) {
        // 纯图片条目 → 复制图片本身到剪贴板（多张时取第一张）
        const first = item.image_path!.split("\n").filter(Boolean)[0];
        const res = await fetch(imageSrc(first));
        const blob = await res.blob();
        await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
      } else {
        await navigator.clipboard.writeText(item.text);
      }
    } catch {
      navigator.clipboard?.writeText(item.text).catch(() => undefined);
    }
    setCopiedId(item.id);
    window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopiedId(null), 1400);
  }

  // 按天分组：插入日期分隔，并标出最新一条（节点会跳动）
  let prevDay = "";
  const rows = questions.map((q, index) => {
    const day = dayLabel(q.asked_at);
    const showMark = day !== prevDay;
    prevDay = day;
    return { q, latest: index === 0, day, showMark };
  });

  return (
    <main className="dock">
      <header className="titlebar" data-tauri-drag-region onPointerDown={() => { holdingTitle.current = true; }}>
        <span className="brand">AskDock</span>
        <button
          className={`ic ${freeMode ? "" : "live"}`}
          type="button"
          onClick={() => setFreeMode((v) => !v)}
          title={freeMode ? "自由位置 · 点击重新贴靠终端" : "实时贴靠中 · 点击解锁后可拖动标题栏移动"}
        >
          <Zap size={15} />
        </button>
        <button className="ic" type="button" onClick={hideWindow} title="关闭（点托盘图标可重新打开）">
          <X size={15} />
        </button>
      </header>

      <div className="sessionbar">
        <div className="session">
          <span className="pr">&gt;_</span>
          <span className="nm">{windowId ? appName ?? "终端" : "等待终端窗口"}</span>
        </div>
        {questions.length ? <button className="clear" type="button" onClick={clearAll}>清空</button> : null}
      </div>

      <div className="stream">
        {questions.length === 0 ? (
          <div className="streamEmpty">
            在终端里问 Claude / Codex，<br />提问会顺着时间线出现在这里
          </div>
        ) : (
          rows.map(({ q, latest, day, showMark }) => (
            <React.Fragment key={q.id}>
              {showMark ? (
                <div className="daymark"><span>{day}</span><div className="rule" /></div>
              ) : null}
              <article className={`entry ${latest ? "latest" : ""}`} data-src={q.source}>
                <div className="node" />
                <button className={`copy ${copiedId === q.id ? "done" : ""}`} type="button" title="复制" onClick={() => copy(q)}>
                  {copiedId === q.id ? <Check size={13} /> : <Copy size={13} />}
                </button>
                <div className="meta">
                  <span className="src">{sourceLabel(q.source)}</span>
                  <span className="dot" />
                  <span>{timeLabel(q.asked_at)}</span>
                </div>
                {q.text && q.text !== IMAGE_MARKER ? <div className="body">{q.text}</div> : null}
                {q.image_path ? (
                  <div className="qImgs">
                    {q.image_path.split("\n").filter(Boolean).map((p, i) => (
                      <img
                        key={i}
                        className="qImg"
                        src={imageSrc(p)}
                        alt="图片"
                        loading="lazy"
                        onClick={() => setLightbox(imageSrc(p))}
                      />
                    ))}
                  </div>
                ) : q.text === IMAGE_MARKER ? (
                  <div className="body muted">{IMAGE_MARKER}</div>
                ) : null}
              </article>
            </React.Fragment>
          ))
        )}
      </div>

      <footer className="footer">
        <div className="status">
          <span className="pin" />
          <span>{freeMode ? "自由位置（已停止贴靠）" : message || "等待终端窗口"}</span>
        </div>
        <button className="settings" type="button" onClick={() => command("open_settings").catch(() => undefined)}>
          <Settings size={14} /> <span>设置</span>
        </button>
      </footer>

      {lightbox ? <Lightbox src={lightbox} onClose={() => setLightbox(null)} /> : null}
    </main>
  );
}

/* ===================== 图片查看器（滚轮缩放 / 拖动平移 / 双击复位） ===================== */

function Lightbox({ src, onClose }: { src: string; onClose: () => void }) {
  const [zoom, setZoom] = React.useState(1);
  const [pan, setPan] = React.useState({ x: 0, y: 0 });
  const drag = React.useRef<{ x: number; y: number } | null>(null);
  const rootRef = React.useRef<HTMLDivElement | null>(null);

  React.useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
      setZoom((z) => Math.min(8, Math.max(1, z * factor)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  React.useEffect(() => {
    if (zoom <= 1) setPan({ x: 0, y: 0 });
  }, [zoom]);

  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      ref={rootRef}
      className="lightbox"
      onClick={onClose}
      onPointerMove={(e) => {
        if (drag.current) setPan({ x: e.clientX - drag.current.x, y: e.clientY - drag.current.y });
      }}
      onPointerUp={() => (drag.current = null)}
      onPointerLeave={() => (drag.current = null)}
    >
      <img
        src={src}
        alt="图片"
        draggable={false}
        className={zoom > 1 ? "zoomed" : ""}
        style={{ transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})` }}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => {
          e.stopPropagation();
          setZoom((z) => (z > 1 ? 1 : 2));
        }}
        onPointerDown={(e) => {
          if (zoom > 1) {
            e.stopPropagation();
            drag.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
          }
        }}
      />
      <div className="lightboxHint">滚轮缩放 · 拖动平移 · 双击复位 · Esc/点背景关闭</div>
    </div>
  );
}

/* ===================== 设置窗口 ===================== */

function SettingsPage() {
  const [prefs, setPrefs] = React.useState<Prefs>(initialPrefs);
  const [cat, setCat] = React.useState("appearance");
  const saveTimer = React.useRef<number | undefined>(undefined);

  useApplyAppearance(prefs);

  React.useEffect(() => {
    (async () => {
      try {
        setPrefs(await command<Prefs>("get_prefs"));
      } catch {
        /* ignore */
      }
    })();
  }, []);

  const update = React.useCallback((patch: Partial<Prefs>) => {
    setPrefs((prev) => {
      const next = { ...prev, ...patch };
      window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        command("set_prefs", {
          theme: next.theme,
          fontFace: next.font_face,
          fontSize: next.font_size,
          glass: next.glass || "off",
          cornerRadius: next.corner_radius || "0,0,0,0",
          filterRules: next.filter_rules ?? "",
          retentionDays: Number(next.retention_days) || 0,
          mode: next.dock_mode,
          side: next.attach_side,
          width: next.dock_width,
          height: next.dock_height,
          follow: next.follow
        }).catch(() => undefined);
      }, 200);
      return next;
    });
  }, []);

  const isScreen = prefs.dock_mode === "screen";
  const widthAuto = !isScreen && prefs.follow && (prefs.attach_side === "top" || prefs.attach_side === "bottom");
  const heightAuto = !isScreen && prefs.follow && (prefs.attach_side === "left" || prefs.attach_side === "right");

  // 圆角四角: [左上, 右上, 左下, 右下]
  const radii = (prefs.corner_radius || "0,0,0,0").split(",").map((n) => Number(n) || 0);
  const setRadius = (index: number, value: number) => {
    const next = [radii[0] || 0, radii[1] || 0, radii[2] || 0, radii[3] || 0];
    next[index] = Math.max(0, Math.min(40, value || 0));
    update({ corner_radius: next.join(",") });
  };
  const cornerInput = (index: number) => (
    <span className="numField">
      <input
        type="number"
        min={0}
        max={40}
        value={radii[index] || 0}
        onChange={(e) => setRadius(index, Number(e.target.value))}
      />
    </span>
  );

  return (
    <div className="prefs">
      <aside className="prefsSidebar">
        <button className={cat === "appearance" ? "on" : ""} type="button" onClick={() => setCat("appearance")}>
          <Palette size={16} /> <span>外观</span>
        </button>
        <button className={cat === "font" ? "on" : ""} type="button" onClick={() => setCat("font")}>
          <Type size={16} /> <span>字体</span>
        </button>
        <button className={cat === "layout" ? "on" : ""} type="button" onClick={() => setCat("layout")}>
          <PanelRight size={16} /> <span>贴靠</span>
        </button>
        <button className={cat === "filter" ? "on" : ""} type="button" onClick={() => setCat("filter")}>
          <Filter size={16} /> <span>过滤</span>
        </button>
        <button className={cat === "about" ? "on" : ""} type="button" onClick={() => setCat("about")}>
          <Info size={16} /> <span>关于</span>
        </button>
      </aside>

      <div className="prefsContent">
        {cat === "appearance" && (
          <>
            <div className="prefsGroupTitle">主题</div>
            <div className="prefsCard">
              <div className="swatches">
                {THEMES.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    className={`swatch ${prefs.theme === t.id ? "on" : ""}`}
                    onClick={() => update({ theme: t.id })}
                  >
                    <span className="dotpair">
                      <span className="d" style={{ background: t.bg, border: "1px solid rgba(125,125,135,.4)" }} />
                      <span className="d" style={{ background: t.a }} />
                      <span className="d" style={{ background: t.b }} />
                    </span>
                    {t.name}
                  </button>
                ))}
              </div>
            </div>
            <div className="prefsGroupTitle" style={{ marginTop: 18 }}>玻璃（macOS）</div>
            <div className="prefsCard">
              <div className="prefsRow">
                <span>玻璃质感</span>
                <Segmented value={prefs.glass || "off"} onChange={(v) => update({ glass: v })} options={[["off", "关"], ["frosted", "经典磨砂"], ["liquid", "液态风"]]} />
              </div>
            </div>
            <div className="prefsGroupTitle" style={{ marginTop: 18 }}>圆角（px，0=直角）</div>
            <div className="prefsCard">
              <div className="prefsRow">
                <span>左上 / 右上</span>
                <span className="cornerPair">{cornerInput(0)}{cornerInput(1)}</span>
              </div>
              <div className="prefsRow">
                <span>左下 / 右下</span>
                <span className="cornerPair">{cornerInput(2)}{cornerInput(3)}</span>
              </div>
            </div>
          </>
        )}

        {cat === "font" && (
          <>
            <div className="prefsGroupTitle">字体</div>
            <div className="prefsCard">
              <div className="prefsRow">
                <span>字体</span>
                <Segmented value={prefs.font_face} onChange={(v) => update({ font_face: v })} options={[["system", "系统"], ["mono", "等宽"], ["rounded", "圆体"]]} />
              </div>
              <div className="prefsRow">
                <span>字号</span>
                <Segmented value={prefs.font_size} onChange={(v) => update({ font_size: v })} options={[["small", "小"], ["medium", "中"], ["large", "大"]]} />
              </div>
            </div>
          </>
        )}

        {cat === "layout" && (
          <>
            <div className="prefsGroupTitle">贴靠</div>
            <div className="prefsCard">
              <div className="prefsRow">
                <span>模式</span>
                <Segmented value={prefs.dock_mode} onChange={(v) => update({ dock_mode: v })} options={[["terminal", "跟随终端"], ["screen", "固定屏幕边"]]} />
              </div>
              <div className="prefsRow">
                <span>{isScreen ? "屏幕边" : "方向"}</span>
                <Segmented value={prefs.attach_side} onChange={(v) => update({ attach_side: v })} options={[["top", "上"], ["bottom", "下"], ["left", "左"], ["right", "右"]]} />
              </div>
              {!isScreen && (
                <div className="prefsRow">
                  <span>跟随终端尺寸</span>
                  <Segmented value={prefs.follow ? "on" : "off"} onChange={(v) => update({ follow: v === "on" })} options={[["on", "开"], ["off", "关"]]} />
                </div>
              )}
              <div className="prefsRow">
                <span>宽度</span>
                {widthAuto ? (
                  <span className="autoTag">跟随终端</span>
                ) : (
                  <span className="numField">
                    <input
                      type="number"
                      min={180}
                      max={900}
                      value={prefs.dock_width}
                      onChange={(e) => update({ dock_width: Number(e.target.value) })}
                      onBlur={() => update({ dock_width: Math.max(180, Math.min(900, prefs.dock_width || 300)) })}
                    />
                    <i>px</i>
                  </span>
                )}
              </div>
              <div className="prefsRow">
                <span>高度</span>
                {heightAuto ? (
                  <span className="autoTag">跟随终端</span>
                ) : (
                  <span className="numField">
                    <input
                      type="number"
                      min={120}
                      max={1600}
                      value={prefs.dock_height}
                      onChange={(e) => update({ dock_height: Number(e.target.value) })}
                      onBlur={() => update({ dock_height: Math.max(120, Math.min(1600, prefs.dock_height || 420)) })}
                    />
                    <i>px</i>
                  </span>
                )}
              </div>
            </div>
          </>
        )}

        {cat === "filter" && (
          <>
            <div className="prefsGroupTitle">过滤规则</div>
            <div className="prefsHint">每行一个词。抓到的提问如果整条正好等于某行（忽略大小写和末尾标点），就不收进列表。常用于「继续」「你好」这类只是催状态的短消息。</div>
            <textarea
              className="filterArea"
              value={prefs.filter_rules ?? ""}
              spellCheck={false}
              placeholder={"继续\n你好\nok"}
              onChange={(e) => update({ filter_rules: e.target.value })}
            />

            <div className="prefsGroupTitle" style={{ marginTop: 18 }}>缓存清理</div>
            <div className="prefsHint">超过保留天数的会话记录和图片会自动清理。0 = 永久保留。</div>
            <div className="prefsCard">
              <div className="prefsRow">
                <span>保留天数</span>
                <span className="numField">
                  <input
                    type="number"
                    min={0}
                    max={365}
                    value={prefs.retention_days}
                    onChange={(e) => update({ retention_days: Number(e.target.value) })}
                    onBlur={() => update({ retention_days: Math.max(0, Math.min(365, Number(prefs.retention_days) || 0)) })}
                  />
                  <i>天</i>
                </span>
              </div>
            </div>
          </>
        )}

        {cat === "about" && (
          <>
            <div className="prefsGroupTitle">关于</div>
            <div className="prefsCard aboutBox">
              <div className="aboutTitle">
                <strong>AskDock</strong>
                <span className="ver">v0.1.0</span>
              </div>
              <p>自动收集你在终端里问 AI 的提问，按终端窗口归类，切回窗口就能看到当时在问什么。</p>
              <p className="aboutMuted">提问来源：Claude Code、Codex 的会话记录</p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function Root() {
  return isSettingsWindow() ? <SettingsPage /> : <Dock />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>
);
