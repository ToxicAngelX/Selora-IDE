# The premium UI revamp

**Where things stand — verified by me today (2026-10-08), not assumed:**

- **Nothing is committed yet.** The whole revamp is uncommitted working-tree changes on branch `main` (HEAD `07dff33`): 19 files modified, +2,733/−444 (`git diff HEAD --stat`), plus new untracked files — `crates/selora-ui/src/presence/beam.rs`, `docs/ui/` (spec + this report), `revamp-cheat.png`. Reviewing and committing this state is the open action.
- **I ran the three gates myself on this exact tree.** Build: pass. Clippy: pass. Tests: pass on a full re-run — but the first full run failed 3 filesystem tests that then passed twice; they look flaky, are not in revamp-touched code, and are flagged honestly in "Verified by".
- "Design spec approved" and "Design QA: passed" come from this run's brief. No QA checklist or artifact was handed to me, so I cannot show what that review covered or when it ran relative to the last edits. What I can show is in "Verified by".
- Per-workstream details originate from the three builders' summaries in this run's record (not a file in the repo). Everything load-bearing below is re-grounded in code lines, git output, or commands I ran myself.

## What you will see

**Hero composer & chat.** New: a band of light travels the composer card's edge while a request is in flight. The card, focus rings, welcome hero, streaming bubble and status line predate this pass and were verified in place. Typing behavior is unchanged — Enter sends, Shift+Enter newline, ↑/↓ history, send↔stop swap — and that behavior is covered by passing tests I ran today: `enter_sends_pushes_the_user_bubble_and_clears_the_composer`, `shift_enter_inserts_a_newline_without_sending`, `arrow_recall_walks_the_history_and_restores_the_draft`, `send_is_blocked_while_a_stream_is_in_flight` (`chat/mod.rs:3474`, `:3508`, `:3650`, `:3588`). The composer builder reports reading those paths before and after its edits and finding them unchanged; I could not diff-check that claim because the whole revamp is one uncommitted state — the git diff spans all phases, not just this pass.

**Presence & welcome.** The new `BorderBeam` widget (`presence/beam.rs:58`, exported at `presence/mod.rs:45`). The splash test harness was repaired: egui windows fade in over ~0.08s and two tests asserted exact colors on the very first frame, so the harness now advances a fake clock past the fade before asserting (visible in `git diff HEAD -- crates/selora-ui/src/splash.rs`). The deleted `debug_dump_rect_strokes` was a debug-only helper — `grep -rn debug_dump_rect_strokes crates/` now returns nothing, so nothing depended on it and no assertion coverage was lost. Tests red at session start, green now (per the summaries; both re-run by me today, ok): the splash focus-border test and the find-bar focus-border test.

**Workbench chrome.** Already landed; this pass repaired the one red chrome test and updated stale comment cross-references to the spec's renumbered sections (bend cites §10→§11, glow-ring cites §2.3→§2.2, standing-cost cites §8→§9, test-matrix cites §9→§10 — comments only, no code; per the builder summaries).

## The composer & the glow

- The composer sits on a quiet card with a hairline border; focusing it blooms five soft glow rings. Idle, hover and focus choreography unchanged.
- While a request is in flight, the beam rides the card's perimeter: 24 short segments in four stepped gray brightness bands, 2px, no tint, no blur, one eased lap every 2.4s (`DUR_LAP_MS = 2400`, `crates/selora-tokens/src/lib.rs:127`). "Strength" is the widget's intensity parameter; at 0 it paints nothing (`zero_strength_paints_nothing`, `beam.rs:438`). Under reduce-motion it is a single static segment.
- **Disconnect:** the beam is driven only by the in-flight flag (`chat/mod.rs:2175`), never by connection state. This is verified by `header_orb_matches_the_status_line_mapping` (`chat/mod.rs:5124`), whose assertion at `:5261-5265` checks that with the gateway disconnected the orbs pause and only the in-flight beam still requests frames — I ran it today: ok.
- All seven beam tests run by me today, all ok: the five widget tests (`beam.rs:308-438`) plus `in_flight_composer_paints_the_beam` and `beam_stops_when_the_stream_completes` (`chat/mod.rs:5499`, `:5530`).

## Presence

