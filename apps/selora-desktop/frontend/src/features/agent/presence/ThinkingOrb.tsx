// Selora IDE — Elevated ThinkingOrb (PHASE_Z §8.2, KIMI_UI_PROMPT §1)
//
// Nine hand-tuned animated celestial states reflecting Selora's Monochrome Space Design System.
// Sizes: 20px (inline status) and 64px (welcome hero).
// High-DPI geometric precision, fluid orbital ring pulse, atmospheric moonlight halo,
// multi-satellite orbital paths, and strict reduced-motion compliance.
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
  const filterId = `orb-glow-${size}`;
  const coreGradId = `orb-core-grad-${size}`;
  const ringGradId = `orb-ring-grad-${size}`;
  const pulseGradId = `orb-pulse-grad-${size}`;

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
        shapeRendering="geometricPrecision"
      >
        <defs>
          {/* High-DPI Celestial Core Gradient */}
          <radialGradient id={coreGradId} cx="0.38" cy="0.32" r="0.9">
            <stop offset="0%" stopColor="var(--fg-bright)" />
            <stop offset="65%" stopColor="var(--c-f5f6f8)" />
            <stop offset="100%" stopColor="var(--moonlight-400)" />
          </radialGradient>

          {/* Tilted Orbital Track Gradient */}
          <linearGradient id={ringGradId} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--moonlight-200)" stopOpacity="0.95" />
            <stop offset="50%" stopColor="var(--moonlight-400)" stopOpacity="0.6" />
            <stop offset="100%" stopColor="var(--moonlight-700)" stopOpacity="0.35" />
          </linearGradient>

          {/* Fluid Ring Pulse Gradient */}
          <radialGradient id={pulseGradId} cx="0.5" cy="0.5" r="0.5">
            <stop offset="70%" stopColor="var(--moonlight-400)" stopOpacity="0.4" />
            <stop offset="100%" stopColor="var(--moonlight-700)" stopOpacity="0" />
          </radialGradient>

          {/* Soft high-DPI glow filter */}
          <filter id={filterId} x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation={isHero ? "1.5" : "0.75"} result="glow" />
            <feComposite in="SourceGraphic" in2="glow" operator="over" />
          </filter>
        </defs>

        {/* 1. Moonlight atmospheric halo */}
        <circle
          cx={center}
          cy={center}
          r={orbitRadius + (isHero ? 7 : 2.5)}
          fill="var(--c-rgba-205-214-228-0p08)"
          className="selora-orb-halo"
        />

        {/* 2. Fluid Orbital Ring Pulse (hero and complex states) */}
        {isHero && (
          <ellipse
            cx={center}
            cy={center}
            rx={orbitRadius + 2}
            ry={(orbitRadius + 2) * 0.48}
            fill="none"
            stroke={`url(#${pulseGradId})`}
            strokeWidth={1.8}
            transform={`rotate(-18 ${center} ${center})`}
            className="orb-pulse-ring"
          />
        )}

        {/* 3. Tilted orbital track ring (-18deg per Selora mark) */}
        {isHero && (
          <ellipse
            cx={center}
            cy={center}
            rx={orbitRadius}
            ry={orbitRadius * 0.48}
            fill="none"
            stroke={`url(#${ringGradId})`}
            strokeWidth={1.5}
            strokeDasharray={state === 'working' || state === 'searching' ? '3 3' : undefined}
            opacity={0.85}
            transform={`rotate(-18 ${center} ${center})`}
            className="orb-track"
          />
        )}

        {/* 4. Core celestial sphere */}
        <circle
          cx={center}
          cy={center}
          r={coreRadius}
          fill={`url(#${coreGradId})`}
          className="orb-core"
          filter={`url(#${filterId})`}
        />

        {/* 5. Specular glint (hero mode) */}
        {isHero && (
          <circle
            cx={center - coreRadius * 0.32}
            cy={center - coreRadius * 0.32}
            r={coreRadius * 0.25}
            fill="var(--fg-bright)"
            opacity={0.9}
            className="orb-glint"
          />
        )}

        {/* 6. Satellite 1 (primary orbital traveler) */}
        <g className="orb-satellite-group satellite-1">
          <circle
            cx={center + orbitRadius}
            cy={center}
            r={isHero ? 3.5 : 1.5}
            className="orb-satellite"
          />
        </g>

        {/* 7. Satellite 2 (counter traveler for complex states) */}
        {(state === 'working' || state === 'weaving' || state === 'solving' || state === 'searching') && (
          <g className="orb-satellite-group satellite-2">
            <circle
              cx={center - orbitRadius}
              cy={center}
              r={isHero ? 2.8 : 1.2}
              className="orb-satellite secondary"
            />
          </g>
        )}

        {/* 8. Satellite 3 (radial shimmer for weaving & solving) */}
        {isHero && (state === 'weaving' || state === 'solving') && (
          <g className="orb-satellite-group satellite-3">
            <circle
              cx={center}
              cy={center - orbitRadius * 0.48}
              r={2}
              className="orb-satellite tertiary"
            />
          </g>
        )}

        {/* 9. Semantic --ok liveness dot */}
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
