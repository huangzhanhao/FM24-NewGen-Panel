---
version: alpha
name: NewGAN-Manager-Design
description: >-
  Desktop utility app; high information density and action-oriented.
  CSS variables are the sole token source; components are global CSS classes
  (no dedicated UI library); system sans-serif is the base typeface.

colors:
  primary: "#2563eb"
  on-primary: "#ffffff"
  primary-active: "#1d4ed8"
  primary-soft: "#e8f0fe"
  ink: "#1f2329"
  muted: "#6b7280"
  canvas: "#f5f6f8"
  surface: "#ffffff"
  hairline: "#d9dde3"
  hover-bg: "#f0f2f5"
  input-bg: "#eef0f3"
  progress-bg: "#e3e6ea"
  danger: "#dc2626"
  danger-hover: "#b91c1c"
  danger-soft: "#fee2e2"
  success: "#16a34a"
  warning: "#b45309"
  busy: "#d97706"
  overlay: "rgba(0, 0, 0, 0.25)"

typography:
  heading-brand:
    fontFamily: "system-ui, sans-serif"
    fontSize: 15px
    fontWeight: 700
    lineHeight: 1.4
    letterSpacing: 0px
  heading-card:
    fontFamily: "system-ui, sans-serif"
    fontSize: 14px
    fontWeight: 600
    lineHeight: 1.4
    letterSpacing: 0px
  body-md:
    fontFamily: "system-ui, sans-serif"
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.45
    letterSpacing: 0px
  body-sm:
    fontFamily: "system-ui, sans-serif"
    fontSize: 13px
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: 0px
  caption:
    fontFamily: "system-ui, sans-serif"
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.35
    letterSpacing: 0px
  label-xs:
    fontFamily: "system-ui, sans-serif"
    fontSize: 11px
    fontWeight: 400
    lineHeight: 1.3
    letterSpacing: 0.5px
  button-md:
    fontFamily: "system-ui, sans-serif"
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.25
    letterSpacing: 0px
  button-lg:
    fontFamily: "system-ui, sans-serif"
    fontSize: 15px
    fontWeight: 400
    lineHeight: 1.25
    letterSpacing: 0px
  code-md:
    fontFamily: "Consolas, 'Cascadia Mono', monospace"
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: 0px

rounded:
  none: 0px
  xs: 2px
  sm: 4px
  md: 6px
  lg: 8px
  full: 999px

spacing:
  xxs: 2px
  xs: 4px
  sm: 6px
  md: 8px
  lg: 12px
  xl: 14px
  xxl: 16px
  section: 24px

components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
    padding: "6px 14px"
  button-primary-hover:
    backgroundColor: "{colors.primary-active}"
    textColor: "{colors.on-primary}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
    padding: "6px 14px"
  button-lg:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    typography: "{typography.button-lg}"
    rounded: "{rounded.sm}"
    padding: "10px 18px"
  button-disabled:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.muted}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
  button-danger:
    backgroundColor: "{colors.danger}"
    textColor: "{colors.on-primary}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
    padding: "6px 14px"
  button-danger-hover:
    backgroundColor: "{colors.danger-hover}"
    textColor: "{colors.on-primary}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
  button-filled-disabled:
    backgroundColor: "{colors.input-bg}"
    textColor: "{colors.muted}"
    typography: "{typography.button-md}"
    rounded: "{rounded.sm}"
  status-label:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.muted}"
    typography: "{typography.body-md}"
  status-label-pending:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.warning}"
    typography: "{typography.body-md}"
  icon-btn:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.muted}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
    padding: "2px 6px"
  icon-btn-danger:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.danger}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
    padding: "0px 10px"
  icon-btn-danger-hover:
    backgroundColor: "{colors.danger-soft}"
    textColor: "{colors.danger-hover}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
  text-input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
    padding: "6px 8px"
  text-input-readonly:
    backgroundColor: "{colors.input-bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
    padding: "6px 8px"
  text-input-focused:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-md}"
    rounded: "{rounded.sm}"
  card:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-md}"
    rounded: "{rounded.lg}"
    padding: "12px 14px"
  mode-chip:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.full}"
    padding: "4px 12px"
  mode-chip-active:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.on-primary}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.full}"
    padding: "4px 12px"
  table-header:
    backgroundColor: "{colors.input-bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
    padding: "4px 8px"
  table-row-hover:
    backgroundColor: "{colors.hover-bg}"
    textColor: "{colors.ink}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.none}"
  badge-error:
    backgroundColor: "{colors.danger}"
    textColor: "{colors.on-primary}"
    typography: "{typography.label-xs}"
    rounded: "{rounded.lg}"
    padding: "0px 6px"
  progress-track:
    backgroundColor: "{colors.progress-bg}"
    rounded: "5px"
    height: 10px
  progress-fill:
    backgroundColor: "{colors.primary}"
    rounded: "5px"
    height: 10px