The full presence family, all sharing the same motion tokens: **orbs** (breathing, header + empty state), **avatar** (beside streamed answers), **stream glow** (rises with the answer, completion spark), **liquid pill** (the gooey agent toggle), **metal mark** (metallic brand mark), and the new **border beam** above.

## The seven libraries from the owner's set

Six ported: thinking-orbs → the orbs; bot-avatars → the avatar; voice-glow → the stream glow; liquid-gooey → the agent pill; metal-fx → the metal mark; border-beam → the composer's in-flight beam.

One deliberate non-port: **img-fx**, a WebGL image loader. A Lite IDE has no image-display surface — the spec records a verification grep over the crates finding no image/texture API anywhere — so there is nowhere for it to land. Recorded as a decision, not an oversight (`docs/ui/REVAMP_SPEC.md:228`).

## Chrome

Verified against the spec's state matrix by reading every surface: title-bar menus, activity-bar pills with edge marks, tabs, file-tree selection, find bar, status bar, git/search/problems panels, toasts. The five spec-named paint tests and the repaired find-bar test are not just present — I ran them today: `selected_pill_paints_the_edge_mark` and `selected_row_paints_the_edge_mark` ok; `menu_button_hover_paints_ink_faint_background` and `connect_run_hover_paints_the_defined_target` ok; `focused_query_field_paints_the_focus_border` ok; `clickable_run_hover_uses_the_defined_target` inside the 361-test suite pass below. I also read the brand, icon, cost-meter and editor files against the token rules and found no violations (a read-through, not an automated check — none exists for those).

**Three pre-existing nits — reported, not changed, decision needed.** Each predates this revamp, proven by git blame, and each sits outside the revamp's enumerated scope, which is why the state-matrix checks never covered them:

- Problems badge: 9px font and hardcoded offsets (`activity_bar.rs:172-185`) — blame: commit `b8898b7`, 2026-10-07 (Phase R6a).
- Toast fade timings as literals 0.24s/0.40s instead of motion tokens (`toasts.rs:84-85`) — blame: `07dff33` (Phase R7).
- Problems-panel constants off the 4px grid (`panel/problems.rs:22`, `:87`) — blame: `b8898b7`.

There is no issue tracker; this list is the record. Decision requested: bring them onto tokens/the type floor, or accept as-is.

## Owner sign-off bends

A "bend" is a change the owner requests when signing off a pass. The run brief records **none** for this revamp. In plain terms: the owner approved the spec and design before implementation and has not yet reviewed this final state — no review round has happened since these edits.

## Verified by

All commands run by me today on the final, uncommitted tree:

- `cargo build --workspace` → `Finished` in 1m34s, no errors.
- `cargo clippy --workspace --all-targets -- -D warnings` → `Finished`, no warnings.
- `cargo test --workspace` → **run twice.** First run: **3 failures**, all in `selora-fs` tree tests (`create_file_and_dir_succeed_then_conflict`, `rename_moves_and_reports_conflicts`, `rename_directories`). Then `cargo test -p selora-fs --lib` alone: 43/43 ok. Full second run: every suite ok, 0 failed (including selora-ui 361/361). `git diff HEAD --stat -- crates/selora-fs` is empty — the revamp touches none of that code. Verdict: flaky filesystem tests under full-suite parallelism, not a revamp regression — but the earlier "all gates green" claim in the brief predates my runs, and I am reporting both outcomes rather than papering over the first one. Worth a separate look.
- Name-filtered runs quoted above (beam 8/8, disconnected 6/6, edge-mark 2/2, hover 2/2, both repaired focus-border tests ok).

**Not verified, said plainly:** the "unchanged behavior" claim rests on the composer builder's before/after read-through — there is no committed baseline to diff against; the design-QA verdict has no artifact I can inspect; the run brief's gate results cannot be tied to a point in time, which is why I re-ran everything myself.

**Visual evidence:** I captured no screenshots or recordings this session (headless work). One real screenshot of the current build exists at `revamp-cheat.png` (repo root, uncommitted, not mine): it shows the file tree, editor, the "Ask anything" welcome state with suggestion chips, the agent pill, and the keyboard-shortcut cheat sheet. It does not show the in-flight beam — that only appears during a live request, which is what the checklist below is for.

## Check it yourself

Run `cargo run -p selora-app`, open the chat, watch the breathing orb on the empty state, focus the composer to see the glow bloom, send a message and watch the glow rise with the stream, toggle the agent pill.
