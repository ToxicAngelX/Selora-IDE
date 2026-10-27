## What

<!-- What does this PR do? One paragraph. -->

## Phase / plan reference

<!-- Which ROADMAP.md phase does this belong to? PRs that implement deferred
     features out of phase are closed, not merged (CONTRIBUTING.md). -->

## Gate

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`

## Design

- [ ] Every color, font, spacing, and motion value comes from
      `crates/selora-ui/src/theme.rs` (no hardcoded visual values)
- [ ] No secrets in files or logs (the API key lives in the OS keyring only)

## Notes for the reviewer

<!-- Deviations from the plan documents, risks, follow-ups. -->
