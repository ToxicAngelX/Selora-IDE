# icons — raster derivatives

`selora-ide-mono.svg` is the hand-authored source mark (see DESIGN_TOKENS.md
§9). The raster derivatives are generated from it, never edited directly.

Generated files (committed; regenerate with the script after any SVG change):

- `selora-ide.ico` — Windows app icon (16/32/48 BMP + 256 PNG entries), used
  by the installer (`installer/selora-ide.iss`) and available for the window
  exe.
- `selora-ide-256.png` — 256px raster.
- `selora-ide-512.png` — 512px raster.

## How they are generated

`generate_icons.py` (this directory) replicates the SVG's construction with
Pillow on a 4x-supersampled canvas — no SVG rasterizer needed:

```sh
python assets/icons/generate_icons.py
```

The IDE chrome itself does **not** import these rasters: the title-bar /
splash / about instances of the mark are painted in code
(`crates/selora-ui/src/brand.rs`) per DESIGN_TOKENS.md §9.
