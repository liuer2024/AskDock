# QuestionDock Tauri MVP 方案

## 1. 产品定位

QuestionDock 是一款贴近终端窗口的轻量浮窗应用，用来记录用户当前正在处理的问题，以及历史提问。

它不替代终端，也不做完整任务管理。第一版只解决一个场景：

> 用户在多个终端任务之间切换时，能立刻看到“我刚才在问什么、现在在处理哪个问题”。

## 2. MVP 范围

第一版只做两件事：

1. 贴近当前终端窗口显示浮窗。
2. 显示当前问题和历史问题。

不进入第一版的能力：

- 自动读取终端命令。
- 自动总结命令输出。
- 接入 Codex、Claude、ChatGPT 等 AI 对话。
- 任务协作、云同步、团队共享。
- 深度嵌入 Ghostty、iTerm2、Warp 等终端内部。

## 3. 目标用户

主要面向频繁使用终端和 AI 助手的开发者、运维人员、独立开发者。

典型痛点：

- 开了多个终端窗口后，忘记每个窗口对应的问题。
- 等命令执行时去处理别的事，回来后不知道刚才目标是什么。
- 连续问 AI 多个问题后，当前上下文容易混在一起。
- 终端历史能看到“执行了什么”，但看不到“为什么执行”。

## 4. 核心体验

用户打开终端时，QuestionDock 浮窗贴在终端右侧。

```text
┌────────────────────────────┐ ┌──────────────────┐
│ Terminal                   │ │ QuestionDock     │
│                            │ │                  │
│ $ npm run dev              │ │ 当前问题          │
│                            │ │ 登录页样式错位    │
│                            │ │                  │
│                            │ │ 历史问题          │
│                            │ │ API 500 报错      │
│                            │ │ 构建失败          │
└────────────────────────────┘ └──────────────────┘
```

用户可以：

- 使用快捷键新增一个问题。
- 将某个历史问题设为当前问题。
- 标记问题为完成。
- 收起或展开浮窗。
- 在多个终端窗口之间切换时，浮窗跟随当前活跃终端窗口。

## 5. 产品形态

### 5.1 桌面浮窗

QuestionDock 是一个独立桌面应用窗口，不嵌入终端内部。

原因：

- Ghostty、iTerm2、Terminal、Alacritty 等终端的插件能力不统一。
- 跨平台插件接口不可控。
- 系统级浮窗更容易实现跨终端兼容。

浮窗行为：

- 默认宽度：280px。
- 默认贴在当前终端窗口右侧。
- 高度与终端窗口一致，或使用最小高度 480px。
- 终端窗口靠近屏幕右侧时，浮窗可以贴在终端左侧。
- 用户可手动拖拽浮窗，拖拽后进入“手动定位模式”。
- 用户可点击按钮恢复“自动贴靠模式”。

### 5.2 菜单栏 / 托盘

应用常驻系统托盘或菜单栏。

托盘功能：

- 显示 / 隐藏浮窗。
- 新增问题。
- 打开设置。
- 退出应用。

### 5.3 快捷键

建议默认快捷键：

- 新增问题：`Option + Space`
- 显示 / 隐藏浮窗：`Option + Q`
- 切换当前问题：`Option + Tab`

快捷键必须允许用户自定义，避免和系统或编辑器冲突。

## 6. 信息结构

### 6.1 当前问题

当前问题是浮窗的核心区域。

字段：

- 标题。
- 创建时间。
- 所属窗口或应用。
- 状态：进行中、已完成、已暂停。
- 简短备注。

显示示例：

```text
当前问题
登录页样式错位

备注
移动端按钮被遮住，正在检查 CSS。
```

### 6.2 历史问题

历史问题按最近更新时间排序。

每条展示：

- 问题标题。
- 状态。
- 更新时间。

点击历史问题后，可以：

- 设为当前问题。
- 编辑标题。
- 添加备注。
- 标记完成。
- 删除。

## 7. MVP 界面结构

```text
┌──────────────────────────┐
│ QuestionDock          +  │
├──────────────────────────┤
│ 当前问题                  │
│ 登录页样式错位            │
│                          │
│ [编辑] [完成] [暂停]      │
├──────────────────────────┤
│ 历史问题                  │
│ API 500 报错              │
│ 构建失败                  │
│ brew 安装异常             │
├──────────────────────────┤
│ 搜索问题...               │
└──────────────────────────┘
```

第一版界面原则：

- 信息密度高，不做营销式界面。
- 不使用大面积装饰图形。
- 侧栏宽度稳定，文字过长自动换行或截断。
- 当前问题永远在顶部。
- 新增问题入口必须明显。

## 8. 技术路线

### 8.1 框架

采用 Tauri。

推荐组合：

- Tauri 2.x
- Rust 后端
- React / Vue / Svelte 前端任选其一
- SQLite 本地存储

如果追求最快 MVP，建议：

- 前端：React + Vite
- UI：自写轻量组件，不引入重型组件库
- 数据库：SQLite

### 8.2 窗口能力

需要的 Tauri 窗口能力：

- 无边框窗口。
- always-on-top。
- 设置窗口位置和大小。
- 获取显示器尺寸。
- 隐藏 / 显示窗口。
- 全局快捷键。
- 系统托盘。

### 8.3 终端窗口识别

第一版目标是识别常见终端应用窗口：

- Ghostty
- iTerm2
- Apple Terminal
- Warp
- Alacritty
- WezTerm

识别方式按平台区分。

#### macOS

使用 Accessibility API 获取当前前台应用和窗口位置。

需要用户授权：

- Accessibility 权限。

实现要点：

- 监听前台应用变化。
- 判断 bundle identifier 是否属于终端应用。
- 获取活跃窗口的 `x`、`y`、`width`、`height`。
- 将 QuestionDock 放到终端窗口右侧。

