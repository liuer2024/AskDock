import React from "react";
import ReactDOM from "react-dom/client";
import { Cat, Check, Copy, Filter, Folder, Info, Palette, PanelRight, PanelRightClose, PanelRightOpen, Settings, Type, X, Zap } from "lucide-react";
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
  group_drawn_tabs: boolean;
  group_native_windows: boolean;
  notch_expand: string;
  notch_mascot: string;
};

// 一只边缘小人（Codex Pets 格式：pet.json + 精灵图）。
type PetInfo = {
  id: string;
  display_name: string;
  description: string;
  spritesheet: string; // 绝对路径（Tauri 下用 convertFileSrc）或 dev 预览的 /sprout.png
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
  follow: true,
  group_drawn_tabs: true,
  group_native_windows: false,
  notch_expand: "hover",
  notch_mascot: "sprout"
};

// 边缘小人精灵图的通用规格（Codex Pets 标准）：8 列 × 9 行，每格 192×208，透明。
// 9 行 = 9 个动画状态，帧数与逐帧时序固定，社区现成宠物都遵循这套。
const PET_CELL_W = 192;
const PET_CELL_H = 208;
const PET_COLS = 8;
type PetRow = { name: string; index: number; frames: number; timings: number[] };
const PET_ROWS: PetRow[] = [
  { name: "idle", index: 0, frames: 6, timings: [280, 110, 110, 140, 140, 320] },
  { name: "running-right", index: 1, frames: 8, timings: [120, 120, 120, 120, 120, 120, 120, 220] },
  { name: "running-left", index: 2, frames: 8, timings: [120, 120, 120, 120, 120, 120, 120, 220] },
  { name: "waving", index: 3, frames: 4, timings: [140, 140, 140, 280] },
  { name: "jumping", index: 4, frames: 5, timings: [140, 140, 140, 140, 280] },
  { name: "failed", index: 5, frames: 8, timings: [140, 140, 140, 140, 140, 140, 140, 240] },
  { name: "waiting", index: 6, frames: 6, timings: [150, 150, 150, 150, 150, 260] },
  { name: "running", index: 7, frames: 6, timings: [120, 120, 120, 120, 120, 220] },
  { name: "review", index: 8, frames: 6, timings: [150, 150, 150, 150, 150, 280] }
];
const PET_ROW_BY_NAME: Record<string, PetRow> = Object.fromEntries(PET_ROWS.map((r) => [r.name, r]));
const PET_ROW_COUNT = PET_ROWS.length;
const PET_DISPLAY_H = 124; // 把手里精灵的显示高度（px）；宽≈114，恰好放进 120 宽的把手窗口

// 精灵图 URL：Tauri 下走 asset 协议（路径在 $APPDATA/**，已在白名单），dev 直接用原路径。
function petSheetSrc(path: string) {
  return isTauriRuntime() ? convertFileSrc(path) : path;
}

// 把用户粘的宠物链接规整成 raw 的 pet.json URL；裸 GitHub 仓库补 main 分支（再回退 master）。
function normalizePetUrl(input: string): { petJsonUrl: string; tryMaster: boolean } {
  let u = input.trim();
  if (u.includes("github.com") && !u.includes("raw.githubusercontent.com")) {
    u = u.replace("github.com", "raw.githubusercontent.com").replace("/blob/", "/").replace("/tree/", "/");
  }
  const rawPrefix = "raw.githubusercontent.com/";
  if (u.includes(rawPrefix)) {
    const segs = (u.split(rawPrefix)[1] || "").split("/").filter(Boolean);
    if (segs.length === 2) u = `https://raw.githubusercontent.com/${segs[0]}/${segs[1]}/main/`; // 裸仓库 → 补分支
  }
  const petJsonUrl = /\.json($|\?)/.test(u) ? u : (u.endsWith("/") ? u : u + "/") + "pet.json";
  return { petJsonUrl, tryMaster: petJsonUrl.includes("/main/") };
}

function arrayBufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  return btoa(binary);
}

// 一只会动的小人：按所选动画行(state)用逐帧时序循环切帧（比纯 CSS steps() 更忠实）。
function PetSprite({ sheet, state, displayH = PET_DISPLAY_H, animate = true }: { sheet: string; state: string; displayH?: number; animate?: boolean }) {
  const row = PET_ROW_BY_NAME[state] ?? PET_ROW_BY_NAME.idle;
  const [frame, setFrame] = React.useState(0);
  React.useEffect(() => {
    setFrame(0);
    if (!animate) return; // 静态：只显示第 0 帧（设置选择器里几十个缩略图不逐帧动画，省 CPU）
    let i = 0;
    let timer: number | undefined;
    const step = () => {
      const wait = row.timings[i] ?? 140;
      timer = window.setTimeout(() => {
        i = (i + 1) % row.frames;
        setFrame(i);
        step();
      }, wait);
    };
    step();
    return () => window.clearTimeout(timer);
  }, [row, animate]);
  const scale = displayH / PET_CELL_H;
  const w = PET_CELL_W * scale;
  const h = PET_CELL_H * scale;
  return (
    <div
      className="petSprite"
      style={{
        width: `${w}px`,
        height: `${h}px`,
        backgroundImage: `url(${petSheetSrc(sheet)})`,
        backgroundSize: `${PET_COLS * w}px ${PET_ROW_COUNT * h}px`,
        backgroundPosition: `${-frame * w}px ${-row.index * h}px`
      }}
    />
  );
}

const THEMES: { id: string; name: string; bg: string; a: string; b: string }[] = [
  { id: "midnight", name: "午夜", bg: "#0e1014", a: "#3fcf8e", b: "#d97757" },
  { id: "graphite", name: "石墨", bg: "#161618", a: "#57d4a1", b: "#e08a63" },
  { id: "ember", name: "暖炭", bg: "#150f0b", a: "#5fcf8e", b: "#ff8c5a" },
  { id: "indigo", name: "靛蓝", bg: "#13152b", a: "#5be0a8", b: "#ff9b6b" },
  { id: "linen", name: "亚麻", bg: "#f4f0e7", a: "#18895a", b: "#bf552f" },
  { id: "slate", name: "石板", bg: "#f1f3f6", a: "#0f9b69", b: "#c2562f" }
];

