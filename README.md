# Selora IDE

**The editor born connected to Selora.**

Selora IDE is an open-source, native-feel code editor written in Rust. Its
chat, completions, and model switching are billed through
[api.selora.lol](https://api.selora.lol) — a first-class AI gateway, not an
afterthought. No API key pasted into a third-party panel, no extension
marketplace between you and the models.

The design language is the Selora brand: monochrome dark, hairline borders,
calm motion, functional green only. The editor pane is custom-rendered on top
of [egui](https://github.com/emilk/egui) — pure Rust, no webview.

> This repository currently contains the **Lite Edition** plan and its
> foundation (Phase 0). The full roadmap — editor core, LSP, git, Selora AI
> integration, installer — is in [`PLAN.md`](PLAN.md) and
> [`ROADMAP.md`](ROADMAP.md).

## Status

Phase 0 (foundation) is complete:

- Cargo workspace with all 9 crates (`crates/`).
- Design tokens implemented in code (`crates/selora-ui/src/theme.rs`, the
  typed mirror of `DESIGN_TOKENS.md`).
- `cargo run -p selora-app` opens a 1280x800 dark window titled "Selora IDE"
  with the brand mark on the left and an empty ink-canvas workbench.

Later phases arrive per [`ROADMAP.md`](ROADMAP.md).

## Building

Requires Rust stable (MSVC toolchain on Windows). See
[rustup](https://rustup.rs/).

```sh
cargo build --workspace
cargo run -p selora-app
cargo test --workspace
```

Before committing, also run:

```sh
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

Bundled fonts (Space Grotesk and JetBrains Mono, both SIL OFL 1.1) live in
`assets/fonts/` — nothing is downloaded at runtime.

## Project layout

| Path | What |
|---|---|
| `PLAN.md` | Master plan (read this first) |
| `ARCHITECTURE.md` | Technical contract |
| `DESIGN_TOKENS.md` | Visual contract (colors, type, spacing, motion) |
| `UI_SPEC.md` | Screen-by-screen layout contract |
| `ROADMAP.md` | Phase breakdown and exit gates |
| `AGENT_ORCHESTRATION.md` | Instructions for AI-agent contributors |
| `crates/` | The 9 workspace crates (see `PLAN.md` §5) |
| `assets/` | Bundled fonts and the brand mark source |

## License

MIT (see [LICENSE](LICENSE)). Bundled fonts are SIL OFL 1.1.
