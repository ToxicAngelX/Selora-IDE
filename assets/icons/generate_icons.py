# Generate selora-ide.ico / selora-ide-256.png / selora-ide-512.png from the
# brand mark's construction (assets/icons/selora-ide-mono.svg is the source of
# truth; this script replicates it exactly — ellipse rx=8 ry=2 stroke 1
# rotated 20° clockwise, core r=4, satellite r=1 at the ring's far end — on a
# 4x-supersampled canvas, with no SVG dependency).
#
# Usage: python assets/icons/generate_icons.py
# (Pillow required; outputs land next to this script.)

from PIL import Image, ImageDraw
from pathlib import Path

REF = 32.0          # the SVG's reference size
SS = 4              # supersampling factor
HERE = Path(__file__).parent


def render(size: int) -> Image.Image:
    """Render the mark at `size` px, white on transparent."""
    s = size * SS
    k = s / REF  # reference -> supersampled canvas
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    # Tilted ring: draw the axis-aligned ellipse on its own layer, then
    # rotate the layer -20° (PIL rotates counterclockwise; SVG's rotate(20)
    # is clockwise in screen coordinates — the ring's right end dips down,
    # matching the satellite at (23.52, 18.74)).
    ring = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(ring)
    cx = cy = 16 * k
    rx, ry = 8 * k, 2 * k
    stroke = max(1, round(1 * k))
    d.ellipse([cx - rx, cy - ry, cx + rx, cy + ry],
              outline=(255, 255, 255, 255), width=stroke)
    ring = ring.rotate(-20, resample=Image.BICUBIC, center=(cx, cy))
    img.alpha_composite(ring)

    d = ImageDraw.Draw(img)
    # Core orb.
    r = 4 * k
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(255, 255, 255, 255))
    # Satellite at the ring's far end (local (8, 0) rotated 20° clockwise).
    d.ellipse([23.52 * k - 1 * k, 18.74 * k - 1 * k,
               23.52 * k + 1 * k, 18.74 * k + 1 * k], fill=(255, 255, 255, 255))

    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    png256 = render(256)
    png256.save(HERE / "selora-ide-256.png")
    render(512).save(HERE / "selora-ide-512.png")
    # The Windows icon: BMP entries up to 48, PNG-compressed 256 (PIL packs
    # both from one call).
    png256.save(
        HERE / "selora-ide.ico",
        sizes=[(16, 16), (32, 32), (48, 48), (256, 256)],
    )

    # The Inno Setup wizard art (RELEASE.md §2.2: the installer shows the
    # brand mark): the white mark on ink black. BMPs — Inno does not read
    # PNG wizard images.
    large = Image.new("RGB", (164, 314), (0, 0, 0))
    mark = render(128)
    large.paste(mark, ((164 - 128) // 2, (314 - 128) // 2), mark)
    large.save(HERE / "wizard_large.bmp")
    small = Image.new("RGB", (55, 58), (0, 0, 0))
    mark = render(48)
    small.paste(mark, ((55 - 48) // 2, (58 - 48) // 2), mark)
    small.save(HERE / "wizard_small.bmp")

    for name in (
        "selora-ide.ico",
        "selora-ide-256.png",
        "selora-ide-512.png",
        "wizard_large.bmp",
        "wizard_small.bmp",
    ):
        print("wrote", HERE / name)


if __name__ == "__main__":
    main()
