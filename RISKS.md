# Selora IDE — Risks

This is a list of the things that can kill the project, ranked by impact and likelihood, with a concrete mitigation for each. The orchestrator reads this before every phase to check whether a known risk is materializing.

Risks are organized by category: technical, product, agent-coordination, and external. Each risk has a **trigger** (how we know it's happening), a **mitigation** (what we do about it), and an **owner-action** (what only the human can do).

---

## 1. UI framework dead-end (HIGH)

**Risk:** egui turns out to be a poor fit for an editor that needs custom text rendering, ghost text overlays, smooth scroll, and per-glyph subpixel positioning. We pick egui, build Phase 1, and discover the ceiling is too low.

**Likelihood:** Medium. egui works for chrome; the open question is whether the editor pane can ship on top of it at the quality bar Selora demands.

**Impact:** High. If egui is the wrong call, we either (a) rewrite the editor pane in raw `wgpu` while keeping egui for chrome, or (b) switch UI framework entirely. Both are 2-4 weeks of work.

**Trigger signals in Phase 1:**
- Text rendering shows visible jank on scroll (dropped frames).
- Custom `egui::Painter` calls don't hit the GPU efficiently enough — profiler shows main-thread-bound on draw.
- Ghost text animation can't be made smooth.
- egui's layout system can't produce the exact gutter / tab bar / sidebar split the design tokens require without ugly workarounds.

**Mitigation:**
- **Phase 1 is the canary.** If the spike proves egui is wrong, we fail fast — before we have built 10 features on top of it.
- The architecture (`AppState`, `Message`, `Effect`, crate boundaries) is framework-agnostic. Swapping egui for iced or a custom `wgpu` UI does not require rewriting the editor buffer, the LSP client, the AI client, or the file system code.
- A documented fallback in `ARCHITECTURE.md` §16: move just the editor pane to a lower-level renderer. Keep egui for chrome.

**Owner-action:** Watch the Phase 1 spike window carefully. If you see jank, hold the line — do not let subagents "fix" the wrong layer. Escalate to a framework decision before Phase 2.

---

## 2. LSP complexity on Windows (HIGH)

**Risk:** Wiring `rust-analyzer` and `typescript-language-server` as child processes, framing stdio JSON-RPC, handling request cancellation, surviving server crashes, and surfacing diagnostics is harder than it looks. The first cut of `selora-lsp` is full of subtle bugs: race conditions on didChange, dropped diagnostics, hangs on server death.

**Likelihood:** High. LSP looks like a wire protocol; it is a stateful multi-request protocol with 100+ method shapes. Building a correct client is a real engineering task.

**Impact:** Medium-High. A buggy LSP is worse than no LSP — it crashes the editor, drops completions, hangs the UI.

**Trigger signals in Phase 3:**
- The LSP test suite (`cargo test -p selora-lsp`) is flaky.
- Phase 3 acceptance tests pass on a small fixture but fail on a real-world 5000-line Rust file.
- The status bar shows LSP "ready" when it is actually crashed.
- Diagnostics arrive minutes late or not at all.

**Mitigation:**
- **Write the LSP client to the spec, not from memory.** The agent must read the LSP spec (microsoft/language-server-protocol) for the methods it implements. Each method has a typed request/response shape from `lsp-types`.
- **Build a 100% automated test layer for selora-lsp** before wiring it into the editor. Tests use a fake LSP server (a small `tokio` process that responds to initialize with a fixed set of capabilities) so we can test framing, request routing, cancellation, and server crash recovery without depending on rust-analyzer.
- **Run rust-analyzer against a tiny fixture workspace in CI** (a single 50-line file). If the fixture test passes, we have evidence the wiring is correct on a known-good input. Larger tests are the owner's job.
- **Hard timeouts on every LSP call** (5s default, 30s for hover/definition). If a server doesn't respond, drop the request, don't hang.
- **Server crash = disable the server, not crash the editor.** The actor pattern in `ARCHITECTURE.md` §5 is specifically designed for this: the child process dies, the actor emits `ServerDied`, the editor shows a toast and continues without LSP.

**Owner-action:** When triaging Phase 3 bugs, always check `git log --oneline crates/selora-lsp/` first. The most likely bug is in the actor loop, not the editor wiring.

---

## 3. SSE edge cases (MEDIUM-HIGH)

**Risk:** The gateway's SSE stream has subtle behaviors: a chunked response that ends abruptly, an event with no `data:` line, multi-line `data:` fields, a `:` comment line, mid-event disconnects, server-sent `[DONE]` followed by more data. The hand-rolled SSE parser in `selora-ai` has a bug in one of these cases and a chat reply gets truncated or duplicated.

**Likelihood:** Medium. SSE *is* simple, but it is also "simple" the way email headers are simple. There are real edge cases.

**Impact:** Medium. A bad chat reply is annoying but recoverable (the user just re-sends). A silent loss of the last tokens means the cost meter under-reports.

**Trigger signals in Phase 1 and Phase 4:**
- A chat reply ends mid-sentence.
- The `[DONE]` event is missed and the function never returns.
- A `data:` line exceeds the parser's buffer (unbounded growth).
- Disconnect mid-stream crashes the chat task.

**Mitigation:**
- **The SSE parser is unit-tested with 12+ fixtures** including: normal case, multi-line data, comment lines, missing event, abrupt close, very large single chunk, chunked across packet boundaries, `[DONE]` last, `[DONE]` followed by trailing data. These tests are mandatory in Phase 1.
- **Hard buffer cap** in the parser (1 MB). If a single `data:` exceeds that, surface a "gateway returned oversized chunk" error and abort the stream.
- **Streaming is per-request, not per-connection.** Each `stream_chat` opens its own HTTP request. There is no keep-alive SSE connection to manage. This means there is no "what if the connection drops" complexity at the protocol level — HTTP itself handles it.
- **The "Done" event is the source of truth for cost.** The receipt is built from the final chunk's `usage` field (prompt_tokens, completion_tokens). If the gateway doesn't return usage, we fall back to `len(chars)/4` with a "estimated" flag in the receipt. The cost meter always adds up correctly.

**Owner-action:** If you see a chat reply that looks truncated, the first place to look is the `parse_sse_event` function and its test fixtures. Do not let subagents "fix" the symptom by adding retry logic on top.

---

## 4. Subagent coordination failures (HIGH)

**Risk:** The build runs as multiple parallel subagents (Kimi K3, GLM 5.3, Claude with subagents). They have inconsistent context, sometimes hallucinate APIs, occasionally produce code that doesn't compile against each other. Phase 2 has 6 parallel subagents writing to 6 different crates — if their interfaces don't match, the merge fails.

**Likelihood:** High. This is the single most likely failure mode for a multi-agent project.

**Impact:** High. A broken merge stalls the project for hours while the orchestrator reconciles the subagents' outputs.

**Trigger signals:**
- A subagent's PR doesn't compile against the trunk.
- A subagent invented an API that doesn't exist in a sibling crate.
- A subagent rewrote a file another subagent is also rewriting.
- A subagent produced code that compiles but is semantically wrong (e.g. uses `String` where the spec says `Uuid`).
- Two subagents both added the same feature in different ways.

**Mitigation:**
- **`ARCHITECTURE.md` is the contract.** Every subagent reads it in full. If a subagent wants to change an interface, it must update `ARCHITECTURE.md` in the same PR. The orchestrator reviews `ARCHITECTURE.md` diffs in every PR.
- **Small, focused subagent briefs.** Each brief is one crate or one feature, with explicit inputs ("the sibling crate's API surface looks like this") and explicit outputs ("you add a function with this signature"). A subagent that has to guess is a subagent that will guess wrong.
- **The orchestrator runs a build + test gate between every subagent merge.** Not a "looks good" review — an actual `cargo build --workspace && cargo test --workspace`. If red, the subagent goes back.
- **One subagent per file is the rule, not the goal.** If two subagents both need to edit the same file (e.g. `AppState`), they are merged sequentially, not in parallel. The orchestrator splits the file first.
- **The orchestrator maintains a "stubs" crate** with empty function signatures for every cross-crate interface. Subagents implement against the stubs. The stubs are the contract; if the stub's signature is wrong, that's an architecture decision and the orchestrator updates it.
- **Explicitly state what subagents are NOT allowed to do.** "Do not modify `crates/selora-core`." "Do not change the `Effect` enum." "Do not add a dependency." Negative constraints prevent drift.

**Owner-action:** When a subagent merge fails, the *first* thing to check is whether the brief was clear. If the brief was unclear, rewrite the brief before re-running. Never re-run a subagent with the same brief and expect a different result.

---

## 5. Context window exhaustion (MEDIUM-HIGH)

**Risk:** Each subagent has a finite context window. By the time a subagent is in Phase 4, it has read `PLAN.md` (4k tokens), `ARCHITECTURE.md` (8k tokens), `DESIGN_TOKENS.md` (5k tokens), `UI_SPEC.md` (10k tokens), `ROADMAP.md` (10k tokens), `RISKS.md` (this file, ~5k tokens), `AGENT_ORCHESTRATION.md` (~5k tokens), and the existing crate sources. The brief is also long. The subagent runs out of context mid-implementation, starts producing code that contradicts earlier instructions, and the orchestrator has to throw away the run.

**Likelihood:** Medium-High in Phases 3-4-5 where the codebase is largest.

**Impact:** Medium. Wasted time and money, but recoverable.

**Trigger signals:**
- Subagent output drifts from the spec ("I'll just use `git2` for everything" when the brief said no).
- Subagent forgets the design tokens and uses hardcoded colors.
- Subagent re-defines types that already exist.

**Mitigation:**
- **Briefs are skim, not read.** The orchestrator gives the subagent a brief that points to `ARCHITECTURE.md` §X for the relevant section, not a copy-pasted re-summarization. The subagent reads just what it needs.
- **One subagent per phase chunk, not per phase.** A subagent that does "all of Phase 2" is too big. A subagent that does "2d syntax highlighting" is small enough to hold in context.
- **Subagents write a STATUS.md at the end** (under 200 words): what they did, what's open, what's blocked. The next subagent reads STATUS.md as a fast-load summary, not the full git log.
- **The orchestrator does not use the same subagent for two consecutive phases** if it can avoid it. Fresh context, fresh eyes.

**Owner-action:** If you see drift in a subagent's output, stop the run, look at the context usage, and consider whether the brief was too long.

---

## 6. Windows-specific build issues (MEDIUM)

**Risk:** egui + tree-sitter + rustls + Inno Setup + Windows specifics combine to produce platform-specific bugs: DPI scaling, font rendering, file path encoding, line endings, symlinks, long path support, the Windows Credential Manager API, dark mode title bar, etc.

**Likelihood:** High. Every cross-platform project hits at least 2-3 of these.

**Impact:** Medium. Each is fixable; together they eat time.

**Trigger signals:**
- Fonts look wrong on different DPI settings.
- File paths with non-ASCII characters break.
- The Windows title bar is white instead of dark.
- `git status` shows every file as modified (line ending mismatch).
- The Selora key is not actually stored in Credential Manager (the `keyring` crate is misconfigured).

**Mitigation:**
- **Test on the real owner's machine early.** Phase 0 is verified on `C:\Users\Toxic\Desktop\Selora IDE` (Windows 11). Phase 1 must be verified on the same machine, not on a CI runner.
- **CI on `windows-latest` for every PR.** Catches regressions.
- **Path handling is centralized in `selora-core::paths`.** All other crates go through it. Subagents don't get to call `Path::new` directly. This prevents one crate from using `\` and another from using `/`.
- **UTF-8 everywhere, always.** Rust strings are UTF-8; we never call `to_string_lossy` or `as_bytes` on a path.
- **Line endings are LF in the editor.** On save, the editor preserves the original line endings (detected from the file's first 8KB). This is a Phase 2 task; if it's missed, git status will look wrong on the owner's Windows machine.

**Owner-action:** When filing a Windows-specific bug, include the DPI setting, the Windows version, and the path encoding of the affected file. Subagents can't repro without it.

---

## 7. Editor feature creep (MEDIUM)

**Risk:** Phase 2-4 are large enough that subagents are tempted to "improve" the spec. "Let me also add bracket pair colorization." "Let me add multi-cursor." "Let me add minimap." Each of these is a real feature that real editors have. None are in Lite scope.

**Likelihood:** High. Subagents are trained to be helpful.

**Impact:** High if allowed. The Lite Edition stops being lite. The whole point of the lite tag is to ship a useful subset and stop.

**Trigger signals:**
- A subagent PR adds a feature that isn't in the brief.
- A subagent PR adds a dependency not in the workspace `Cargo.toml`.
- A subagent PR adds a new module that doesn't exist in `PLAN.md` §5.

**Mitigation:**
- **The "Non-goals" table in `PLAN.md` §2 is the policy.** Every PR review checks: is this feature in the non-goals? If yes, reject.
- **The "no" list in each subagent brief is binding.** "Do NOT add minimap" is a hard constraint, not a suggestion.
- **The orchestrator's job is to say no.** The build agent wants to be helpful. The orchestrator's job is to enforce the Lite tag.

**Owner-action:** When the build agent asks "should we also add X?", the answer is almost always no. The exceptions are documented in `PLAN.md` §2.

---

## 8. Selora API instability (LOW-MEDIUM)

**Risk:** `api.selora.lol` is the owner's gateway. It's a real production service. It could go down, change its auth scheme, return a different SSE format, deprecate a model, or rate-limit the IDE's heavy use.

**Likelihood:** Low-Medium. The owner controls the gateway, so the changes are coordinated, not surprise.

**Impact:** Low-Medium. The IDE should fail gracefully on any of these: show a toast, retry with backoff, never crash.

**Trigger signals in Phase 4:**
- A chat request times out.
- `list_models` returns a different shape than expected.
- A model id from last week is gone.
- The gateway returns a 502 mid-stream.

**Mitigation:**
- **The retry policy in `ARCHITECTURE.md` §6.5 is mandatory.** Network errors get 1 retry, 5xx gets 3 with backoff, 4xx gets no retry with a surface message. This is enough for the gateway to recover from transient issues.
- **The model picker caches the last successful `list_models` response** in `settings.json`. If the gateway is unreachable, the cached list is used with a "stale" indicator.
- **The error toast always shows the gateway's error message.** We do not paraphrase network errors; we show what the gateway said.
- **No background polling.** The IDE pings the gateway only on user action (open settings, switch model, start chat). Idle background polling is wasteful and creates rate-limit pressure.

**Owner-action:** If the gateway's API changes, the owner updates `selora-ai` and the model catalog. The IDE has a `SeloraStatus` indicator that surfaces problems quickly.

---

## 9. Agent hallucination of dependencies (LOW-MEDIUM)

**Risk:** Subagents sometimes invent crate names, version numbers, or API surfaces. `cargo add` will then fail, or the code will compile against a wrong API, or a published crate will be a typosquat.

**Likelihood:** Medium. This is a known failure mode of LLM-based code generation.

**Impact:** Low-Medium. Caught by `cargo build`, but wastes time.

**Trigger signals:**
- A subagent imports a crate that doesn't exist on crates.io.
- A subagent uses an API that doesn't exist on the version pinned.
- A subagent adds a crate that has the wrong license.

**Mitigation:**
- **`Cargo.lock` is committed.** Every PR is checked: is the new dep in `Cargo.lock`? If not, reject.
- **The orchestrator reviews every `Cargo.toml` change.** No subagent adds a dependency without the orchestrator's eyes.
- **License check.** `cargo deny` is run in CI (added in Phase 0). Any new crate with a non-MIT-compatible license fails the build.
- **The pinned dependency list is in `ARCHITECTURE.md` §4.** Subagents are told: "Use these exact crates at these exact versions. Do not substitute." If a subagent wants a different crate, the orchestrator approves the change.

**Owner-action:** When a subagent's PR is broken because of a phantom dependency, do not let them "just use a different crate". The orchestrator decides what the dependency is.

---

## 10. Installer + signing on Windows (LOW)

**Risk:** Windows code signing requires an Authenticode certificate ($200-400/year) and an EV cert for SmartScreen reputation ($300+/year). Without signing, users see an "unknown publisher" warning. With signing but no EV, SmartScreen still warns for the first ~100 downloads. The owner may not be ready to pay.

**Likelihood:** Certain (no cert exists yet).

**Impact:** Low. The installer works unsigned; it just looks scary to non-technical users. We document this as a known v1.x limitation.

**Trigger signals:** None — this is a known-acceptable state for Lite.

**Mitigation:**
- **`RELEASE.md` documents signing as optional.** Phase 5d's CI includes a `SIGNING_CERT_PATH` secret placeholder, with a no-op step that skips signing if the secret is empty. The owner can flip it on later.
- **The README mentions the warning honestly:** "Windows SmartScreen will warn on first run. Click 'More info' → 'Run anyway'. We're working on code signing."

**Owner-action:** Decide later whether to buy a cert. It's a one-line CI change to enable.

---

## 11. AI agent budget overruns (LOW-MEDIUM)

**Risk:** Building an IDE with subagents is not free. The orchestrator runs many long-context subagents over 8-14 sessions. If the subagents are inefficient (re-reading large files, retrying failed commands, producing bloated code that needs refactoring), the cost can balloon.

**Likelihood:** Medium. AI-agent projects have a tendency to "just one more iteration" their way into a large bill.

**Impact:** Medium. Stops the build at the budget wall.

**Trigger signals:**
- A single subagent session uses 5x the expected tokens.
- A subagent enters a "fix bug → break something → fix that" loop.
- The orchestrator runs the same subagent twice because the first output was rejected.

**Mitigation:**
- **Per-phase budget caps.** The orchestrator tracks tokens per phase. If a phase exceeds 2x the expected budget, the orchestrator pauses and the owner decides.
- **Briefs are concise.** A 4-page brief is more expensive to read than a 1-page brief, and not more likely to produce a better subagent. Concise briefs save money.
- **Subagents are told to stop and report if they're stuck.** Better to return "I'm stuck on X" than to burn 30 minutes trying to escape a corner.
- **`STATUS.md` after each subagent run** is the gate. If a subagent didn't write one, its output is suspect.

**Owner-action:** Watch the token usage dashboard. If Phase 2 is using 3x Phase 1's budget, something is wrong with the orchestration, not the subagents.

---

## 12. Burnout of the owner (MEDIUM)

**Risk:** This is a *people* risk, not a technical one. The owner is solo. They run the orchestrator each evening. The build takes 8-14 sessions. That's 2-3 weeks of nightly commitment. If the owner loses a week to a day job crunch, the project stalls.

**Likelihood:** Medium-High. Solo maintainer projects are vulnerable to life.

**Impact:** High. A stalled project rots; AI agents' context drifts; the owner loses momentum.

**Trigger signals:**
- Two consecutive weeks without a session.
- The owner starts "catching up" by running the orchestrator in the morning, which burns out faster.
- The owner starts skipping acceptance tests to "save time".

**Mitigation:**
- **Each phase has a clean DONE state.** If the owner has to stop, they stop after a merge, not mid-phase. The next session starts from a green build.
- **`STATUS.md` is also for the owner.** "Phase 2 done. Phase 3 brief is in `ROADMAP.md` §3. No blockers." Pick up here tomorrow.
- **The plan is sized for 2-3 weeks, not 2-3 months.** If the owner can't sustain 2-3 weeks, this project isn't the right project. The plan says so honestly.
- **Lite Edition is the goal.** When the Lite Edition ships, the project is done. There is no Phase 6, 7, 8. The owner gets to stop and use what they built.

**Owner-action:** When life gets busy, stop after the current merge, write a status note, and walk away. The project will be there tomorrow.

---

## Risk register summary

| # | Risk | Likelihood | Impact |
|---|---|---|---|
| 1 | UI framework dead-end | Medium | High |
| 2 | LSP complexity on Windows | High | Medium-High |
| 3 | SSE edge cases | Medium | Medium |
| 4 | Subagent coordination failures | High | High |
| 5 | Context window exhaustion | Medium-High | Medium |
| 6 | Windows-specific build issues | High | Medium |
| 7 | Editor feature creep | High | High |
| 8 | Selora API instability | Low-Medium | Low-Medium |
| 9 | Agent hallucination of dependencies | Medium | Low-Medium |
| 10 | Installer + signing | Certain (cost issue) | Low |
| 11 | AI agent budget overruns | Medium | Medium |
| 12 | Burnout of the owner | Medium-High | High |

**Top three risks to watch in execution order:**
1. **#1 UI framework dead-end** — caught in Phase 1.
2. **#4 Subagent coordination failures** — present from Phase 2 onward.
3. **#2 LSP complexity on Windows** — concentrated in Phase 3.

The other risks are managed by the architecture and the orchestration pattern; these three are where the project is most likely to actually fail.
