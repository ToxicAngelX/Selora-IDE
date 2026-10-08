# Selora IDE — Design Tokens

This document is the **single source of truth** for every visual value in Selora IDE. Build agents implement these as a Rust theme module (`crates/selora-ui/src/theme.rs`) that every UI component reads from. **No component hardcodes a color, font, or spacing value.** All values come from here.

The design language follows the Selora brand: monochrome dark, hairline borders, calm motion, functional green only.

---

## 1. Color tokens

### 1.1 The ink ramp

The IDE uses a true-black canvas with a 5-step grayscale ramp. The ramp is the entire color system — there are no other neutral colors.

| Token | Hex | RGB | Use |
|---|---|---|---|
| `--ink-canvas` | `#000000` | `0, 0, 0` | Window background, central editor canvas |
| `--ink-faint` | `#0a0a0a` | `10, 10, 10` | Subtle surface lift (e.g. hovered rows in trees) |
| `--ink-low` | `#141414` | `20, 20, 20` | Panel surfaces, sidebar, status bar |
| `--ink-mid` | `#1f1f1f` | `31, 31, 31` | Modal, dropdown, command palette |
| `--ink-hi` | `#2a2a2a` | `42, 42, 42` | Borders, dividers, focused surfaces |
| `--ink-rule` | `#3a3a3a` | `58, 58, 58` | Stronger rule, ghost text, disabled text |

### 1.2 Foreground (text)

| Token | Hex | RGB | Use |
|---|---|---|---|
| `--fg-default` | `#e5e5e5` | `229, 229, 229` | All body text, code |
| `--fg-muted` | `#8a8a8a` | `138, 138, 138` | Secondary text, labels, file tree secondary |
| `--fg-faint` | `#5a5a5a` | `90, 90, 90` | Tertiary text, line numbers, ghost text |
| `--fg-disabled` | `#3a3a3a` | `58, 58, 58` | Disabled controls, placeholders |

### 1.3 Functional green (the only color that is allowed to be a color)

`#22c55e` is **reserved for status, liveness, and OK indicators only**. It is never used as decoration, accent, gradient, or hover.

| Token | Hex | RGB | Use |
|---|---|---|---|
| `--ok` | `#22c55e` | `34, 197, 94` | "Connected", "Saved", "Tests pass", live dot |

### 1.4 Status colors (used only inside small badges/dots)

The rest of the status palette stays muted to preserve the monochrome feel. Saturated only when needed for meaning.

| Token | Hex | Use |
|---|---|---|
| `--warn` | `#d4a017` | Warnings, dirty, attention |
| `--err` | `#dc2626` | Errors, diagnostics squiggle, "key invalid" |
| `--info` | `#6b7280` | Informational, "indexing", "queued" |

### 1.5 Syntax highlighting palette (mono-tinted)

Syntax colors stay inside the ink ramp's neutral family, with the smallest possible tint shifts. The goal is that code reads like structure, not like a rainbow.

| Token | Hex | Use |
|---|---|---|
| `--syn-default` | `#e5e5e5` | Default text |
| `--syn-keyword` | `#cfcfcf` | Keywords (`fn`, `let`, `if`) — slightly brighter |
| `--syn-fn` | `#ffffff` | Function names — pure white, the brightest |
| `--syn-type` | `#b8b8b8` | Type names, classes |
| `--syn-string` | `#9a9a9a` | Strings — slightly dimmer |
| `--syn-number` | `#d0d0d0` | Numeric literals |
| `--syn-comment` | `#5a5a5a` | Comments, italics |
| `--syn-punct` | `#8a8a8a` | Punctuation, brackets |
| `--syn-var` | `#e5e5e5` | Variables — same as default (kept for future) |
| `--syn-attr` | `#a8a8a8` | Attributes, decorators |

The 13-color syntax palette is on the low end (VS Code uses 20+). This is intentional: the brand is monochrome, the syntax highlighting serves structure not flourish.

### 1.6 Diagnostic colors

| Token | Hex | Use |
|---|---|---|
| `--diag-err` | `#dc2626` | Error squiggle (3px underline) |
| `--diag-warn` | `#d4a017` | Warning squiggle |
| `--diag-info` | `#6b7280` | Info squiggle |

Diagnostics are the one place saturated red/yellow are allowed because they are meaningful, not decorative.

### 1.7 Overlay scrim