---

# NewGAN Manager Design Contract

> **Structure 结构说明**
> This document contains two parallel versions:
> - **Part I** — Complete English version (all section headings and body prose in English).
> - **Part II** — 完整中文版，所有小节标题与正文均为中文。
>
> **Canvas behavior**：The Design Review Canvas viewer only indexes Part I because its English H2/H3 titles match the plugin's regex filters (`/do|dont|accessibility|gap/` etc.). Part II's Chinese titles intentionally do not match, so Canvas output stays clean and non-duplicated. To read Part II prose, use Markdown Preview (`Ctrl+K V`).

---

# Part I — English

## Overview

NewGAN Manager is a **desktop utility application** (Tauri 2 + Svelte 5) that manages football-manager player face-packet replacements.

**Product personality:**

- **Dense & efficient**: Information-first; no large whitespace. Base size 14px, compact controls.
- **Action-oriented**: Buttons and forms are the primary surface. Hierarchy via background/border, not shadows.
- **Single light theme**: No dark mode is currently implemented; all colors assume a light canvas.

**Design-system characteristics:**

- The single token source is the `:root` CSS variables in `src-svelte/app.css`.
- Components are implemented as global CSS classes (no dedicated UI library); primitives such as button / input / card exist as utility classes.
- Typeface uses the platform default sans-serif (`system-ui`); Consolas is used for monospaced surfaces.

## Colors

All application code must use semantic tokens; hard-coded hex/rgb values are not allowed in feature code.

| Token | Value | Semantic role |
| --- | --- | --- |
| `{colors.primary}` | `#2563eb` | Primary action (button, link, focus ring) |
| `{colors.on-primary}` | `#ffffff` | Foreground on primary |
| `{colors.primary-active}` | `#1d4ed8` | Primary hover / active |
| `{colors.primary-soft}` | `#e8f0fe` | Soft primary background (selected row) |
| `{colors.ink}` | `#1f2329` | Primary text |
| `{colors.muted}` | `#6b7280` | Secondary / muted text |
| `{colors.canvas}` | `#f5f6f8` | Page background |
| `{colors.surface}` | `#ffffff` | Panel / card background |
| `{colors.hairline}` | `#d9dde3` | Border / divider |
| `{colors.hover-bg}` | `#f0f2f5` | Generic hover background |
| `{colors.input-bg}` | `#eef0f3` | Readonly input / table header background |
| `{colors.progress-bg}` | `#e3e6ea` | Progress track |
| `{colors.danger}` | `#dc2626` | Error / destructive |
| `{colors.danger-hover}` | `#b91c1c` | Destructive button hover |
| `{colors.danger-soft}` | `#fee2e2` | Destructive icon hover background |
| `{colors.success}` | `#16a34a` | Success |
| `{colors.warning}` | `#b45309` | Warning text |
| `{colors.busy}` | `#d97706` | Busy status indicator |
| `{colors.overlay}` | `rgba(0,0,0,0.25)` | Backdrop overlay |

**Contrast requirements:**

- `{colors.ink}` on `{colors.surface}` ≥ 7:1 (AAA)
- `{colors.primary}` on `{colors.on-primary}` ≥ 4.5:1 (AA)
- `{colors.muted}` on `{colors.surface}` ≥ 4.5:1 (AA)

