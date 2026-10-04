# Contributing to Selora IDE

Thanks for your interest in Selora IDE.

## Ground rules

- Read `PLAN.md` first. It is the master plan; the non-goals in §2 are
  load-bearing — PRs that implement deferred features out of phase will be
  closed, not merged.
- `ARCHITECTURE.md` is the technical contract, `DESIGN_TOKENS.md` is the
  visual contract, `UI_SPEC.md` is the layout contract. Do not deviate from
  them; propose a change to the document first, then change the code.
- No color, font, or spacing value outside `DESIGN_TOKENS.md`. Everything
  comes from `crates/selora-ui/src/theme.rs`.
- No Zed or VS Code code, branding, or assets. We borrow architecture ideas,
  nothing else.
- Rust + egui only. No webview.

## Agent contributors

This project is built partly by AI agents. Agent contributors must follow
`AGENT_ORCHESTRATION.md` — phase briefs, acceptance gates, and the
orchestration rules live there. Humans writing code are also welcome to skim
it to understand how the project is structured.

## Building

```sh
cargo build --workspace
cargo run -p selora-app
cargo test --workspace
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

CI (`.github/workflows/ci.yml`) enforces fmt and clippy with warnings as
errors on `windows-latest`; run both locally before pushing.

## Commit style

Short, imperative subject lines (e.g. `feat(editor): gutter change markers`).
One logical change per commit.
