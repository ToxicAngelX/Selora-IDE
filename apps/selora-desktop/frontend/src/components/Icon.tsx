// Selora IDE — Phase Z2 icon set.
//
// Inline SVG per icon (DESIGN §10): 16×16 viewBox, no fill, `currentColor`,
// 1.5px stroke, square caps, miter joins. Paths are ported from the egui
// sprite set (crates/selora-ui/src/sidebar/icons.rs) — ported, not redrawn.
// The Z2 subset is ARCHITECTURE §9's (file, search, git-branch, error,
// settings, chat, close, check, command) plus the three window-control
// glyphs DESIGN §3.1 adds, plus the toast severity glyphs UI_SPEC §13
// specifies ('i' / '!') drawn in the same stroke system.
import type { ReactNode } from 'react';

export type IconName =
  | 'file'
  | 'search'
  | 'git-branch'
  | 'error'
  | 'settings'
  | 'chat'
  | 'close'
  | 'check'
  | 'command'
  | 'win-min'
  | 'win-max'
  | 'win-restore'
  | 'toast-info'
  | 'toast-warning';

const ICON_PATHS: Record<IconName, ReactNode> = {
  // paint_sheet (icons.rs:53): a sheet with a folded corner.
  file: (
    <>
      <path d="M3.5 1.5v13h9V4.5L9.5 1.5H3.5z" />
      <path d="M9.5 1.5v3h3" />
    </>
  ),
  // paint_search (icons.rs:107): a magnifier.
  search: (
    <>
      <circle cx="6.5" cy="6.5" r="3.5" />
      <path d="M9.3 9.3 13 13" />
    </>
  ),
  // paint_git_branch (icons.rs:120): trunk, two commits, branch head.
  'git-branch': (
    <>
      <circle cx="4.5" cy="4" r="1.75" />
      <circle cx="4.5" cy="12" r="1.75" />
      <circle cx="12" cy="4" r="1.75" />
      <path d="M4.5 6v4" />
      <path d="M4.5 8h4.5l2-2" />
    </>
  ),
  // paint_warning (icons.rs:150): the problems activity's triangle.
  error: (
    <>
      <path d="M8 2 2 14h12L8 2z" />
      <circle cx="8" cy="10" r="1.25" fill="currentColor" stroke="none" />
    </>
  ),
  // A gear: center bore plus eight radial teeth (authored in the same
  // 16×16 / 1.5px discipline; the egui strip has no settings glyph).
  settings: (
    <>
      <circle cx="8" cy="8" r="3.25" />
      <path d="M13 8h1.75M11.5 11.5l1.24 1.24M8 13v1.75M4.5 11.5l-1.24 1.24M3 8H1.25M4.5 4.5 3.26 3.26M8 3V1.25M11.5 4.5l1.24-1.24" />
    </>
  ),
  // paint_chat (icons.rs:163): a speech bubble with a bottom-left tail.
  chat: (
    <>
      <rect x="1.5" y="2.5" width="13" height="8.5" rx="2.5" />
      <path d="M4.5 11 3.5 15l3.5-4" />
    </>
  ),
  // paint_x (panel/search.rs:731): the × glyph.
  close: <path d="M4.5 4.5 11.5 11.5M4.5 11.5 11.5 4.5" />,
  check: <path d="M3 8.5 6.5 12 13 4" />,
  // A terminal prompt: chevron + underscore.
  command: <path d="M5 4.5 9 8l-4 3.5M10.5 11.5H13" />,
  // Window controls (DESIGN §3.1): line, square, offset squares.
  'win-min': <path d="M4 8h8" />,
  'win-max': <rect x="4" y="4" width="8" height="8" />,
  'win-restore': (
    <>
      <rect x="6.5" y="3.5" width="6" height="6" />
      <path d="M3.5 9.5h6v3h-6z" />
    </>
  ),
  // Toast severity glyphs (UI_SPEC §13): 'i' and '!', 16×16 monochrome.
  'toast-info': (
    <>
      <circle cx="8" cy="3.5" r="1.25" fill="currentColor" stroke="none" />
      <path d="M8 6v6.5" />
    </>
  ),
  'toast-warning': (
    <>
      <path d="M8 3.5v7" />
      <circle cx="8" cy="12.75" r="1.25" fill="currentColor" stroke="none" />
    </>
  ),
};

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="square"
      strokeLinejoin="miter"
      aria-hidden="true"
      focusable="false"
    >
      {ICON_PATHS[name]}
    </svg>
  );
}