## Typography

Type scale maps to `{typography.*}` tokens. Each level names a role, not a raw size.

| Level | Size | Weight | Usage |
| --- | --- | --- | --- |
| `heading-brand` | 15px | 700 | Sidebar brand title |
| `heading-card` | 14px | 600 | Card title |
| `body-md` | 14px | 400 | Body / form |
| `body-sm` | 13px | 400 | Link, chip, table |
| `caption` | 12px | 400 | Secondary info, status bar |
| `label-xs` | 11px | 400 | Sidebar section head, badge |
| `button-md` | 14px | 400 | Standard button |
| `button-lg` | 15px | 400 | Big button (`.btn.big`) |
| `code-md` | 12px | 400 | Log / UID monospace surfaces |

Monospaced text uses the `.mono` class or `{typography.code-md}`.

## Layout

The app skeleton is a CSS Grid.

- Left column: fixed 250px sidebar (approximately `{spacing.section}` × 10).
- Right column: `1fr` main content area.
- Bottom row: `auto` status bar, spanning the right column only.
- Log drawer is a `position: fixed` right panel, width 520px / max 80vw.

Spacing rhythm uses `{spacing.*}`:

- Card padding: `12px 14px` → `{spacing.lg} {spacing.xl}`
- Row gap: `{spacing.md}` (8px)
- Content-area gap: `{spacing.lg}` (12px)
- Form label width: 90px (fixed)
- Path row (`.path-row`): 3-column grid — `90px` label / `minmax(0, 1fr)` readonly input / `104px` action button — so stacked path inputs keep identical edges
- Checkbox row (`.row.checks`): labels size to their content and never wrap; the row may open with a `.row-label` caption (e.g. `选项设置：`) that keeps right-aligning on the 90px gutter
- Dense field area (PlayerViewer `.fgrid`): wrapping flex box; at narrow widths fields re-flow instead of producing a horizontal scrollbar

## Elevation & Depth

This system **does not use `box-shadow`**. Surface hierarchy is expressed through:

1. Background color stepping: `{colors.canvas}` → `{colors.surface}` → `{colors.hover-bg}`
2. Borders: `1px solid {colors.hairline}`
3. Z-index stacking: overlay `40` / drawer `50`

## Shapes

Radius scale `{rounded.*}` and its usage:

| Token | Value | Usage |
| --- | --- | --- |
| `{rounded.none}` | `0px` | Table cells |
| `{rounded.sm}` | `4px` | Buttons, inputs, `.icon-btn` |
| `{rounded.md}` | `6px` | Log area, avatar preview |
| `{rounded.lg}` | `8px` | Cards, badge |
| `{rounded.full}` | `999px` | Mode chip / pill |

## Components

This project has no standalone UI library. All components are implemented as global CSS classes. The families and their states follow.

### Button

CSS class: `.btn` + variant modifier

| Variant | Class | Token |
| --- | --- | --- |
| Primary | `.btn.primary` | `{components.button-primary}` |
| Primary hover | `.btn.primary:hover` | `{components.button-primary-hover}` |
| Secondary | `.btn` | `{components.button-secondary}` |
| Danger hover | `.btn.danger:hover` | border → `{colors.danger}` |
| Danger solid | `.btn.danger-solid` | `{components.button-danger}` |
| Danger solid hover | `.btn.danger-solid:hover` | `{components.button-danger-hover}` |
| Big | `.btn.big` | `{components.button-lg}` |
| Disabled | `button:disabled` | `{components.button-disabled}`, opacity 0.5 |
| Disabled (filled) | `.btn.primary:disabled`, `.btn.danger-solid:disabled` | `{components.button-filled-disabled}`, opacity 1 |
| Icon | `.icon-btn` | `{components.icon-btn}` |
| Icon danger (list row) | `.group-list .icon-btn.danger` | `{components.icon-btn-danger}`, stretched to the full row height; hover `{components.icon-btn-danger-hover}` |
| Link | `.link` | No border, `{colors.primary}` text |