// 原生 tab 终端：每个 tab 是独立窗口（独立 CGWindowID），一个 window_id 里的多 cwd
// 是同一 tab 内 cd 出来的。其余（iTerm2/Otty/Warp…）是自绘 tab，多 tab 挤一个 window_id。
const NATIVE_TAB_TERMINALS = ["Ghostty", "Terminal", "Apple Terminal"];

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
  if (name === "purge_cache_now") {
    return { rows: 0, images: 0 } as T;
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
  if (name === "list_pets") {
    return [
      { id: "sprout", display_name: "小芽", description: "AskDock 内置的小芽，扒在屏幕边看着你。", spritesheet: "/sprout.png" }
    ] as T;
  }
  if (name === "install_pet") {
    const a = args as { id?: string; displayName?: string; description?: string };
    return { id: a.id ?? "pet", display_name: a.displayName ?? "pet", description: a.description ?? "", spritesheet: "/sprout.png" } as T;
  }
  if (
    name === "hide_main_window" ||
    name === "set_prefs" ||
    name === "dock_collapse" ||
    name === "open_pets_dir"
  ) {
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
  const mk = (id: string, source: string, text: string, ms: number, cwd: string | null = null): CapturedQuestion => ({
    id, window_id: BROWSER_WINDOW_ID, source, session_id: null, cwd, text, asked_at: new Date(now - ms).toISOString(), image_path: null
  });
  // 两个项目目录 → 演示自绘 tab 终端多 tab 的按项目分组
  const A = "/Users/me/Documents/askDock";
  const B = "/Users/me/Documents/peixun2024";
  const sample: CapturedQuestion[] = [
    mk("1", "codex", "你开一个新的分支，将这些问题，用去控件化处理一下。", 2 * 60000, A),
    mk("2", "codex", "现在的元素都包了边框、底色、阴影、强调色块，控件感太强，于是显得“重”。看看是不是有这个问题？", 6 * 60000, A),
    mk("3", "claude", "把时间戳和来源都换成等宽字体，整体节奏会更像终端日志，也更耐看。", 90 * 60000, A),
    mk("4", "claude", "学员报名后的短信通知，加一个失败重试。", 110 * 60000, B),
    mk("5", "codex", "课程表导出 Excel，表头合并单元格。", 26 * 3600000, B),
    mk("6", "claude", "内容直接占满整行，不要卡片。让文字本身成为主角，留白来分隔，而不是边框。", 30 * 3600000, A)
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

// 项目分组标题：取工作目录最后一段（cwd 为空 → 未归类）
function projectLabel(cwd: string | null) {
  if (!cwd) return "未归类";
  const parts = cwd.split("/").filter(Boolean);
  return parts[parts.length - 1] || cwd;
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
  const [collapsed, setCollapsed] = React.useState(false); // 刘海式收起到屏幕右缘
  const [peeking, setPeeking] = React.useState(false); // 收起后悬停滑出中
  const [pets, setPets] = React.useState<PetInfo[]>([]); // 已安装的边缘小人
  const [greet, setGreet] = React.useState(false); // 抓到新提问时小人挥手一下
  const [petDragged, setPetDragged] = React.useState(false); // 小人是否被拖到桌面任意位置（桌面宠物模式）
  const [petMenu, setPetMenu] = React.useState(false); // 右键小人弹出的浮层菜单
  const [petAction, setPetAction] = React.useState("idle"); // 小人当前动画（空闲时随机来点小动作）

  const windowIdRef = React.useRef<string | null>(null);
  windowIdRef.current = windowId;
  const freeModeRef = React.useRef(false);
  freeModeRef.current = freeMode;
  const collapsedRef = React.useRef(false);
  collapsedRef.current = collapsed;
  const peekTimer = React.useRef<number | undefined>(undefined);
  const greetTimer = React.useRef<number | undefined>(undefined);
  const holdingTitle = React.useRef(false);
  const petDraggedRef = React.useRef(false);
  petDraggedRef.current = petDragged;
  const holdingPet = React.useRef(false); // 正在按住小人（可能要拖动）
  const petDragMoved = React.useRef(false); // 本次手势是否真的拖动了
  const petStart = React.useRef({ x: 0, y: 0 });

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

  // 已安装小人列表：开窗加载一次，prefs 变化（含新导入/换选）后再刷新。
  React.useEffect(() => {
    const loadPets = async () => {
      try {
        setPets(await command<PetInfo[]>("list_pets"));
      } catch {
        /* ignore */
      }
    };
    loadPets();
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    listen("prefs-changed", loadPets).then((fn) => {
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
        const status = await command<FrontStatus>("poll_front_window", { reposition: !freeModeRef.current && !collapsedRef.current });
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
    listen("questions-updated", () => {
      refresh(windowIdRef.current);
      // 收起状态下抓到新提问 → 小人挥手一下提醒你。
      if (collapsedRef.current) {
        setGreet(true);
        window.clearTimeout(greetTimer.current);
        greetTimer.current = window.setTimeout(() => setGreet(false), 2600);
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [refresh]);

  // 拖动标题栏移动后进入“自由位置”，停止贴靠回弹。
  React.useEffect(() => {
    if (!isTauriRuntime()) return;
    const onUp = () => {
      holdingTitle.current = false;
      holdingPet.current = false;
    };
    window.addEventListener("pointerup", onUp);
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onMoved(() => {
        if (holdingTitle.current) setFreeMode(true);
        // 拖动小人 → 进入桌面宠物模式（自由位置、完整显示、点击就地展开）。
        if (holdingPet.current) {
          petDragMoved.current = true;
          setPetDragged(true);
          setFreeMode(true);
        }
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

  // ===== 刘海式收起 =====
  const enterCollapse = React.useCallback(() => {
    window.clearTimeout(peekTimer.current);
    setCollapsed(true);
    setPeeking(false);
    // 桌面宠物模式：在当前位置缩回小人；否则收成屏幕右缘的把手。
    const state = petDraggedRef.current ? "pet-collapse" : "handle";
    command("dock_collapse", { state }).catch(() => undefined);
  }, []);

  const exitCollapse = React.useCallback(() => {
    window.clearTimeout(peekTimer.current);
    setCollapsed(false);
    setPeeking(false);
    setPetDragged(false); // 退出收起＝回到正常贴靠，离开桌面宠物模式
    command("dock_collapse", { state: "normal" }).catch(() => undefined);
  }, []);

  // 桌面宠物：点击小人在当前位置就地展开完整浮窗（仍是自由位置，不跳回贴终端）。
  const petExpand = React.useCallback(() => {
    window.clearTimeout(peekTimer.current);
    setCollapsed(false);
    setPeeking(false);
    command("dock_collapse", { state: "pet-expand" }).catch(() => undefined);
  }, []);

  // 桌面宠物右键菜单的「去一边」：回到屏幕右缘、探出半身的小人。
  const goEdge = React.useCallback(() => {
    window.clearTimeout(peekTimer.current);
    setPetDragged(false);
    setCollapsed(true);
    setPeeking(false);
    command("dock_collapse", { state: "handle" }).catch(() => undefined);
  }, []);

  const peekStart = React.useCallback(() => {
    if (!collapsedRef.current) return;
    window.clearTimeout(peekTimer.current);
    setPeeking(true);
    command("dock_collapse", { state: "peek" }).catch(() => undefined);
  }, []);

  const peekEnd = React.useCallback(() => {
    if (!collapsedRef.current) return;
    // 留一点延迟，避免缩放越界时鼠标瞬时离开导致闪烁。
    window.clearTimeout(peekTimer.current);
    peekTimer.current = window.setTimeout(() => {
      setPeeking(false);
      command("dock_collapse", { state: "handle" }).catch(() => undefined);
    }, 180);
  }, []);

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

  // 时间线最新一条会跳动（光标感）
  const newestId = questions[0]?.id;
  const renderEntry = (q: CapturedQuestion) => (
    <article key={q.id} className={`entry ${q.id === newestId ? "latest" : ""}`} data-src={q.source}>
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
  );

  // 按天分组（单项目时用）：插入日期分隔
  let prevDay = "";
  const rows = questions.map((q) => {
    const day = dayLabel(q.asked_at);
    const showMark = day !== prevDay;
    prevDay = day;
    return { q, day, showMark };
  });

  // 同一窗口里出现多个项目目录（常见于 iTerm2/Otty 等自绘 tab 终端的多 tab）→ 按项目分组。
  // questions 已按 asked_at 倒序，组按"组内最新一条"先后排列（最近用的项目在上）。
  const groups: { key: string; cwd: string | null; items: CapturedQuestion[] }[] = [];
  for (const q of questions) {
    const key = q.cwd ?? "";
    let g = groups.find((x) => x.key === key);
    if (!g) {
      g = { key, cwd: q.cwd, items: [] };
      groups.push(g);
    }
    g.items.push(q);
  }
  // 当前窗口属于原生 tab 终端（Ghostty/Terminal）还是自绘 tab 终端（iTerm2/Otty…），
  // 分别由对应开关决定是否按项目分组。
  const isNativeTab = !!appName && NATIVE_TAB_TERMINALS.includes(appName);
  const groupingOn = isNativeTab ? prefs.group_native_windows : prefs.group_drawn_tabs;
  const multiProject = groupingOn && groups.length > 1;

  const clickExpand = prefs.notch_expand === "click";
  // 当前选中的边缘小人（off 或找不到 → 退回细握把）。
  const currentPet =
    prefs.notch_mascot && prefs.notch_mascot !== "off"
      ? pets.find((p) => p.id === prefs.notch_mascot) ?? null
      : null;

  // 收起并显示小人时，连 html/body 一起透明（原生窗口已透明，但根元素背景是主题色会盖住）。
  React.useEffect(() => {
    const root = document.documentElement;
    if (collapsed && !peeking && currentPet) root.setAttribute("data-petnotch", "1");
    else root.removeAttribute("data-petnotch");
    return () => root.removeAttribute("data-petnotch");
  }, [collapsed, peeking, currentPet]);

  // 空闲小动作：小人显示时，每隔一会儿随机蹦一下/招手/张望，播完回到待机。
  React.useEffect(() => {
    if (!collapsed || !currentPet) return;
    let alive = true;
    let timer: number | undefined;
    // 桌面完整态：蹦/招手/张望全有；右缘半身态只侧脸招手（露脸自然，不会露半张正脸）。
    const acts = petDragged ? ["jumping", "waving", "waiting"] : ["waving"];
    const dur: Record<string, number> = { jumping: 900, waving: 1300, waiting: 1100 };
    const loop = () => {
      timer = window.setTimeout(() => {
        if (!alive) return;
        const a = acts[Math.floor(Math.random() * acts.length)];
        setPetAction(a);
        timer = window.setTimeout(() => {
          if (!alive) return;
          setPetAction("idle");
          loop();
        }, dur[a] ?? 1000);
      }, 7000 + Math.random() * 9000);
    };
    loop();
    return () => {
      alive = false;
      window.clearTimeout(timer);
      setPetAction("idle");
    };
  }, [collapsed, currentPet, petDragged]);

  // 收起且未展开：屏幕右缘显示小人/把手。
  // 细握把：沿用 notch_expand（悬停滑出 / 点击展开）。
  // 小人：统一为「按住拖动可移到桌面任意处 · 点击展开」（hover 滑出会和拖动冲突，故不用）。
  if (collapsed && !peeking) {
    const petFree = !!currentPet && petDragged; // 已拖到桌面＝完整显示、点击就地展开
    const onPetDown = (e: React.PointerEvent) => {
      holdingPet.current = true;
      petDragMoved.current = false;
      petStart.current = { x: e.screenX, y: e.screenY };
    };
    const onPetMove = (e: React.PointerEvent) => {
      if (!holdingPet.current || petDragMoved.current) return;
      if (Math.abs(e.screenX - petStart.current.x) > 4 || Math.abs(e.screenY - petStart.current.y) > 4) {
        petDragMoved.current = true; // 超过阈值＝拖动，启动系统拖窗
        if (isTauriRuntime()) getCurrentWindow().startDragging().catch(() => undefined);
      }
    };
    const onPetClick = () => {
      if (petDragMoved.current) { petDragMoved.current = false; return; } // 刚拖动，不当点击
      if (petFree) petExpand(); // 桌面：就地展开
      else exitCollapse(); // 右缘：还原成正常贴靠的完整 dock
    };
    // 关闭菜单并把临时增高的窗口缩回小人。
    const closeMenu = () => {
      setPetMenu(false);
      command("dock_collapse", { state: "pet-menu-end" }).catch(() => undefined);
    };
    return (
      <div
        className={`notchRoot ${petMenu ? "menuing" : ""}`}
        onMouseEnter={!currentPet && !clickExpand ? peekStart : undefined}
        onClick={!currentPet && clickExpand ? exitCollapse : undefined}
        title={currentPet ? "拖动可移到桌面 · 点击展开 · 右键更多" : clickExpand ? "点击展开 AskDock" : "移上来展开 AskDock"}
      >
        {currentPet ? (
          <>
            <div
              className={`petStage ${petFree ? "free" : ""}`}
              data-greet={greet ? "1" : undefined}
              onPointerDown={onPetDown}
              onPointerMove={onPetMove}
              onClick={onPetClick}
              onContextMenu={(e) => {
                e.preventDefault();
                setPetMenu(true);
                command("dock_collapse", { state: "pet-menu" }).catch(() => undefined);
              }}
            >
              <PetSprite sheet={currentPet.spritesheet} state={greet ? "waving" : petAction} />
            </div>
            {petMenu ? (
              <div className="petMenu" onMouseLeave={closeMenu}>
                {/* 去/展开/贴靠 三项自带尺寸切换，不必再 pet-menu-end */}
                <button type="button" onClick={() => { setPetMenu(false); goEdge(); }}>去屏幕右缘</button>
                <button type="button" onClick={() => { setPetMenu(false); petExpand(); }}>展开列表</button>
                <button type="button" onClick={() => { setPetMenu(false); exitCollapse(); }}>贴靠终端</button>
                <div className="petMenuSep" />
                <button type="button" onClick={() => { closeMenu(); command("open_settings").catch(() => undefined); }}>设置…</button>
              </div>
            ) : null}
          </>
        ) : (
          <div className="notchHandle"><span className="grip" /></div>
        )}
      </div>
    );
  }

  return (
    <main
      className="dock"
      onMouseEnter={collapsed && !clickExpand ? peekStart : undefined}
      onMouseLeave={collapsed && !clickExpand ? peekEnd : undefined}
    >
      <header className="titlebar" data-tauri-drag-region onPointerDown={() => { holdingTitle.current = true; }}>
        <span className="brand">AskDock</span>
        <button
          className={`ic ${freeMode ? "" : "live"}`}
          type="button"
          onClick={() => setFreeMode((v) => { const nv = !v; if (!nv) setPetDragged(false); return nv; })}
          title={freeMode ? "自由位置 · 点击重新贴靠终端" : "实时贴靠中 · 点击解锁后可拖动标题栏移动"}
        >
          <Zap size={15} />
        </button>
        {isTauriRuntime() ? (
          <button
            className="ic"
            type="button"
            onClick={collapsed ? exitCollapse : enterCollapse}
            title={collapsed ? "退出收起" : "收起到屏幕右缘（移上来展开）"}
          >
            {collapsed ? <PanelRightOpen size={15} /> : <PanelRightClose size={15} />}
          </button>
        ) : null}
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
        ) : multiProject ? (
          groups.map((g) => (
            <section className="projGroup" key={g.key}>
              <div className="projmark">
                <Folder size={11} />
                <span title={g.cwd ?? undefined}>{projectLabel(g.cwd)}</span>
                <div className="rule" />
              </div>
              {g.items.map((q) => renderEntry(q))}
            </section>
          ))
        ) : (
          rows.map(({ q, day, showMark }) => (
            <React.Fragment key={q.id}>
              {showMark ? (
                <div className="daymark"><span>{day}</span><div className="rule" /></div>
              ) : null}
              {renderEntry(q)}
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
  const [purging, setPurging] = React.useState(false);
  const [purgeMsg, setPurgeMsg] = React.useState("");
  const [confirmPurge, setConfirmPurge] = React.useState(false);
  const [pets, setPets] = React.useState<PetInfo[]>([]);
  const [petUrl, setPetUrl] = React.useState("");
  const [installing, setInstalling] = React.useState(false);
  const [installMsg, setInstallMsg] = React.useState("");
  const saveTimer = React.useRef<number | undefined>(undefined);

  useApplyAppearance(prefs);

  const loadPets = React.useCallback(async () => {
    try {
      setPets(await command<PetInfo[]>("list_pets"));
    } catch {
      /* ignore */
    }
  }, []);
  React.useEffect(() => {
    loadPets();
  }, [loadPets]);

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
          follow: next.follow,
          groupDrawnTabs: next.group_drawn_tabs,
          groupNativeWindows: next.group_native_windows,
          notchExpand: next.notch_expand,
          notchMascot: next.notch_mascot
        }).catch(() => undefined);
      }, 200);
      return next;
    });
  }, []);

  // 一键安装：从链接 fetch pet.json + 精灵图（webview 下载，绕过 Rust HTTP），交后端落盘。
  const installPet = React.useCallback(async () => {
    const raw = petUrl.trim();
    if (!raw || installing) return;
    setInstalling(true);
    setInstallMsg("下载中…");
    try {
      const { petJsonUrl, tryMaster } = normalizePetUrl(raw);
      const candidates = tryMaster ? [petJsonUrl, petJsonUrl.replace("/main/", "/master/")] : [petJsonUrl];
      let json: Record<string, unknown> | null = null;
      let baseUrl = "";
      for (const url of candidates) {
        try {
          const r = await fetch(url);
          if (r.ok) {
            json = await r.json();
            baseUrl = url.replace(/[^/]*$/, "");
            break;
          }
        } catch {
          /* try next */
        }
      }
      if (!json) throw new Error("找不到 pet.json（链接要指向含 pet.json 的目录或文件）");
      const spritePath = String(json.spritesheetPath || "spritesheet.webp");
      const sr = await fetch(new URL(spritePath, baseUrl).toString());
      if (!sr.ok) throw new Error("精灵图下载失败");
      const b64 = arrayBufferToBase64(await sr.arrayBuffer());
      const ext = (spritePath.split(".").pop() || "webp").toLowerCase().replace(/[^a-z0-9]/g, "");
      const id = String(json.id || raw.split(/[/?#]/).filter(Boolean).pop() || "pet");
      const info = await command<PetInfo>("install_pet", {
        id,
        displayName: String(json.displayName || id),
        description: String(json.description || ""),
        spriteB64: b64,
        ext
      });
      await loadPets();
      update({ notch_mascot: info.id });
      setInstallMsg(`已安装并选用：${info.display_name}`);
      setPetUrl("");
    } catch (e) {
      setInstallMsg("安装失败：" + (e instanceof Error ? e.message : String(e)));
    } finally {
      setInstalling(false);
    }
  }, [petUrl, installing, loadPets, update]);

  const runPurge = React.useCallback(async () => {
    setConfirmPurge(false);
    setPurging(true);
    setPurgeMsg("");
    try {
      const stats = await command<{ rows: number; images: number }>("purge_cache_now");
      setPurgeMsg(
        stats.rows || stats.images
          ? `已清理 ${stats.rows} 条记录、${stats.images} 张图片`
          : "没有可清理的内容"
      );
    } catch {
      setPurgeMsg("清理失败，请重试");
    } finally {
      setPurging(false);
    }
  }, []);

  React.useEffect(() => {
    if (!confirmPurge) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setConfirmPurge(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirmPurge]);

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
        <button className={cat === "pet" ? "on" : ""} type="button" onClick={() => setCat("pet")}>
          <Cat size={16} /> <span>小人</span>
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

            <div className="prefsGroupTitle" style={{ marginTop: 18 }}>按项目分组</div>
            <div className="prefsHint">同一窗口里出现多个工作目录时，把提问按项目分组显示。</div>
            <div className="prefsCard">
              <div className="prefsRow">
                <span>自绘 tab 终端<br /><i className="rowSub">iTerm2 / Otty / Warp 等，多 tab 挤一个窗口</i></span>
                <Segmented value={prefs.group_drawn_tabs ? "on" : "off"} onChange={(v) => update({ group_drawn_tabs: v === "on" })} options={[["on", "开"], ["off", "关"]]} />
              </div>
              <div className="prefsRow">
                <span>原生 tab 终端<br /><i className="rowSub">Ghostty / 终端，每 tab 即独立窗口</i></span>
                <Segmented value={prefs.group_native_windows ? "on" : "off"} onChange={(v) => update({ group_native_windows: v === "on" })} options={[["on", "开"], ["off", "关"]]} />
              </div>
            </div>
          </>
        )}

        {cat === "pet" && (
          <>
            <div className="prefsGroupTitle">收起到边缘</div>
            <div className="prefsHint">点标题栏的收起按钮把浮窗收成屏幕右缘的小把手，收起后这样展开：</div>
            <div className="prefsCard petCard">
              <div className="prefsRow">
                <span>展开方式<br /><i className="rowSub">悬停：移上去临时滑出、移开收回；点击：点一下展开成正常浮窗</i></span>
                <Segmented value={prefs.notch_expand === "click" ? "click" : "hover"} onChange={(v) => update({ notch_expand: v })} options={[["hover", "悬停"], ["click", "点击"]]} />
              </div>
              <div className="prefsRow petRow">
                <span>边缘小人<br /><i className="rowSub">收起时扒在屏幕右缘看着你，抓到新提问会挥手</i></span>
              </div>
              <div className="petPicker">
                <button
                  type="button"
                  className={`petChoice ${prefs.notch_mascot === "off" ? "on" : ""}`}
                  onClick={() => update({ notch_mascot: "off" })}
                  title="不要小人，只显示一根细握把"
                >
                  <span className="petChoiceArt"><span className="grip" /></span>
                  <span className="petChoiceName">细握把</span>
                </button>
                {pets.map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className={`petChoice ${prefs.notch_mascot === p.id ? "on" : ""}`}
                    onClick={() => update({ notch_mascot: p.id })}
                    title={p.description || p.display_name}
                  >
                    <span className="petChoiceArt">
                      <PetSprite sheet={p.spritesheet} state="idle" displayH={66} animate={false} />
                    </span>
                    <span className="petChoiceName">{p.display_name}</span>
                  </button>
                ))}
              </div>
              <div className="petActions">
                <button type="button" className="petBtn" onClick={() => command("open_pets_dir").catch(() => undefined)}>
                  <Folder size={13} /> 打开宠物文件夹
                </button>
                <button type="button" className="petBtn ghost" onClick={loadPets}>刷新</button>
              </div>
              <div className="petInstall">
                <input
                  type="text"
                  value={petUrl}
                  placeholder="粘贴宠物链接（GitHub 仓库/目录/pet.json）"
                  onChange={(e) => setPetUrl(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") installPet(); }}
                />
                <button type="button" className="petBtn" disabled={installing || !petUrl.trim()} onClick={installPet}>
                  {installing ? "安装中…" : "一键安装"}
                </button>
              </div>
              {installMsg ? <div className="prefsHint petInstallMsg">{installMsg}</div> : null}
              <div className="prefsHint petGalleryHint">
                沿用 Codex Pets 格式：把社区做好的小人（一个含 <code>pet.json</code> + 精灵图的文件夹）拖进该目录，点刷新即可选用。
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
              <div className="prefsRow">
                <span>手动清理</span>
                <span className="purgeAction">
                  {purgeMsg && <i className="purgeMsg">{purgeMsg}</i>}
                  <button type="button" className="prefsBtn" onClick={() => setConfirmPurge(true)} disabled={purging}>
                    {purging ? "清理中…" : "立即清理"}
                  </button>
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

      {confirmPurge && (
        <div className="confirmOverlay" onClick={() => setConfirmPurge(false)}>
          <div className="confirmCard" onClick={(e) => e.stopPropagation()}>
            <div className="confirmTitle">确认清理</div>
            <p className="confirmBody">
              {(Number(prefs.retention_days) || 0) > 0
                ? `将删除 ${Number(prefs.retention_days)} 天前的会话记录，并清理无引用的图片。此操作不可撤销。`
                : "保留天数为 0（永久保留），将仅清理无引用的图片。此操作不可撤销。"}
            </p>
            <div className="confirmActions">
              <button type="button" className="prefsBtn" onClick={() => setConfirmPurge(false)}>
                取消
              </button>
              <button type="button" className="prefsBtn danger" onClick={runPurge}>
                确认清理
              </button>
            </div>
          </div>
        </div>
      )}
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
