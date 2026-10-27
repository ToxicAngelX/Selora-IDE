// Selora IDE — Libraries.dev BotAvatar Port (PHASE_Z §8.3, ZCode dynamic workflow parity)
//
// 16–24px agent identity glyph reflecting Selora's Monochrome Space Design System (E:\Projects\Selora).
// Features:
// - Official living logo-mark orbital planet for the Orchestrator/Core Agent (tilted -18deg ring, orbiting satellite, calm blinking eyes)
// - Specialized persona antennae for subagents:
//   - Frontend Developer: < > angle bracket crown
//   - Rust Developer: > _ terminal prompt / gear crest
//   - UI Reviewer: Monocle scanner lens
//   - Test Runner: Check-shield antenna
// - Dynamic live states: default, working (pulsing --ok beacon), sleeping (gentle horizontal visor).
import React from 'react';

export type BotAvatarStatus = 'default' | 'working' | 'sleeping';

export type BotAvatarRole =
  | 'orchestrator'
  | 'frontend-developer'
  | 'rust-developer'
  | 'ui-reviewer'
  | 'test-runner'
  | string;

export type BotAvatarProps = {
  status?: BotAvatarStatus;
  role?: BotAvatarRole;
  size?: number;
  className?: string;
  name?: string;
};

export const BotAvatar: React.FC<BotAvatarProps> = ({
  status = 'default',
  role = 'default',
  size = 20,
  className = '',
  name = 'Selora Agent',
}) => {
  const normRole = role.toLowerCase();
  const isFrontend = normRole.includes('frontend') || normRole.includes('ui');
  const isRust = normRole.includes('rust') || normRole.includes('backend');
  const isReviewer = normRole.includes('review') || normRole.includes('audit');
  const isTest = normRole.includes('test') || normRole.includes('qa');
  const isOrchestrator = normRole.includes('orchestrator') || normRole.includes('lead') || normRole.includes('selora');
  const isOrbitalPlanet = isOrchestrator || (!isFrontend && !isRust && !isReviewer && !isTest);

  const gradId = `selora-avatar-grad-${size}-${normRole.replace(/[^a-z0-9]/g, '-')}`;

  return (
    <div
      className={`bot-avatar-container avatar-${status} role-${normRole.replace(/[^a-z0-9]/g, '-')} ${className}`}
      style={{ width: `${size}px`, height: `${size}px` }}
      title={`${name} [${role}] (${status})`}
      role="img"
      aria-label={`${name} avatar (${role}): ${status}`}
    >
      <svg
        width={size}
        height={size}
        viewBox="0 0 24 24"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className="bot-avatar-svg"
      >
        <defs>
          <radialGradient id={`${gradId}-core`} cx="0.38" cy="0.32" r="0.9">
            <stop offset="0%" stopColor="var(--fg-bright)" />
            <stop offset="70%" stopColor="var(--c-f5f6f8)" />
            <stop offset="100%" stopColor="var(--moonlight-400)" />
          </radialGradient>
          <linearGradient id={`${gradId}-ring`} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="var(--moonlight-200)" />
            <stop offset="55%" stopColor="var(--moonlight-400)" />
            <stop offset="100%" stopColor="var(--moonlight-700)" />
          </linearGradient>
        </defs>

        {isOrbitalPlanet ? (
          // --- Official Selora Living Logo-Mark / Orbital Planet Companion ---
          <g className="selora-planet-mark">
            {/* Soft moonlight atmospheric halo */}
            <circle cx="12" cy="12" r="10" fill="var(--c-rgba-205-214-228-0p12)" className="selora-orb-halo" />

            {/* Tilted orbital ring (-18deg like logo-mark.svg) */}
            <ellipse
              cx="12"
              cy="12.5"
              rx="9"
              ry="3.8"
              fill="none"
              stroke={`url(#${gradId}-ring)`}
              strokeWidth="1.2"
              opacity="0.9"
              transform="rotate(-18 12 12.5)"
              className="selora-orb-ring"
            />

            {/* Satellite moon on the orbital ring */}
            <circle cx="18" cy="8.5" r="1.5" fill="var(--moonlight-300)" className="selora-orb-sat" />

            {/* Core planet star */}
            <circle
              cx="12"
              cy="12"
              r="6"
              fill={`url(#${gradId}-core)`}
              className="bot-avatar-body"
            />

            {/* Specular glint */}
            <circle cx="10" cy="10" r="1.4" fill="var(--moonlight-300)" opacity="0.8" />

            {/* Blinking eyes / expressions */}
            {status !== 'sleeping' ? (
              <g className="selora-eyes">
                <circle cx="10.2" cy="12.2" r="0.9" fill="var(--c-0b0b0f)" className="bot-avatar-eye" />
                <circle cx="13.8" cy="12.2" r="0.9" fill="var(--c-0b0b0f)" className="bot-avatar-eye" />
              </g>
            ) : (
              <line
                x1="9"
                y1="12.5"
                x2="15"
                y2="12.5"
                className="bot-avatar-sleep-line"
                stroke="var(--c-0b0b0f)"
                strokeWidth="1.2"
                strokeLinecap="round"
              />
            )}
          </g>
        ) : (
          // --- Specialized Subagent Robot Personas ---
          <>
            {/* Robot Head / Body silhouette */}
            <rect
              x="4"
              y="6"
              width="16"
              height="14"
              rx="4"
              className="bot-avatar-body"
            />

            {/* Specialized Persona Crown / Antenna */}
            {isFrontend ? (
              // Frontend: Code angle bracket crown < >
              <>
                <path d="M10 2L8 4L10 6" className="bot-avatar-stem" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
                <path d="M14 2L16 4L14 6" className="bot-avatar-stem" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
              </>
            ) : isRust ? (
              // Rust / Backend: Terminal prompt / gear antenna
              <>
                <line x1="12" y1="2" x2="12" y2="6" className="bot-avatar-stem" strokeWidth="1.5" />
                <rect x="10.5" y="1" width="3" height="3" rx="0.5" className="bot-avatar-tip" />
              </>
            ) : isReviewer ? (
              // Reviewer: Monocle / diamond eye crest
              <>
                <polygon points="12,1 14.5,3.5 12,6 9.5,3.5" className="bot-avatar-tip" />
              </>
            ) : isTest ? (
              // Test Runner: Check shield antenna
              <>
                <path d="M10 3L11.5 4.5L14 2" className="bot-avatar-stem" strokeWidth="1.5" strokeLinecap="round" />
                <line x1="12" y1="4.5" x2="12" y2="6" className="bot-avatar-stem" strokeWidth="1.5" />
              </>
            ) : null}

            {/* Visor / Eye slot */}
            {status !== 'sleeping' ? (
              <>
                {isReviewer ? (
                  // Reviewer: Monocle scanner lens on left, standard eye on right
                  <>
                    <circle cx="9" cy="12" r="2.2" className="bot-avatar-monocle" stroke="var(--fg-default)" strokeWidth="1" fill="none" />
                    <circle cx="9" cy="12" r="1" className="bot-avatar-eye" />
                    <circle cx="15" cy="12" r="1.3" className="bot-avatar-eye" />
                  </>
                ) : isRust ? (
                  // Rust: Terminal prompt look (> _)
                  <>
                    <path d="M8 11L10 12L8 13" className="bot-avatar-stem" strokeWidth="1.2" strokeLinecap="round" />
                    <line x1="13" y1="13" x2="16" y2="13" className="bot-avatar-stem" strokeWidth="1.2" />
                  </>
                ) : (
                  // Dual alert LED eyes
                  <>
                    <circle cx="9" cy="12" r="1.5" className="bot-avatar-eye" />
                    <circle cx="15" cy="12" r="1.5" className="bot-avatar-eye" />
                  </>
                )}
              </>
            ) : (
              // Sleeping mode: gentle slit eyes
              <line
                x1="8"
                y1="13"
                x2="16"
                y2="13"
                className="bot-avatar-sleep-line"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            )}
          </>
        )}

        {/* Working status indicator dot */}
        {status === 'working' && (
          <circle cx="19" cy="5" r="1.8" className="bot-avatar-live-indicator" fill="var(--ok)" />
        )}
      </svg>
    </div>
  );
};