**Required states**: default / hover / focus-visible / disabled. Focus-visible is currently implemented as `outline: 2px solid {colors.primary}` for `input` / `select` only; buttons rely on the browser default focus ring.

### Input / Select

| State | Token |
| --- | --- |
| Default | `{components.text-input}` |
| Readonly | `{components.text-input-readonly}` |
| Focused | `{components.text-input-focused}` + `outline: 2px solid {colors.primary}` |

Size utilities: `.input-sm` (50px) / `.input-md` (60px) / `.input-lg` (140px) / `.w-full` / `.flex-1`

**Long read-only path inputs**: display is anchored to the **tail** of the path (the last directory / file name stays visible, head and middle may be clipped) via the `use:pinScrollEnd` action (`src-svelte/lib/actions.ts`); the full value stays available through the `title` tooltip. Do not solve this with `direction: rtl`, which reorders path separators.

### Card

`{components.card}` — basic container for the main content area. `.card.grow` is used when the card must fill the remaining flex space.

### Status Label 状态标签

CSS class: `.prf-state` (a `<span role="status">`, never a button — nothing is clickable).

| State | Class | Token | Meaning |
| --- | --- | --- | --- |
| Normal | `.prf-state` | `{components.status-label}` | 已激活：`{colors.muted}` text |
| Pending | `.prf-state.off` | `{components.status-label-pending}` | 未激活（`No Profile`）：`{colors.warning}` text |

The label text (已激活 / 未激活) carries the meaning; colour is only an emphasis, and the archive switch itself happens straight from the `存档资料` select.

### Mode Chip / Tabs

- `.mode-chip` → `{components.mode-chip}` (unselected)
- `.mode-chip.active` → `{components.mode-chip-active}` (selected)
- `.seg-tabs button.active` — 2px bottom border in `{colors.primary}` + weight 600

### Table

`table.grid` — sticky header + hover row.

- Header: `{components.table-header}`
- Row hover: `{components.table-row-hover}`
- Font size: `{typography.body-sm}`

### Progress

- Track: `{components.progress-track}` — full width or `.mini` (120px × 6px)
- Fill: `{components.progress-fill}` — `transition: width 0.15s ease`
- Dynamic width is set inline as `style="width:{n}%"` (acceptable exception).

### Badge

`{components.badge-error}` — error-count marker.

### Drawer

- Backdrop: `role="button"` + `aria-label` + `onkeydown(Escape)` + `{colors.overlay}`
- Panel: `position: fixed` right side, z-50, 520px
- Content: `.mono` monospaced log lines

## Accessibility Contract

- Every `<label>` must be associated with its control via `for`/`id`.
- Clickable overlays must expose a `role` and handle keyboard events.
- Color must never be the sole indicator of state; success/failure/warning also require text or icon.
- Focus must remain visible; `outline` must never be removed without an alternative.
- Motion must respect the user's "reduce motion" system setting.

## Do's and Don'ts

### Do

- Add new colors to `:root` CSS variables first, then reference them via `var(--token)`.
- Pick new radius values from `{rounded.sm}` / `{rounded.md}` / `{rounded.lg}` only.
- Reuse existing utility classes: `.flex-1`, `.w-full`, `.mono`, `.small`, `.muted`, `.row-label`.
- Layout properties (`flex`, `grid`, `overflow`, `position`) can be used directly; no token required.
- Runtime dynamic values (e.g. progress percentage `style="width:{n}%"`) may remain inline.
- Destructive commands that must stand out use the filled `.btn.danger-solid`; list-row destructive affordances use `.icon-btn.danger` stretched to the full row height.
- Status readouts (e.g. 已激活 / 未激活) are `.prf-state` labels, not buttons — a control that can never be clicked must not look like one.
- When the window shrinks, re-flow rows (`.row.checks`, `.fgrid`, `.viewer-row`) before ever falling back to a horizontal scrollbar.
- Read-only path inputs pair `use:pinScrollEnd` with a `title` tooltip so both the tail and the full value are reachable.

