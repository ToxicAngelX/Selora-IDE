// Selora IDE — Libraries.dev ThinkingOrb Port (PHASE_Z §8.2, KIMI_UI_PROMPT §1)
//
// Nine hand-tuned animated states reflecting Selora's Monochrome Space Design System (E:\Projects\Selora).
// Sizes: 20px (inline status) and 64px (welcome hero).
// Celestial materials: radial core (#FFFFFF -> #F5F6F8 -> #CDD6E4), tilted orbital ring (-18deg),
// moonlight halo aura, and semantic --ok liveness dot.
import React from 'react';
import type { ThinkingState } from '../types';

export type ThinkingOrbProps = {
  state?: ThinkingState;
  size?: 20 | 64 | number;
  paused?: boolean;
  isLive?: boolean;
  className?: string;
};

export const ThinkingOrb: React.FC<ThinkingOrbProps> = ({
  state = 'breathing',
  size = 20,
  paused = false,
  isLive = true,
  className = '',
}) => {
  const isHero = size >= 48;
  const coreRadius = isHero ? 14 : 4.5;
  const orbitRadius = isHero ? 24 : 7.5;
  const center = size / 2;

  return (
    <div
      className={`thinking-orb orb-${state} ${isHero ? 'orb-hero' : 'orb-inline'} ${paused ? 'is-paused' : ''} ${className}`}
      style={{ width: `${size}px`, height: `${size}px` }}
      role="img"
      aria-label={`Agent state: ${state}`}
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        className="thinking-orb-svg"
      >
        <defs>
          <radialGradient id={`orb-core-grad-${size}`} cx="0.38" cy="0.32" r="0.9">
            <stop offset="0%" stopColor="#FFFFFF" />
            <stop offset="70%" stopColor="#F5F6F8" />
            <stop offset="100%" stopColor="#CDD6E4" />
          </radialGradient>
          <linearGradient id={`orb-ring-grad-${size}`} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#F2F5FA" />
            <stop offset="55%" stopColor="#CDD6E4" />
            <stop offset="100%" stopColor="#8B9AB5" />
          </linearGradient>
        </defs>

        {/* Moonlight atmospheric halo */}
        <circle
          cx={center}
          cy={center}
          r={orbitRadius + (isHero ? 7 : 2.5)}
          fill="rgba(205, 214, 228, 0.08)"
          className="selora-orb-halo"
        />

        {/* Tilted orbital track ring (hero mode & complex states: -18deg per Selora mark) */}
        {isHero && (
          <ellipse
            cx={center}
            cy={center}
            rx={orbitRadius}
            ry={orbitRadius * 0.48}
            fill="none"
            stroke={`url(#orb-ring-grad-${size})`}
            strokeWidth={1.5}
            opacity={0.8}
            transform={`rotate(-18 ${center} ${center})`}
            className="orb-track"
          />
        )}

        {/* Core celestial sphere */}
        <circle
          cx={center}
          cy={center}
          r={coreRadius}
          fill={`url(#orb-core-grad-${size})`}
          className="orb-core"
        />

        {/* Specular glint (hero mode) */}
        {isHero && (
          <circle
            cx={center - coreRadius * 0.32}
            cy={center - coreRadius * 0.32}
            r={coreRadius * 0.25}
            fill="#E4E9F2"
            opacity={0.85}
            className="orb-glint"
          />
        )}

        {/* Satellite 1 (orbital traveler) */}
        <g className="orb-satellite-group satellite-1">
          <circle
            cx={center + orbitRadius}
            cy={center}
            r={isHero ? 3.5 : 1.5}
            className="orb-satellite"
          />
        </g>

        {/* Satellite 2 (counter traveler for complex states) */}
        {(state === 'working' || state === 'weaving' || state === 'solving') && (
          <g className="orb-satellite-group satellite-2">
            <circle
              cx={center - orbitRadius}
              cy={center}
              r={isHero ? 2.8 : 1.2}
              className="orb-satellite secondary"
            />
          </g>
        )}

        {/* Semantic --ok liveness dot */}
        {isLive && isHero && (
          <circle
            cx={center + orbitRadius * 0.707}
            cy={center - orbitRadius * 0.707}
            r={2}
            className="orb-liveness-dot"
          />
        )}
      </svg>
    </div>
  );
};
