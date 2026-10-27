# Kimi K3 brief — copy everything below the line into the subagent

You are Phase 6h of the Selora IDE build (AI presence & motion effects).
Phase 6h runs AFTER Phase 5d has merged — and 5d is NOT started yet. The prerequisite brief is PHASE5D_PROMPT.md at the repo root (Phase 5d: agent cards UI + the final Phase 5 wiring). Run that first; it must be merged and green before you start. If 5d is not merged, stop and report.

Working directory: E:\Projects\Selora IDE (this checkout — adjust paths if you run elsewhere)

---

CONTEXT — WHAT THIS PHASE IS:

The owner wants the interaction language of five web libraries brought into the IDE:

- thinking-orbs — "ThinkingOrb": nine hand-tuned animated states replacing a spinner (docs: https://libraries.dev/orbs.html)
- bot-avatars — "BotAvatar": lit canvas-drawn bot shapes with idle/working/sleeping states, a roster (docs: https://libraries.dev/bots.html)
- voice-glow — "VoiceBeam": a sound-reactive glow along a child's bottom edge (docs: https://libraries.dev/voice.html)
- liquid-gooey — "Liquid": SVG-filter gooey merge between marked items (docs: https://libraries.dev/gooey.html)
- metal-fx — "MetalFx": WebGL liquid-metal with wandering halo (docs: https://libraries.dev/metal.html)

Selora IDE is pure egui: **no webview, no React, no npm, no JS** (PLAN.md §2 non-goals; ARCHITECTURE.md §4.1). You do NOT install these packages. You port their *behavior* — the motion semantics, states, and reaction curves — as native egui painted widgets, in the Selora monochrome design language. The libraries are reference material: study their state sets and timing, then draw them yourself.

The hard constraint is DESIGN_TOKENS.md: monochrome ink ramp, hairlines, the 8 motion glyphs (§7.3), no decoration (§12). These five effects are colorful/exuberant on the web; in Selora they become ink. Where the port bends §12, flag it in your report for owner sign-off — never silently.

READ FIRST, in full, before writing any code:
- DESIGN_TOKENS.md §1 (ink ramp), §7 (motion — curves, durations, the 8 glyphs), §12 (the no-decoration rule)
- UI_SPEC.md §4 (chat sidebar: streaming bubble, composer, agent pill) and §4.2 (agent status line, tool cards, turn summary)
- ARCHITECTURE.md §17.3–17.4 (the loop states your widgets visualize)
- crates/selora-editor/src/ghost_text.rs — the comet fade: the codebase's existing custom-paint + animation pattern to follow
- crates/selora-ui/src/theme.rs — the typed tokens; every color, duration, and curve comes from here, never hardcoded

THE DEPENDENCIES YOU CONSUME:
- crates/selora-ui — theme tokens; the chat panel (src/sidebar/chat/mod.rs); 5d's landed card/status-line surfaces
- crates/selora-ai/src/agent/turn.rs — AgentEvent::State(AgentTurnState) / Delta / ToolCallStarted / ToolResult / ApprovalNeeded / TurnDone: the signals your widgets react to (via the AppMessage reduction in selora-app)

DELIVERABLES — one widget module per file under crates/selora-ui/src/, each with headless paint tests (the existing selora-ui test pattern):

1. `ThinkingOrb` (presence/orb.rs): the thought indicator. Replaces the ⠿⠿⠿ wave in the streaming bubble (UI_SPEC §4 — same trigger: only after 800ms with no delta) and rides the agent status line. Nine states, mapped to loop states:
   - Thinking → `working`; RunningTool (read-type) → `searching`; RunningTool (bash/write) → `solving`; AwaitingApproval → `listening`; streaming deltas → `connecting`; idle/finished → `breathing`
   - `weaving`/`composing`/`shaping` reserved: composing = ghost text appearing, weaving = search running, shaping = file tree refresh (wire only if 5d exposes the state cheaply; otherwise leave the enum arms)
   - Two sizes: 20 (inline) and 64 (avatar scale), separately tuned. `speed` (clock multiplier, default 1) and `paused` props preserved.
   - Ink monochrome: dots in --fg-faint / --fg-muted; --ok reserved for a liveness dot only. Loops use --dur-loop (1200ms) and --ease-orbital — no loop faster than 1.2s (§12).
   - Reduce-motion (§7.4): a static dot, no motion.

2. `AgentAvatar` (presence/avatar.rs): the bot-avatar port — an agent identity glyph. One ink silhouette (your pick from simple rounded body shapes; translate to 2D canvas painting in ink steps --ink-faint/--ink-low/--ink-mid — the web version's fur/plastic shading becomes ink steps). Three states: `default` (idle look-around), `working` (a hop/spin synced to the ThinkingOrb clock), `sleeping` (head down, slow breath — only when the gateway is disconnected). `seed` (0–1) so a future row never blinks in unison. Used at 20px in the agent status line and tool-card headers; 64px is reserved for a future multi-agent roster. No accessories (hats/glasses), no saturated color, no emoji.

3. `StreamGlow` (presence/stream_glow.rs): the voice-glow port — a glow along the composer's bottom edge that rises and blooms with **streaming activity** (deltas per second), NOT a microphone (voice input is out of Lite scope — a deliberate deviation from the source library; note it in your report). `processing` gathers it into a beam that travels the composer's width while a tool runs. Monochrome: bloom in --fg-faint/--fg-muted; the ONLY green is a one-shot --ok spark on turn completion (semantic, §1.3). Auto-detect the composer's corner radius. Decay uses attack/release-style envelope tuned to §7.2 durations.

4. `LiquidPill` (presence/liquid.rs): the gooey port, on ONE seam — the agent-mode pill in the chat header (UI_SPEC §4.2). Off → on: the dot slides and the pill background melts between --ink-low and --ink-mid. The web version's SVG blur becomes layered painted alpha circles (cap it: ≤12 circles). `transition` prop: sharp | pulse | soft | bouncy → the four §7.1 curves.

5. `MetalMark` (presence/metal.rs) — OWNER-DECISION ITEM: a liquid-metal sheen sweep across the splash brand mark, built on the comet glyph vocabulary. Monochrome (white sheen on ink). Ship behind a `const METAL_MARK_ENABLED: bool = false` flag; the owner flips it. If the owner says no, delete the file. This is the single permitted specular highlight in the app — do not extend it anywhere else.

INTEGRATION (small, surgical — 5d is merged by now):
- Streaming bubble: ⠿⠿⠿ → ThinkingOrb(20) per the 800ms rule.
- Agent status line: AgentAvatar(20) + ThinkingOrb(20) + phase text + Stop button (5d painted the line; you swap its indicator).
- Composer: StreamGlow under the input.
- Chat header: LiquidPill on the agent toggle.

DO NOT:
- Add React, npm, a webview, Node, or any JavaScript. Native egui only; the five libraries are reference material.
- Violate DESIGN_TOKENS.md §12 (no gradients on chrome, no shadows, no saturated color except semantic --ok/--err/--warn, no loop under 1.2s, no bold, no emoji). Every bend you must make: list it for owner sign-off.
- Touch selora-ai, the router, the loop, wiring.rs, or 5d's card rendering internals (file ownership: AGENT_ORCHESTRATION.md §10).
- Hardcode colors, durations, or curves — everything flows from theme.rs.
- Block the UI thread: paint-only widgets, zero per-frame allocation, no threads. Drive repaint with `ctx.request_repaint_after(...)` only while animating (the ghost_text.rs pattern), and pause offscreen.

ACCEPTANCE:
- `cargo build --workspace && cargo test --workspace` green; `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`.
- Headless paint tests covering: every orb state paints distinct geometry; avatar state transitions cross-fade correctly; StreamGlow rises on a scripted delta burst and decays to zero; LiquidPill off→on paints the melt; reduce-motion freezes every widget.
- A stress frame paints 100 widgets within the 60fps budget (the existing egui_kittest-style headless frame, timed).
- Manual: `cargo run -p selora-app` → stream a chat reply (orb replaces the wave after 800ms); toggle the agent pill (it melts); the composer glow follows streaming; disconnect the gateway (avatar sleeps).

WHEN DONE, report:
- The exact commands you ran and their outputs.
- Every deviation from DESIGN_TOKENS §12 / UI_SPEC §4, with the owner-decision items flagged.
- Whether the §7.3 glyph vocabulary was sufficient or you needed a new glyph (if new: name it, spec it, and flag it).
- Whether the architecture held for this kind of painted-widget work.