### Don't

- Do not hard-code hex/rgb colors in feature components.
- Do not hard-code `border-radius` in feature components; use `var(--radius-*)`.
- Do not use `<label>` as a pure layout spacer; use a `<span class="row-label">` instead.
- Do not add a new `.btn` sub-variant without updating the Button table in this document.
- Do not introduce an external UI library (Tailwind, shadcn, etc.) without a documented adapter plan.
- Do not hide focus outlines without an equally visible replacement indicator.

## Implementation Alignment

| Design token | Location |
| --- | --- |
| `:root` CSS variables | `src-svelte/app.css` — `:root` block |
| `.btn` family | `src-svelte/app.css` — “按钮 / Buttons” block |
| `.card` family | `src-svelte/app.css` — “卡片 / Card” block |
| `.row` / `.row.checks` / `.path-row` | `src-svelte/app.css` — “表单行 / Form row” block |
| Sidebar rows (`.prf-actions`, `.prf-state`, `.group-list .icon-btn.danger`) | `src-svelte/app.css` — “侧栏 / Sidebar” block |
| `.mode-chip` | `src-svelte/app.css` — “模式 chip / 分段 tab” block |
| `table.grid` | `src-svelte/app.css` — “表格 / Table” block |
| `.progress-*` | `src-svelte/app.css` — “进度 / Progress” block |
| `.drawer-*` | `src-svelte/app.css` — “日志抽屉” block |
| `.statusbar` | `src-svelte/app.css` — “状态栏” block |
| Utility classes | `src-svelte/app.css` — “杂项 / Misc” block |
| `pinScrollEnd` action | `src-svelte/lib/actions.ts` |
| PlayerViewer scoped styles (`.fgrid`, `.field`, `.replace-row`) | `src-svelte/lib/components/PlayerViewer.svelte` `<style>` |

## Known Gaps

- `needs-design-decision`: Whether to implement a dark mode. All current tokens assume a light design.
- `needs-design-decision`: Whether Button focus-visible should use a custom outline consistently (currently relies on the browser default for buttons).
- `needs-design-decision`: Whether the 250px fixed sidebar width needs a responsive collapse.

---

# Part II — 中文版

## 概述

NewGAN Manager 是一款 **桌面工具型应用**（Tauri 2 + Svelte 5），用于管理足球经理游戏的球员头像替换。

**产品性格：**

- **高效密集**：信息优先，不追求大留白。主字号 14px、控件紧凑。
- **操作导向**：按钮/表单为主要交互面，视觉层级靠背景和边框区分而非阴影。
- **单主题浅色**：当前不实现暗色模式，所有颜色基于浅色前提。

**设计系统特征：**

- 唯一的 token 源为 `src-svelte/app.css` 中的 `:root` CSS 变量。
- 组件通过全局 CSS class 实现（无独立 UI 组件库），button / input / card 等原语以 utility class 形式提供。
- 字体使用系统默认 sans-serif（`system-ui`），等宽场景使用 Consolas。

## 色彩

所有应用代码必须使用语义 token，不得在功能代码中硬编码 hex/rgb 值。

| Token | 色值 | 语义角色 |
| --- | --- | --- |
| `{colors.primary}` | `#2563eb` | 主操作色（按钮、链接、焦点环） |
| `{colors.on-primary}` | `#ffffff` | 主操作色上的前景文字 |
| `{colors.primary-active}` | `#1d4ed8` | 主操作色悬停/激活态 |
| `{colors.primary-soft}` | `#e8f0fe` | 主操作色浅底（选中行背景） |
| `{colors.ink}` | `#1f2329` | 主文本 |
| `{colors.muted}` | `#6b7280` | 辅助/弱化文本 |
| `{colors.canvas}` | `#f5f6f8` | 页面背景 |
| `{colors.surface}` | `#ffffff` | 面板/卡片背景 |
| `{colors.hairline}` | `#d9dde3` | 边框、分割线 |
| `{colors.hover-bg}` | `#f0f2f5` | 通用悬停背景 |
| `{colors.input-bg}` | `#eef0f3` | 只读输入/表头背景 |
| `{colors.progress-bg}` | `#e3e6ea` | 进度条轨道 |
| `{colors.danger}` | `#dc2626` | 错误/危险 |
| `{colors.danger-hover}` | `#b91c1c` | 危险按钮悬停 |
| `{colors.danger-soft}` | `#fee2e2` | 危险图标悬停背景 |
| `{colors.success}` | `#16a34a` | 成功 |
| `{colors.warning}` | `#b45309` | 警告文本 |
| `{colors.busy}` | `#d97706` | 忙碌状态指示 |
| `{colors.overlay}` | `rgba(0,0,0,0.25)` | 遮罩层 |