推荐优先支持 macOS，因为 Ghostty、iTerm2、Terminal 是该需求最明确的场景。

#### Windows

使用 Win32 API：

- 获取前台窗口。
- 读取窗口标题、进程名、窗口矩形。
- 判断进程是否为 Windows Terminal、WezTerm、Alacritty、Ghostty 等。
- 将浮窗贴到右侧。

需要注意：

- 多显示器 DPI 缩放。
- 管理员权限窗口可能无法被普通权限应用完整控制。

#### Linux

Linux 需要按桌面环境分层处理：

- X11：可以用 Xlib / xdotool 类能力获取窗口信息。
- Wayland：限制较多，不同 compositor 支持不同。

MVP 可以先声明：

- Linux X11 支持自动贴靠。
- Wayland 第一版只支持手动贴靠或快捷键显示。

## 9. 窗口贴靠算法

输入：

- 终端窗口矩形：`terminalRect`
- 当前显示器可用区域：`screenRect`
- 浮窗宽度：`dockWidth`
- 间距：`gap`

规则：

1. 优先贴在终端右侧。
2. 如果右侧空间不足，贴在终端左侧。
3. 如果左右空间都不足，浮窗覆盖在终端右边缘上方。
4. 浮窗高度跟随终端高度。
5. 浮窗顶部与终端顶部对齐。

伪代码：

```ts
function calculateDockRect(terminalRect, screenRect) {
  const dockWidth = 280;
  const gap = 8;
  const rightX = terminalRect.x + terminalRect.width + gap;
  const leftX = terminalRect.x - dockWidth - gap;

  if (rightX + dockWidth <= screenRect.x + screenRect.width) {
    return {
      x: rightX,
      y: terminalRect.y,
      width: dockWidth,
      height: terminalRect.height,
    };
  }

  if (leftX >= screenRect.x) {
    return {
      x: leftX,
      y: terminalRect.y,
      width: dockWidth,
      height: terminalRect.height,
    };
  }

  return {
    x: terminalRect.x + terminalRect.width - dockWidth,
    y: terminalRect.y,
    width: dockWidth,
    height: terminalRect.height,
  };
}
```

## 10. 数据模型

### 10.1 questions

```sql
CREATE TABLE questions (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  note TEXT,
  status TEXT NOT NULL,
  source_app TEXT,
  source_window_title TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);
```

### 10.2 settings

```sql
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

常见设置：

- `dock_width`
- `dock_gap`
- `auto_attach_enabled`
- `theme`
- `global_shortcut_add_question`
- `global_shortcut_toggle_window`

## 11. 关键状态

### 11.1 自动贴靠模式

浮窗跟随当前终端窗口。

触发更新：

- 前台应用变化。
- 终端窗口移动。
- 终端窗口缩放。
- 显示器变化。
- 用户切换 Space 或虚拟桌面。

### 11.2 手动定位模式

用户拖动浮窗后，暂时停止自动贴靠。

用户可以点击“重新贴靠”按钮回到自动模式。

### 11.3 非终端应用前台

可选策略：

1. 保持浮窗在最后一个终端旁边。
2. 自动隐藏浮窗。
3. 缩成小胶囊。

MVP 推荐使用策略 1，减少窗口闪烁。

## 12. MVP 开发里程碑

### 阶段 1：基础应用

- Tauri 项目初始化。
- 创建无边框浮窗。
- 托盘菜单。
- 显示 / 隐藏浮窗。
- 本地 SQLite 初始化。

### 阶段 2：问题管理

- 新增问题。
- 编辑问题。
- 删除问题。
- 标记完成。
- 当前问题展示。
- 历史问题列表。

### 阶段 3：贴靠能力

- macOS 获取前台窗口。
- 识别常见终端应用。
- 根据终端窗口位置计算浮窗位置。
- 移动终端时同步浮窗。

### 阶段 4：体验打磨

- 全局快捷键。
- 浮窗收起状态。
- 搜索历史问题。
- 设置页。
- 权限引导。

## 13. 主要风险

### 13.1 跨平台窗口能力不一致

macOS 和 Windows 可控性较好，Linux Wayland 限制较多。

应对：

- 第一版明确 macOS 为最佳体验平台。
- Windows 作为第二优先级。
- Linux 提供降级体验。

### 13.2 浮窗打扰用户

浮窗如果频繁跳动，会让用户烦躁。

应对：

- 增加贴靠延迟和防抖。
- 用户拖动后进入手动模式。
- 不主动改变终端窗口大小。

### 13.3 权限门槛

macOS 需要 Accessibility 权限，用户可能不理解。

应对：

- 首次启动给出简短权限说明。
- 无权限时仍可手动使用浮窗。
- 权限只用于识别和贴靠窗口，不读取终端内容。

### 13.4 产品边界膨胀

容易变成任务管理、笔记、AI 助手、终端增强的大杂烩。

应对：

- 第一版只围绕“当前问题”和“历史问题”。
- 不做复杂项目系统。
- 不做自动总结。
- 不做团队功能。

## 14. 可行性结论

采用 Tauri 做跨平台版本是可行的。

最稳妥的实现方式是：

- 应用本身作为独立浮窗存在。
- 通过系统窗口 API 贴近终端。
- 通过本地数据库记录问题。
- 第一版不读取终端内容，不嵌入终端内部。

这个 MVP 技术难度适中，产品边界清晰，适合快速验证。

推荐第一阶段只做 macOS 原型：

1. 识别 Ghostty / iTerm2 / Terminal 当前窗口。
2. 浮窗贴右侧。
3. 能新增和查看问题。

如果这个体验成立，再扩展 Windows 和 Linux。

