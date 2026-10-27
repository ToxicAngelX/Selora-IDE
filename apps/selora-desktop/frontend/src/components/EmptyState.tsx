// Selora IDE — Phase Z2 empty states (DESIGN §6).
//
// Tone rules: state what is true, then offer the one next action; heading
// --type-lg --fg-default; hint --type-md --fg-muted, italic for
// placeholders; at most two actions; keybind hints as chips.
import type { ReactNode } from 'react';

export function EmptyState({
  heading,
  hint,
  children,
  mark,
}: {
  heading?: string;
  hint?: ReactNode;
  children?: ReactNode;
  /** The 64px splash mark shown above the editor's no-workspace state. */
  mark?: ReactNode;
}) {
  return (
    <div className="empty-state">
      {mark}
      {heading ? <h3 className="empty-heading">{heading}</h3> : null}
      {hint ? <p className="empty-hint">{hint}</p> : null}
      {children ? <div className="empty-actions">{children}</div> : null}
    </div>
  );
}

/** UI_SPEC §12's keybind chip: --ink-mid, --radius-1, --type-xs, mono. */
export function KeyChip({ children }: { children: ReactNode }) {
  return <kbd className="key-chip">{children}</kbd>;
}

/** UI_SPEC §12's action button: --ink-mid fill, 32px tall, --radius-1. */
export function EmptyStateButton({
  children,
  disabled,
  title,
  onClick,
}: {
  children: ReactNode;
  disabled?: boolean;
  title?: string;
  onClick?: () => void;
}) {
  return (
    <button type="button" className="empty-action" disabled={disabled} title={title} onClick={onClick}>
      {children}
    </button>
  );
}
