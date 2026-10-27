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
  | 'toast-warning'
  | 'plus'
  | 'split'
  | 'trash'
  | 'clear'
  | 'copy'
  | 'lock'
  | 'lock-open'
  | 'word-wrap'
  | 'chevron-down'
  | 'quick-fix'
  | 'severity-error'
  | 'severity-warning'
  | 'severity-info'
  | 'terminal'
  | 'play';

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
  plus: <path d="M8 3v10M3 8h10" />,
  split: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" rx="1" />
      <path d="M8 2.5v11" />
    </>
  ),
  trash: (
    <>
      <path d="M3 4.5h10M6 4.5V3h4v1.5M4.5 4.5v8.5a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1V4.5" />
      <path d="M6.5 7.5v4M9.5 7.5v4" />
    </>
  ),
  clear: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M4.5 11.5 11.5 4.5" />
    </>
  ),
  copy: (
    <>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1" />
      <path d="M3.5 10.5h-1a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1h7a1 1 0 0 1 1 1v1" />
    </>
  ),
  lock: (
    <>
      <rect x="3.5" y="7" width="9" height="7" rx="1" />
      <path d="M5.5 7V4.5a2.5 2.5 0 0 1 5 0V7" />
    </>
  ),
  'lock-open': (
    <>
      <rect x="3.5" y="7" width="9" height="7" rx="1" />
      <path d="M5.5 7V4.5a2.5 2.5 0 0 1 5 0" />
    </>
  ),
  'word-wrap': (
    <>
      <path d="M2.5 4.5h11M2.5 8.5h8a2 2 0 0 1 0 4H7.5" />
      <path d="M9 10.5 7 12.5l2 2" />
    </>
  ),
  'chevron-down': <path d="M4 6.5l4 4 4-4" />,
  'quick-fix': (
    <>
      <path d="M8 2v3M8 11v3M2 8h3M11 8h3" />
      <circle cx="8" cy="8" r="1.5" fill="currentColor" />
    </>
  ),
  'severity-error': (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M5.5 5.5l5 5M10.5 5.5l-5 5" />
    </>
  ),
  'severity-warning': (
    <>
      <path d="M8 2.5L2 13.5h12L8 2.5z" />
      <path d="M8 6.5v3.5" />
      <circle cx="8" cy="11.75" r="0.75" fill="currentColor" stroke="none" />
    </>
  ),
  'severity-info': (
    <>
      <circle cx="8" cy="8" r="6" />
      <circle cx="8" cy="5.25" r="0.75" fill="currentColor" stroke="none" />
      <path d="M8 7.5v4" />
    </>
  ),
  terminal: (
    <>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M5 6.5l2 1.5-2 1.5M9 9.5h2" />
    </>
  ),
  play: <path d="M5.5 3.5v9l7-4.5z" />,
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