**对比度要求：**

- `{colors.ink}` 配 `{colors.surface}` ≥ 7:1（AAA）
- `{colors.primary}` 配 `{colors.on-primary}` ≥ 4.5:1（AA）
- `{colors.muted}` 配 `{colors.surface}` ≥ 4.5:1（AA）

## 排版

字体层级对应 `{typography.*}` token，每一级命名的是用途而非原始字号。

| 层级 | 字号 | 字重 | 用途 |
| --- | --- | --- | --- |
| `heading-brand` | 15px | 700 | 侧栏品牌标题 |
| `heading-card` | 14px | 600 | 卡片标题 |
| `body-md` | 14px | 400 | 正文/表单 |
| `body-sm` | 13px | 400 | 链接、chip、表格 |
| `caption` | 12px | 400 | 辅助信息、状态栏 |
| `label-xs` | 11px | 400 | 侧栏 section 标题、badge |
| `button-md` | 14px | 400 | 标准按钮 |
| `button-lg` | 15px | 400 | 大按钮 (`.btn.big`) |
| `code-md` | 12px | 400 | 日志/UID 等宽场景 |

等宽字体通过 `.mono` class 或 `{typography.code-md}` 引用。

## 布局

应用骨架为 CSS Grid。

- 左列：固定 250px 侧栏（约等于 `{spacing.section}` × 10）。
- 右列：`1fr` 主内容区。
- 底行：`auto` 状态栏（仅跨右列）。
- 日志抽屉为 `position: fixed` 右侧面板，宽 520px / max 80vw。

间距节奏使用 `{spacing.*}`：

- 卡片内边距：`12px 14px` → `{spacing.lg} {spacing.xl}`
- 行间 gap：`{spacing.md}` (8px)
- 内容区 gap：`{spacing.lg}` (12px)
- 表单 label 宽度：90px（固定值）
- 路径行（`.path-row`）：三列栅格 —— `90px` label / `minmax(0, 1fr)` 只读输入框 / `104px` 操作按钮，保证多行路径输入框左右边界完全对齐
- 勾选行（`.row.checks`）：label 宽度随内容自适应，文字不得换行；行首可用 `.row-label` 放标题（如 `选项设置：`），仍按 90px 栏右对齐
- 密集字段区（PlayerViewer `.fgrid`）：可换行 flex；窗口变窄时字段换行，而不是出现横向滚动条

## 高度与层次

本系统 **不使用 `box-shadow`**。表面层次通过：

1. 背景色差异：`{colors.canvas}` → `{colors.surface}` → `{colors.hover-bg}`
2. 边框：`1px solid {colors.hairline}`
3. z-index 堆叠：遮罩层 `40` / 抽屉 `50`

## 形状

圆角 scale `{rounded.*}` 及对应用途：

| Token | 值 | 使用场景 |
| --- | --- | --- |
| `{rounded.none}` | `0px` | 表格单元格 |
| `{rounded.sm}` | `4px` | 按钮、输入框、`.icon-btn` |
| `{rounded.md}` | `6px` | 日志区、头像预览 |
| `{rounded.lg}` | `8px` | 卡片、badge |
| `{rounded.full}` | `999px` | mode-chip / 胶囊 |

## 组件

