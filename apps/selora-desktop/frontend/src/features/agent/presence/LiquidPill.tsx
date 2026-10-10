// Selora IDE — Libraries.dev LiquidPill Port (PHASE_Z §8.4, KIMI_UI_PROMPT §4)
//
// Fluid gooey-inspired mode toggle for binary states (e.g., Agent Mode, Plan Mode).
import React from 'react';

export type LiquidPillProps = {
  active: boolean;
  onToggle: () => void;
  label: string;
  activeLabel?: string;
  icon?: React.ReactNode;
  disabled?: boolean;
  className?: string;
};

export const LiquidPill: React.FC<LiquidPillProps> = ({
  active,
  onToggle,
  label,
  activeLabel,
  icon,
  disabled = false,
  className = '',
}) => {
  return (
    <button
      type="button"
      className={`liquid-pill ${active ? 'is-active' : 'is-idle'} ${disabled ? 'is-disabled' : ''} ${className}`}
      onClick={onToggle}
      disabled={disabled}
      aria-pressed={active}
    >
      <span className="liquid-pill-track">
        <span className="liquid-pill-thumb" />
      </span>
      {icon && <span className="liquid-pill-icon">{icon}</span>}
      <span className="liquid-pill-label">
        {active && activeLabel ? activeLabel : label}
      </span>
    </button>
  );
};