One token for the modal-backdrop dim — every `egui::Modal` backdrop (settings,
about, and the palette-class overlays) paints it; no ad-hoc alphas anywhere
(the token gap STATUS.md 6g noted, closed in Phase R0).

| Token | Value | Use |
|---|---|---|
| `--scrim` | `#000000` @ 60% alpha | Modal backdrops |

---

## 2. Typography

### 2.1 Font families

| Token | Family | License | Use |
|---|---|---|---|
| `--font-sans` | `Space Grotesk` | OFL | All UI text, headings, labels, buttons |
| `--font-mono` | `JetBrains Mono` | OFL | Editor text, code, file tree, terminal-style values, status bar numerics |

Both are **bundled** in `assets/fonts/`. We do not download at runtime; we do not depend on the OS having them.

### 2.2 Type scale

A 6-step scale, anchored at 13px body (the IDE's working size).

| Token | Size (px) | Line-height | Weight | Use |
|---|---|---|---|---|
| `--type-xs` | 11 | 16 | 400 | Status bar, micro-labels, tooltips |
| `--type-sm` | 12 | 18 | 400 | File tree rows, secondary UI |
| `--type-md` | 13 | 20 | 400 | Default body, editor text, menus |
| `--type-lg` | 15 | 24 | 500 | Headings, modal titles, panel titles |
| `--type-xl` | 18 | 26 | 600 | App title in title bar (Selora IDE), splash |
| `--type-xxl` | 24 | 32 | 600 | First-run welcome heading |

Default editor font size is `--type-md` (13px). User-overridable via `editor.font_size` in settings.

### 2.3 Weights

| Token | Value | Used by |
|---|---|---|
| `--weight-regular` | 400 | Body, code |
| `--weight-medium` | 500 | Headings (`--type-lg`), buttons |
| `--weight-semibold` | 600 | Title bar (`--type-xl`), splash (`--type-xxl`) |

There is no `--weight-bold` (700) in the IDE. The brand is calm; bold is too loud.

### 2.4 Letter-spacing

| Token | Value | Use |
|---|---|---|
| `--tracking-tight` | `-0.01em` | App title (negative tracking is a Space Grotesk hallmark) |
| `--tracking-normal` | `0` | Default |
| `--tracking-wide` | `0.04em` | Uppercase micro-labels (e.g. "FILE", "EDIT" menu headers) |

### 2.5 Italic

Only used for:
- Comments in syntax highlighting (`--syn-comment`).
- Placeholder text in empty states (e.g. "Press Ctrl+P to find a file").

Italic is **not** used for emphasis in regular UI.

---

## 3. Spacing scale

A 4px base, 6 steps. Everything in the IDE is on this grid.

| Token | Value (px) | Use |
|---|---|---|
| `--space-0` | 0 | Hairline gap, between siblings of equal weight |
| `--space-1` | 4 | Tight stack (icon + label inside a chip) |
| `--space-2` | 8 | Default stack (label rows, list items) |
| `--space-3` | 12 | Panel padding (vertical) |
| `--space-4` | 16 | Panel padding (horizontal), section gap |
| `--space-5` | 24 | Major section gap (between editor and chat) |
| `--space-6` | 32 | Splash / first-run padding |

### 3.1 Panel internal padding

- Sidebars (file tree, chat): `--space-3` vertical, `--space-4` horizontal.
- Editor canvas: no padding (text touches the gutter).
- Modals: `--space-5` on all sides.
- Status bar: `--space-2` vertical, `--space-4` horizontal.

---

## 4. Radii

Minimal radii. The IDE is mostly square. Round only where it softens a control.

| Token | Value (px) | Use |
|---|---|---|
| `--radius-0` | 0 | Panels, sidebars, status bar, editor canvas |
| `--radius-1` | 2 | Buttons, inputs, chips, tabs |
| `--radius-2` | 4 | Modals, command palette, dropdowns |
| `--radius-pill` | 999 | Status dots, badges (very few) |

There is no `--radius-3` (8px) or larger. The brand is sharp.

---

## 5. Borders

All borders are 1px. There are no 2px borders, no double lines, no shadows.

| Token | Value | Use |
|---|---|---|
| `--border-hairline` | `1px solid var(--ink-hi)` | Default panel divider, status bar top, modal outline |
| `--border-focus` | `1px solid var(--fg-default)` | Focused control (inverse — light line on dark) |
| `--border-error` | `1px solid var(--err)` | Error state (e.g. invalid settings input) |

### 5.1 Where borders go

- Between file tree sidebar and editor canvas: 1px hairline on the right edge of the sidebar.
- Between editor and chat sidebar: 1px hairline on the left edge of the chat sidebar.
- Above status bar: 1px hairline.
- Around modals / command palette: 1px hairline, no shadow.

### 5.2 Where borders do NOT go

- No border around individual file tree rows.
- No border around editor tabs (use ink-faint background for active tab instead).
- No border around menu bar items.
- No border around status bar segments.

---

## 6. Shadows

There are **no shadows** in the IDE. Layering is communicated by 1px hairlines and `--ink-faint` / `--ink-low` / `--ink-mid` surface lifts.

The single exception: the **command palette** has a 1-layer hairline border only. No drop shadow.

If a UI surface needs to feel "above" another, raise its background by one ink step (e.g. modal on top of editor = `--ink-mid` on `--ink-low`).

---

## 7. Motion

The motion vocabulary is the Selora glyph vocabulary: **orbit, pulse, spark, wave, spin, fade, bloom, comet**. Each animation in the IDE maps to one of these.

### 7.1 Curves

| Token | Cubic bezier | Use |
|---|---|---|
| `--ease-precise` | `cubic-bezier(0.2, 0, 0, 1)` | Snappy UI (button press, panel open) |
| `--ease-calm` | `cubic-bezier(0.4, 0, 0.2, 1)` | Default, slightly slower |
| `--ease-orbital` | `cubic-bezier(0.65, 0, 0.35, 1)` | Loops (status pulse, live dot) |
| `--ease-fade` | `cubic-bezier(0.4, 0, 0.6, 1)` | In/out fades |

### 7.2 Durations

| Token | Value | Use |
|---|---|---|
| `--dur-instant` | 80ms | Hover state, button press feedback |
| `--dur-quick` | 160ms | Tooltip appear, panel toggle |
| `--dur-base` | 240ms | Modal open, dropdown |
| `--dur-loop` | 1200ms | Status pulse, live dot, "thinking" indicator |
| `--dur-long` | 1800ms | Splash fade, first-run welcome |
| `--dur-lap` | 2400ms | Border-beam perimeter lap (REVAMP_SPEC §8.3; exactly 2 × `--dur-loop`) |

**No duration under 80ms.** No duration over 2000ms — the one exception is `--dur-lap` (2400ms), the owner-authorized border-beam perimeter lap (REVAMP_SPEC §11 bend 6a: at 824px of composer perimeter a 1200ms lap reads hurried, and this cap forbids tuning past 2000ms without a token). Every other duration stays ≤ 2000ms.

### 7.3 Animation vocabulary (the 8 glyphs)

Each animation has a name, a curve, a duration, and a use.

#### `orbit`

A small element traces a slow elliptical path. Used for: the splash logo, the "connecting" status.

```css
@keyframes orbit {
  0%   { transform: translate(0, 0); }
  25%  { transform: translate(8px, -4px); }
  50%  { transform: translate(0, -8px); }
  75%  { transform: translate(-8px, -4px); }
  100% { transform: translate(0, 0); }
}
animation: orbit var(--dur-loop) var(--ease-orbital) infinite;
```

#### `pulse`

A scale + opacity pulse, 1.2s loop. Used for: live status dot, "streaming" indicator, "thinking" cursor.

```css
@keyframes pulse {
  0%, 100% { transform: scale(1); opacity: 1; }
  50%      { transform: scale(1.08); opacity: 0.7; }
}
animation: pulse var(--dur-loop) var(--ease-orbital) infinite;
```

#### `spark`

A one-shot scale + fade. Used for: a saved indicator that pops and disappears, a successful key connect.

```css
@keyframes spark {
  0%   { transform: scale(0.6); opacity: 0; }
  40%  { transform: scale(1.1); opacity: 1; }
  100% { transform: scale(1); opacity: 0; }
}
animation: spark var(--dur-long) var(--ease-precise) 1;
```

#### `wave`

A horizontal wave of three dots. Used for: chat "model is typing" indicator.

```css
@keyframes wave {
  0%, 60%, 100% { transform: translateY(0); opacity: 0.5; }
  30%           { transform: translateY(-3px); opacity: 1; }
}
animation: wave 1200ms var(--ease-orbital) infinite;
```

#### `spin`

A continuous 360° rotation. Used for: indeterminate progress (LSP indexing), loading spinners.

```css
@keyframes spin {
  to { transform: rotate(360deg); }
}
animation: spin var(--dur-loop) linear infinite;
```

#### `fade`

A simple opacity in/out. Used for: ghost text appearance, modal backdrop, panel collapse.

```css
@keyframes fade {
  from { opacity: 0; }
  to   { opacity: 1; }
}
animation: fade var(--dur-base) var(--ease-fade);
```

#### `bloom`

A scale-in from 0.96 to 1.0 with fade. Used for: command palette open, modal open.

```css
@keyframes bloom {
  from { transform: scale(0.96); opacity: 0; }
  to   { transform: scale(1); opacity: 1; }
}
animation: bloom var(--dur-base) var(--ease-precise);
```

#### `comet`

A bright trailing line that moves across, fades to nothing. Used for: ghost text appearance (a 200ms streak across the suggestion).

```css
@keyframes comet {
  0%   { transform: translateX(-100%); opacity: 0; }
  20%  { opacity: 1; }
  100% { transform: translateX(0%); opacity: 0; }
}
animation: comet var(--dur-base) var(--ease-precise);
```

### 7.4 Reduce-motion respect

A user setting `ui.reduce_motion` (default off) disables `orbit`, `pulse`, `wave`, `spin`, and `comet`. `fade` and `bloom` are kept (they are short and essential for affordance). `spark` becomes a static flash.

---

## 8. Glyph set (icons)

We do not ship an icon font. We ship an SVG sprite (8 KB) with a curated set of 24 icons, each a 16×16 monochrome stroke.

| Icon name | Glyph | Use |
|---|---|---|
| `file` | sheet | Generic file in tree |
| `folder` | folder | Folder in tree |
| `folder-open` | folder open | Expanded folder |
| `chevron-right` | › | Tree row disclosure |
| `chevron-down` | v | Tree row disclosure (open) |
| `search` | magnifier | Fuzzy find, search-in-files |
| `command` | ⌘ | Command palette hint |
| `chat` | speech bubble | Chat sidebar toggle |
| `model` | stacked layers | Model picker |
| `git-branch` | branch | Git panel |
| `git-modified` | dot | Modified file |
| `git-added` | plus | Added file |
| `git-deleted` | x | Deleted file |
| `error` | triangle ! | Error badge |
| `warn` | triangle ! | Warning badge |
| `info` | i | Info badge |
| `connected` | dot | Live status (color = `--ok`) |
| `disconnected` | dashed dot | Disconnected status |
| `close` | x | Modal close, tab close |
| `check` | v | Save / confirm |
| `arrow-up` | ↑ | Send chat |
| `arrow-down` | ↓ | Receive / scroll down |
| `plus` | + | New file, add |
| `settings` | gear | Settings cog |

All icons use `currentColor` (inherit from text color), 1.5px stroke, no fill.

---

## 9. Brand mark

The Selora orbital planet mark:
- White core orb (8px diameter at 32px scale).
- Tilted ring (16px × 4px ellipse, rotated 20°).
- Satellite (2px dot at the ring's far end).

Used:
- In the title bar (16px size, left side, 8px from window edge).
- In the splash (64px size, centered).
- In the installer header (32px size).
- In the about dialog (48px size).

Construction is in `assets/icons/selora-ide-mono.svg`. Build agents render the mark in code from this source (do not import as a raster in the IDE chrome).

---

## 10. Status colors per state (canonical mapping)

| State | Foreground | Background | Border |
|---|---|---|---|
| Idle | `--fg-muted` | transparent | none |
| Hover | `--fg-default` | `--ink-faint` | none |
| Active / pressed | `--fg-default` | `--ink-low` | none |
| Focused | `--fg-default` | `--ink-low` | `--border-focus` |
| Disabled | `--fg-disabled` | transparent | none |
| Selected (list row) | `--fg-default` | `--ink-low` | none |
| Error | `--err` | transparent | `--border-error` |
| Success / Live | `--ok` | transparent | none |

A "live" indicator (e.g. Selora connected) uses `--ok` for both the dot color and any text label.

---

## 11. Theme file (Rust)

This section is the **contract** for `crates/selora-ui/src/theme.rs`. The build agent must produce a file that exposes every token above as a typed constant.

```rust
//! Selora IDE design tokens.
//! Single source of truth — every UI component reads from here.

use egui::Color32;

pub mod color {
    use super::Color32;

    pub const INK_CANVAS: Color32  = Color32::from_rgb(0, 0, 0);
    pub const INK_FAINT: Color32   = Color32::from_rgb(10, 10, 10);
    pub const INK_LOW: Color32     = Color32::from_rgb(20, 20, 20);
    pub const INK_MID: Color32     = Color32::from_rgb(31, 31, 31);
    pub const INK_HI: Color32      = Color32::from_rgb(42, 42, 42);
    pub const INK_RULE: Color32    = Color32::from_rgb(58, 58, 58);

    pub const FG_DEFAULT: Color32  = Color32::from_rgb(229, 229, 229);
    pub const FG_MUTED: Color32    = Color32::from_rgb(138, 138, 138);
    pub const FG_FAINT: Color32    = Color32::from_rgb(90, 90, 90);
    pub const FG_DISABLED: Color32 = Color32::from_rgb(58, 58, 58);

    pub const OK: Color32          = Color32::from_rgb(34, 197, 94);
    pub const WARN: Color32        = Color32::from_rgb(212, 160, 23);
    pub const ERR: Color32         = Color32::from_rgb(220, 38, 38);
    pub const INFO: Color32        = Color32::from_rgb(107, 114, 128);

    // Syntax
    pub const SYN_DEFAULT: Color32 = FG_DEFAULT;
    pub const SYN_KEYWORD: Color32 = Color32::from_rgb(207, 207, 207);
    pub const SYN_FN: Color32      = Color32::WHITE;
    pub const SYN_TYPE: Color32    = Color32::from_rgb(184, 184, 184);
    pub const SYN_STRING: Color32  = Color32::from_rgb(154, 154, 154);
    pub const SYN_NUMBER: Color32  = Color32::from_rgb(208, 208, 208);
    pub const SYN_COMMENT: Color32 = FG_FAINT;
    pub const SYN_PUNCT: Color32   = FG_MUTED;
    pub const SYN_ATTR: Color32    = Color32::from_rgb(168, 168, 168);
}

pub mod space {
    pub const S0: f32 = 0.0;
    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 24.0;
    pub const S6: f32 = 32.0;
}

pub mod radius {
    pub const R0: f32 = 0.0;
    pub const R1: f32 = 2.0;
    pub const R2: f32 = 4.0;
    pub const PILL: f32 = 999.0;
}

pub mod type_scale {
    pub const XS: f32 = 11.0;
    pub const SM: f32 = 12.0;
    pub const MD: f32 = 13.0;
    pub const LG: f32 = 15.0;
    pub const XL: f32 = 18.0;
    pub const XXL: f32 = 24.0;
}

pub mod font {
    pub const SANS: &str = "Space Grotesk";
    pub const MONO: &str = "JetBrains Mono";
}

pub mod motion {
    pub const DUR_INSTANT_MS: u32 = 80;
    pub const DUR_QUICK_MS: u32   = 160;
    pub const DUR_BASE_MS: u32    = 240;
    pub const DUR_LOOP_MS: u32    = 1200;
    pub const DUR_LONG_MS: u32    = 1800;
    // §7.2's one >2000ms exception: border-beam perimeter lap (bend §11.6a).
    pub const DUR_LAP_MS: u32     = 2400;

    // easing curves as tuples for egui / manual interpolation
    pub const EASE_PRECISE: (f32, f32, f32, f32) = (0.2, 0.0, 0.0, 1.0);
    pub const EASE_CALM:    (f32, f32, f32, f32) = (0.4, 0.0, 0.2, 1.0);
    pub const EASE_ORBITAL: (f32, f32, f32, f32) = (0.65, 0.0, 0.35, 1.0);
    pub const EASE_FADE:    (f32, f32, f32, f32) = (0.4, 0.0, 0.6, 1.0);
}
```

The `theme` module also exposes a `set_fonts(ctx: &egui::Context)` function that registers the bundled Space Grotesk + JetBrains Mono with egui, called once on app boot.

---

## 12. The no-decoration rule

If a UI decision could be answered "is this decorative?" with "yes", the answer is **no, don't do it**. The Selora IDE looks like an instrument panel, not a magazine cover. Examples of what is NOT allowed:

- No gradient backgrounds.
- No box shadows (except where §6 carves an exception).
- No saturated colors on hover.
- No animation that loops faster than 1.2s.
- No icons that aren't 16×16 monochrome stroke.
- No bold (weight 700).
- No font sizes below 11px.
- No italics in UI (syntax comments and empty-state placeholders only).
- No emoji in the chrome.
- No background images anywhere.
- No "splash particles" / confetti.
- No rainbow anything.

The discipline is what makes Selora IDE feel like Selora.