本项目无独立 UI 库，所有组件通过全局 CSS class 实现。以下为组件族及其状态。

### Button 按钮

CSS class：`.btn` + 变体修饰符

| 变体 | class | 对应 token |
| --- | --- | --- |
| 主按钮 | `.btn.primary` | `{components.button-primary}` |
| 主按钮悬停 | `.btn.primary:hover` | `{components.button-primary-hover}` |
| 次按钮 | `.btn` | `{components.button-secondary}` |
| 危险悬停 | `.btn.danger:hover` | 边框 → `{colors.danger}` |
| 危险实心 | `.btn.danger-solid` | `{components.button-danger}` |
| 危险实心悬停 | `.btn.danger-solid:hover` | `{components.button-danger-hover}` |
| 大按钮 | `.btn.big` | `{components.button-lg}` |
| 禁用 | `button:disabled` | `{components.button-disabled}`, 透明度 0.5 |
| 禁用（满底） | `.btn.primary:disabled`、`.btn.danger-solid:disabled` | `{components.button-filled-disabled}`, 透明度 1（中性灰） |
| 图标按钮 | `.icon-btn` | `{components.icon-btn}` |
| 图标危险（列表行） | `.group-list .icon-btn.danger` | `{components.icon-btn-danger}`，上下占满整行；悬停 `{components.icon-btn-danger-hover}` |
| 链接按钮 | `.link` | 无边框，`{colors.primary}` 文字 |

**必要状态**：default / hover / focus-visible / disabled。当前 focus-visible 通过 `outline: 2px solid {colors.primary}` 实现（仅 input/select 生效），button 依赖浏览器默认 focus ring。

### Input / Select 表单控件

| 状态 | 对应 token |
| --- | --- |
| 默认 | `{components.text-input}` |
| 只读 | `{components.text-input-readonly}` |
| 聚焦 | `{components.text-input-focused}` + `outline: 2px solid {colors.primary}` |

尺寸工具类：`.input-sm` (50px) / `.input-md` (60px) / `.input-lg` (140px) / `.w-full` / `.flex-1`

**长路径只读输入框**：显示位置钉在路径**尾部**（末级目录 / 文件名始终可见，头部与中间内容可被裁切），由 `use:pinScrollEnd` action 实现（`src-svelte/lib/actions.ts`），完整值通过 `title` 悬停提示获取。不得用 `direction: rtl` 实现，那会打乱路径分隔符顺序。

### Card 卡片

`{components.card}` — 主区基本容器。`.card.grow` 用于需要 flex 填充的场景。

### 状态标签

CSS class：`.prf-state`（是 `<span role="status">`，不是按钮——没有任何可点操作）。

| 状态 | class | 对应 token | 含义 |
| --- | --- | --- | --- |
| 常规 | `.prf-state` | `{components.status-label}` | 已激活：`{colors.muted}` 文字 |
| 待处理 | `.prf-state.off` | `{components.status-label-pending}` | 未激活（`No Profile`）：`{colors.warning}` 文字 |

状态含义由文字（已激活 / 未激活）承载，颜色只是强调；切档动作本身直接由「存档资料」下拉框完成。

### Mode Chip / Tabs 模式选择器

- `.mode-chip` → `{components.mode-chip}`（未选中）
- `.mode-chip.active` → `{components.mode-chip-active}`（选中）
- `.seg-tabs button.active` — 底部 2px `{colors.primary}` 边框 + 600 字重

### Table 数据表格

`table.grid` — sticky 表头 + 悬停行高亮。

- 表头：`{components.table-header}`
- 悬停行：`{components.table-row-hover}`
- 字号：`{typography.body-sm}`

### Progress 进度条

- 轨道：`{components.progress-track}` — 全宽或 `.mini` (120px × 6px)
- 填充：`{components.progress-fill}` — `transition: width 0.15s ease`
- 动态 width 通过内联 `style="width:{n}%"` 设置（可接受例外）。

### Badge 徽章

`{components.badge-error}` — 错误计数标记。

### Drawer 日志抽屉

