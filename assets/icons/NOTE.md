# icons — raster derivatives

`selora-ide-mono.svg` is the hand-authored source mark (see DESIGN_TOKENS.md
§9). It is the only committed icon source; the raster derivatives are
generated from it, never edited directly.

Files expected here eventually (per PLAN.md §5):

- `selora-ide.ico` — Windows app icon (Phase 5, for the installer/window exe).
- `selora-ide-256.png` — 256px raster.
- `selora-ide-512.png` — 512px raster.

## How to generate them from the SVG

With [Inkscape](https://inkscape.org/) and [ImageMagick](https://imagemagick.org/)
installed:

```sh
# PNGs — export at 256 and 512 px (transparent background, white mark).
inkscape selora-ide-mono.svg --export-type=png \
  --export-filename=selora-ide-256.png --export-width=256 --export-height=256
inkscape selora-ide-mono.svg --export-type=png \
  --export-filename=selora-ide-512.png --export-width=512 --export-height=512

# .ico — Windows wants the icon black-on-transparent for dark mode, or
# white-on-dark. Export a dark mark variant, then pack sizes 16/32/48/256:
inkscape selora-ide-mono.svg --export-type=png \
  --export-filename=selora-ide-mark-dark.png --export-width=256 --export-height=256 \
  --export-background-opacity=0   # then recolor if needed
magick selora-ide-256.png -define icon:auto-resize=256,48,32,16 selora-ide.ico
```

Alternatively, [ImageMagick](https://imagemagick.org/) alone can rasterize the
SVG if it is built with an SVG delegate:

```sh
magick -background none selora-ide-mono.svg -resize 256x256 selora-ide-256.png
magick -background none selora-ide-mono.svg -resize 512x512 selora-ide-512.png
magick -background none -define icon:auto-resize=256,48,32,16 selora-ide-mono.svg selora-ide.ico
```

The IDE chrome itself does **not** import these rasters: the title-bar /
splash / about instances of the mark are painted in code
(`crates/selora-ui/src/brand.rs`) per DESIGN_TOKENS.md §9.
