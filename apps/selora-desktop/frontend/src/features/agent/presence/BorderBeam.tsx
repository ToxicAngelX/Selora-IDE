// Selora IDE — Libraries.dev BorderBeam Port (REVAMP_SPEC §8, PHASE_Z §8.1)
//
// An animated stepped-alpha glow that rides the border of the composer card.
// Monochromatic: stepped alpha on FG_DEFAULT ([0.85, 0.50, 0.28, 0.14]).
// Lap duration: 2400ms (DUR_LAP_MS) with orbital easing.
// Reduced-motion: static segment at the top-left corner.
import React from 'react';

export type BorderBeamProps = {
  active?: boolean;
  strength?: number;
  durationMs?: number;
  className?: string;
  borderRadius?: number;
};

export const BorderBeam: React.FC<BorderBeamProps> = ({
  active = true,
  strength = 1.0,
  durationMs = 2400,
  className = '',
  borderRadius = 8,
}) => {
  if (!active && strength <= 0) return null;

  return (
    <div
      className={`border-beam-container ${active ? 'is-active' : 'is-paused'} ${className}`}
      style={
        {
          borderRadius: `${borderRadius}px`,
          '--beam-duration': `${durationMs}ms`,
          '--beam-strength': strength,
        } as React.CSSProperties
      }
      aria-hidden="true"
    >
      <div className="border-beam-glow" />
    </div>
  );
};