- 遮罩：`role="button"` + `aria-label` + `onkeydown(Escape)` + `{colors.overlay}`
- 面板：`position: fixed` 右侧，z-50，520px
- 内容：`.mono` 等宽日志行

## 无障碍约定

- 所有 `<label>` 必须通过 `for`/`id` 关联到对应控件。
- 遮罩/可点击覆盖层必须有 `role` + 键盘事件处理。
- 颜色不作为唯一状态传达：成功/失败/警告须配合文字或图标。
- 焦点可见：`outline` 不得被移除，除非同时提供替代可见焦点指示。
- 动画遵循系统"减少动画"设置。

## 设计约定

### 应该

- 新颜色值先添加到 `:root` CSS 变量，再在组件中使用 `var(--token)`。
- 新圆角值使用 `{rounded.sm}` / `{rounded.md}` / `{rounded.lg}` 之一。
- 复用已有工具类：`.flex-1`, `.w-full`, `.mono`, `.small`, `.muted`, `.row-label`。
- 布局属性（`flex`, `grid`, `overflow`, `position`）可以直接使用，无需 token 化。
- 动态运行时值（如进度百分比 `style="width:{n}%"`）可以保留为内联样式。
- 需要强调的危险操作用满底 `.btn.danger-solid`；列表行内的危险入口用 `.icon-btn.danger` 并上下占满整行。
- 状态读数（如 已激活 / 未激活）用 `.prf-state` 标签而不是按钮——永远点不动的控件不该长得像按钮。
- 窗口变窄时先让行换行（`.row.checks`、`.fgrid`、`.viewer-row`），不要依赖横向滚动条。
- 只读路径输入框同时提供 `use:pinScrollEnd` 与 `title`，尾部与完整值都可获取。

### 不应该

- 不要在功能组件中硬编码 hex/rgb 颜色。
- 不要在功能组件中硬编码 `border-radius` 数值，使用 `var(--radius-*)`。
- 不要用 `<label>` 做纯布局占位——改用 `<span class="row-label">`。
- 不要创建新的 `.btn` 子变体而不更新此文档的 Button 表格。
- 不要引入外部 UI 组件库（如 Tailwind、shadcn）而不做适配层规划。
- 不要隐藏 focus outline 除非同时提供同等可见的替代焦点指示。

## 实现映射

| 设计 token | 实现位置 |
| --- | --- |
| `:root` CSS 变量 | `src-svelte/app.css` — `:root` 块 |
| `.btn` 按钮系列 | `src-svelte/app.css` — 「按钮」块 |
| `.card` 卡片系列 | `src-svelte/app.css` — 「卡片」块 |
| `.row` / `.row.checks` / `.path-row` | `src-svelte/app.css` — 「表单行」块 |
| 侧栏行（`.prf-actions`、`.prf-state`、`.group-list .icon-btn.danger`） | `src-svelte/app.css` — 「侧栏」块 |
| `.mode-chip` | `src-svelte/app.css` — 「模式 chip / 分段 tab」块 |
| `table.grid` | `src-svelte/app.css` — 「表格」块 |
| `.progress-*` 进度条 | `src-svelte/app.css` — 「进度」块 |
| `.drawer-*` 抽屉 | `src-svelte/app.css` — 「日志抽屉」块 |
| `.statusbar` 状态栏 | `src-svelte/app.css` — 「状态栏」块 |
| 工具类 | `src-svelte/app.css` — 「杂项」块 |
| `pinScrollEnd` action | `src-svelte/lib/actions.ts` |
| PlayerViewer 局部样式（`.fgrid`、`.field`、`.replace-row`） | `src-svelte/lib/components/PlayerViewer.svelte` `<style>` |

## 已知缺口

- `needs-design-decision`：是否实现暗色模式。当前所有 token 仅针对浅色设计。
- `needs-design-decision`：Button focus-visible 是否应统一为自定义 outline（当前依赖浏览器默认）。
- `needs-design-decision`：侧栏 250px 固定宽度是否需要响应式折叠。
