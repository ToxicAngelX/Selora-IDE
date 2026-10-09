// The Selora orbital brand mark, rendered in code (DESIGN §9 — never a
// raster). Geometry ported from crates/selora-ui/src/brand.rs's 32px
// reference construction: 8px core orb, 16×4 ring tilted 20°, 2px satellite
// at the ring's far end (theta = 0 — the static pose), 1px ring stroke.
export function SeloraMark({ size }: { size: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" focusable="false">
      <ellipse
        cx="16"
        cy="16"
        rx="8"
        ry="2"
        transform="rotate(20 16 16)"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
      />
      <circle cx="16" cy="16" r="4" fill="currentColor" />
      <circle cx="23.52" cy="18.74" r="1" fill="currentColor" />
    </svg>
  );
}
