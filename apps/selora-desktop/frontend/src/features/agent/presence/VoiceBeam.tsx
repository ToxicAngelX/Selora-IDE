// Selora IDE — Libraries.dev VoiceBeam Port (PHASE_Z §8.5, KIMI_UI_PROMPT §3)
//
// Sound-reactive style glow along the composer's bottom edge driven by
// streaming throughput (deltas/second or tool activity), NOT microphone input.
import React from 'react';

export type VoiceBeamProps = {
  active?: boolean;
  level?: number; // 0 to 1 activity level
  className?: string;
};

export const VoiceBeam: React.FC<VoiceBeamProps> = ({
  active = false,
  level = 0.5,
  className = '',
}) => {
  if (!active) return null;

  const clampedLevel = Math.max(0, Math.min(1, level));

  return (
    <div
      className={`voice-beam-container ${active ? 'is-active' : ''} ${className}`}
      aria-hidden="true"
    >
      <div
        className="voice-beam-glow"
        style={{
          opacity: 0.15 + clampedLevel * 0.4,
          transform: `scaleX(${0.3 + clampedLevel * 0.7})`,
        }}
      />
    </div>
  );
};
