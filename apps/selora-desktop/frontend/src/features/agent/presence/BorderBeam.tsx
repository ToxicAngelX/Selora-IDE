// Selora IDE — Elevated BorderBeam (REVAMP_SPEC §8, PHASE_Z §8.1)
//
// An animated stepped-alpha glow that travels along the composer card perimeter.
// Elevated with dual-beam depth (crisp hairline highlight + soft atmospheric glow),
// high-DPI mask rendering, orbital easing, and reduced-motion compliance.
import React from 'react';

export type BorderBeamProps = {
  active?: boolean;
  strength?: number;
  durationMs?: number;
  className?: string;
  borderRadius?: number;
  colorPreset?: 'emerald' | 'monochrome' | 'aurora';
};

export const BorderBeam: React.FC<BorderBeamProps> = ({
  active = true,
  strength = 1.0,
  durationMs = 2400,
  className = '',
  borderRadius = 8,
  colorPreset = 'emerald',
}) => {
  if (!active && strength <= 0) return null;

  return (
    <div
      className={`border-beam-container ${active ? 'is-active' : 'is-paused'} preset-${colorPreset} ${className}`}
      style={
        {
          borderRadius: `${borderRadius}px`,
          '--beam-duration': `${durationMs}ms`,
          '--beam-strength': strength,
        } as React.CSSProperties
      }
      aria-hidden="true"
    >
      {/* 1. Primary crisp beam */}
      <div className="border-beam-glow" />
      {/* 2. Secondary soft atmospheric aura */}
      <div className="border-beam-aura" />
    </div>
  );
};
