# Selora IDE — Premium UI Revamp Spec (v4)

Status: build-ready. Audience: the four implementers in the ownership map (§12).
Basis: every value below was verified against the code it changes — cited as `path:line`, read or run this session. Where the revamp brief and the code disagreed, the code won and this spec says so.

Direction (owner, verbatim): the IDE looks bleak; the prompt box gets the ZCode glow; the thinking orbs become visible; the whole UI feels premium, with a well-planned UX. Polish, not rebuild: no panel moves, no new crates, no new top-level modules.

## 0. State of play (read first)

Direction items 1–5 are **implemented in the working tree** (uncommitted): the composer card + glow rings at `sidebar/chat/mod.rs:2030-2351`, the header orb at `:2623`, the 64px welcome hero at `:1820`, the chrome edge marks at `sidebar/activity_bar.rs:146-152` and `sidebar/file_tree.rs:341-342`, with their headless tests present. The new work in this spec is **item 6, the border beam (§8)** — no `beam.rs` exists in `presence/` — and **item 7, the img-fx non-port record (§11)**.

**But the baseline is red, and v3 of this spec was wrong to imply otherwise** (it listed a failing test as "landed"). Ran this session: `cargo test -p selora-ui --lib 2>&1 | tail -40` → **352 passed, 3 failed** (`splash::tests::debug_dump_rect_strokes`, `splash::tests::focused_key_input_paints_the_focus_border`, `editor::find_bar::tests::focused_query_field_paints_the_focus_border`). All three sit in the uncommitted diff; none was ever green. §0.1 is the repair contract; it lands before any §8 work.

### 0.1 Baseline repair (gate: `cargo test -p selora-ui --lib` green)

Root cause (verified against egui 0.36.2 source and a throwaway probe crate replicating the structure — probe deleted after use):

- `egui::Area` runs a **sizing pass on its first frame**: the content Ui is invisible (`area.rs:444` `sizing_pass = state.is_none()`, `:625-626` `.sizing_pass().invisible()`), and an invisible painter turns every added shape into `Shape::Noop` (`painter.rs:213-215`). The splash dump test's own output shows it: 16 shapes, all Noop.
- The Area then **fades in** (`fade_in: true` by default, `area.rs:149,632-645`): `multiply_opacity(quadratic_out(age / animation_time))` premultiplies every fill/stroke (`painter.rs:202-206`). A test whose `RawInput.time` never advances keeps `age ≈ 0`, so colors stay ~1/5 intensity forever — the probe showed the focused field's `FG_DEFAULT` border painted as `#363636` alpha 60 instead of `#e5e5e5`. Exact-color assertions can only pass once the headless clock advances past the fade.
- Focus itself is not the bug: `request_focus` registers during the sizing pass (probe: `focused=Some(id)` from frame 0; the sibling test `enter_and_shift_enter_step_the_matches`, gated on focus at `find_bar.rs:122`, passes).

Repairs:

- **Presence builder** (`splash.rs`): delete `debug_dump_rect_strokes` (`:704-728` — leftover scaffolding, `panic!("dump")` by construction). Fix `focused_key_input_paints_the_focus_border` (`:731`): paint one settle frame first (the sizing pass paints all-Noop), and give the harness `frame`/`frame_full` a `time: f64` parameter that advances per frame (frame 2 at ≥ +0.2s so the fade completes and colors are exact) — the chat panel's `raw_input_with` time parameter is the in-repo precedent.
- **Chrome builder** (`editor/find_bar.rs`): same fix in `focused_query_field_paints_the_focus_border` (`:437-460`): `screen()` gains `time: Some(t)`; frame 2 runs at a later `t` than frame 1 (≥ 0.2s past the fade-in), then the `Rect{fill INK_LOW, stroke 1px FG_DEFAULT}` assertion matches. The feature code (`field_frame`, `:304-316`) is correct — the tests were authored broken; no production-code change beyond the deleted debug test.
- **Gate**: each builder re-runs the full command above; green is the definition of done for every later section.

---

## 1. TOKENS

**One new token.** The existing color/space/radius/type ramp suffices; every effect is paint-time alpha on existing tokens via `presence::with_alpha` (`crates/selora-ui/src/presence/mod.rs:67`). File-local alpha/geometry constants follow the `stream_glow.rs:41-62` precedent (file constants, not tokens).

| Token | Value | Joins | Why it is genuinely necessary |
|---|---|---|---|
| `motion::DUR_LAP_MS` | `2400` (exactly 2 × `DUR_LOOP_MS`) | DESIGN_TOKENS.md §7.2 durations table | The beam's perimeter lap is a loop, so §12's ≥1200ms floor makes it a *duration*; at 824px of perimeter (§8.3) a 1200ms lap reads hurried and the §7.2 ceiling (2000ms) forbids tuning past it without a token. Owner pre-authorized "a perimeter-lap duration". Bend §11.6a. |

File-local constants (landed unless noted): `GLOW_RING_ALPHAS` `[0.18, 0.12, 0.08, 0.05, 0.03]` (`chat/mod.rs:139`), `GLOW_RING_STEP` 2.0 (`:143`), `READY_SHIMMER_*` 0.06–0.12 / static 0.08 (`stream_glow.rs:59-62`), `EDGE_MARK_W/H` 2×16 / 2×14 (`activity_bar.rs:34-35`, `file_tree.rs:57-58`), `CHIP_STAGGER_S`/`CHIP_BLOOM_S` (`chat/mod.rs:148-150`), beam constants in §8.2 (new, `presence/beam.rs`).

---

## 2. HERO COMPOSER (the chat input card) — LANDED

Owner: composer builder. Verified: `paint_composer` `chat/mod.rs:2030`, `composer_rows` `:2469`, `composer_height` `:2488`.

### 2.1 Layout (landed)

- Bottom `Panel` stays, transparent frame, no separator (`:1351-1355`). Card margins `space::S3` on all four sides — ring 5's paint extent is 10.5px, so the 12px margin guarantees no panel clip (`:2035-2037`).
- Card: fill `INK_LOW`, corner `radius::R2`, 1px inside stroke — rest `INK_HI`, hover/focus `INK_RULE` (`:2144-2157`). Inner padding `S3`.
- Row 1: frameless multiline `TextEdit` (`.frame(egui::Frame::default())`, `:2174-2189` — egui's own fill/focus stroke would double-paint the card), `--type-md` sans, hint "Ask anything…", Shift+Enter newline, Enter submit, ↑/↓ history. Send/Stop `28×20` (`SEND_BUTTON_SIZE` `:125`): armed fill `INK_MID` hover `INK_HI` glyph `FG_DEFAULT`; disabled no-fill `FG_DISABLED`; plain-stream Stop = painted 8px square, hover `ERR` (`:2200-2237`).
- Row 2 (footer, inside the card, `S2` below row 1, 24px): left = model chip (`set_on_card(true)`, `model_picker.rs:238` — landed) + `S2` + the `agent` `LiquidPill` (24px, `liquid.rs:28`); right = key hint `--type-xs` `FG_FAINT`, right-aligned, **whole or hidden, never elided** (`:2327-2341`).
- Height: `input_row = max(rows·pitch, 20)`, pitch measured from the real galley each frame (`:2472-2480`); `bar = input_row + 2·S3 + 2·S3 + S2 + 24` (`:2491-2496`).

### 2.2 States (landed)

| State | Card stroke | Glow rings | StreamGlow (card bottom edge) | Border beam (§8) |
|---|---|---|---|---|
| Rest | `INK_HI` | none | nothing | nothing |
| Hover (unfocused) | `INK_RULE` | none | nothing | nothing |
| Focused | `INK_RULE` | 5 rings | + ready shimmer iff connected | nothing |
| In flight (plain or agent) | per focus/hover | per focus | delta envelope / tool beam / `--ok` spark | **rides the perimeter** |
| Disconnected | rest/hover | rings still paint | no shimmer (avatars-sleep precedent) | rides while a request is actually in flight |
| Reduce-motion | unchanged | snap with focus, no fade, no repaints | static α0.08 line; static beam/spark | static segment, top-left corner (§8.4) |

- Hover: `ui.rect_contains_pointer(card_rect)` (`:2097`). Rings: `fade` glyph, `DUR_QUICK_MS`, `EASE_FADE` (`:2096-2126`); **reduce-motion snaps them** (owner's glow rule over §7.4's fade-keep; §11.5). Ring i: `card_rect.expand(2·(i+1))`, corner `R2 + 2·(i+1)`, 1px `FG_FAINT` at `GLOW_RING_ALPHAS[i]` (`:2128-2143`).
- StreamGlow: `set_ready(gateway_connected && composer_focused)`, `set_cap_radius(R2)`, `processing` while a tool runs (`:2348-2351`).

### 2.3 The beam hook (the only composer edit in v4)

Paint order on the card, bottom to top: glow rings → card fill + stroke → **border beam (in flight only, §8.6)** → StreamGlow band. The composer constructs one `BorderBeam` at `ChatPanel::new`, threads `set_reduce_motion` beside the other presence widgets (`:826-831`), and calls `beam.show(ui, card_rect)` after the card stroke, before `stream_glow.show`.

---

## 3. CHAT HEADER — LANDED

Owner: composer builder. 40px, bottom hairline `INK_HI` (`:1390-1398`). The 20px orb at x = `S3`, vertically centered (`:1414-1420`); the session switcher shifts right by `ORB_INLINE + S2` (`:1445-1448`); overflow `⋯` unchanged.

`header_orb` (`:2623-2646`) — **the same mapping as the status line over the same turn slice**, so header and timeline never disagree: agent turn → `orb_state_for` (`:2594-2614`: parked approval `Listening`; bash/write `Solving`; read `Searching`; thinking+deltas `Connecting`; thinking `Working`); plain stream → `Connecting`/`Working`; idle → `Breathing`; disconnected → `.paused(true)` on the *mapped* state; reduce-motion threaded (`:826-831`).

---

## 4. EMPTY / WELCOME STATE (chat) — LANDED

Owner: composer builder (`paint_conversation` empty branch, `:1806-1925`).

- **Hero**: `ThinkingOrb::new(OrbState::Breathing, ORB_AVATAR)` (64px), alive at rest; the `--ok` satellite paints iff `liveness(gateway_connected)` and not reduce-motion (`orb.rs:185-191`, `:295-297`; reduce-motion hero = the one static muted dot, `orb.rs:254-263` — satellite-free, stated so nobody re-adds it).
- **Heading**: "Ask anything" — `--type-xxl`, `family_sans_semibold()`, `FG_DEFAULT` (`:1825-1832`). **Subline**: italic `--type-md` `FG_MUTED` (`:1834-1841`; §2.5-sanctioned).
- **Chips**: fill `INK_LOW`, 1px `INK_HI` border, `R1`; hover fill `INK_MID`, border `INK_RULE`, text `FG_DEFAULT`; ≥28px tall, `S3` horizontal padding (`:1868-1900`).
- **Entrance** (`:1787-1798`, `:1851-1860`): `welcome_anchor` on the frame clock, re-armed whenever the transcript re-empties; chip i blooms alpha 0→1 + scale 0.96→1.0, `EASE_PRECISE`, `DUR_BASE`, stagger `i · DUR_INSTANT` — ≤3 steps, one-shot, kept under reduce-motion per §7.4.
- Click → prefill + focus (`:1920-1923`).

---

## 5. STREAMING BUBBLE & STATUS LINE — UNCHANGED, RECORDED

No edits; recorded so implementers never re-derive. Streaming bubble: 800ms no-delta rule (`THINKING_AFTER` `:130`) swaps the text tail to `ThinkingOrb(Working, ORB_INLINE)`; the `comet` fade-in stays (`FADE_DURATION` `:133`). Status line (24px, `:1978-2008`): `AgentAvatar` + orb via `orb_state_for`; Stop on the right. §3's header orb reuses this exact mapping.

---

## 6. SPLASH & EDITOR EMPTY STATE — LANDED (one repair, §0.1)

Owner: presence builder.

- Splash (`splash.rs`): layout/motion unchanged (orbit `:143-149`; MetalMark sheen behind `METAL_MARK_ENABLED = true`, `metal.rs:27`, `:209-215`; fade-out kept under reduce-motion). Key input focus: 1px `FG_DEFAULT` while focused, `INK_HI` otherwise (`:243-262`). The two red tests are §0.1 repairs.
- Editor empty states (`empty_state.rs`): unchanged — §10-conformant buttons (`:63-77`); `pane_no_tabs` chips display-only (`:113-125`).

---

## 7. WORKBENCH CHROME — LANDED (one repair, §0.1)

Owner: chrome builder. The DESIGN_TOKENS §10 matrix everywhere, plus the revamp's edge-mark extension (✱):

| State | Foreground | Background | Extra |
|---|---|---|---|
| Idle | `FG_MUTED` | transparent | — |
| Hover | `FG_DEFAULT` | `INK_FAINT` | — |
| Active/pressed | `FG_DEFAULT` | `INK_LOW` | — |
| Selected | `FG_DEFAULT` | `INK_LOW` | ✱ 2px leading edge mark, `FG_MUTED`, `R1` |
| Focused | `FG_DEFAULT` | `INK_LOW` | 1px `FG_DEFAULT` border |
| Disabled | `FG_DISABLED` | transparent | — |

- **Title bar** (`title_bar.rs`): 32px `INK_CANVAS`; menus ride egui `menu_button` with the scoped `menu_button_matrix` (`:192-205`) — hover `INK_FAINT`, open/pressed `INK_LOW`, no borders, `R1`; text `FG_DEFAULT` in every state (app chrome).
- **Activity bar** (`activity_bar.rs`): 28×28 pills — selected `INK_LOW`, hover `INK_FAINT`, pressed `INK_MID` (`:130-138`); ✱ edge mark 2×16 `FG_MUTED` at `S1`, centered on the pill (`:146-152`).
- **Editor tabs** (`editor/tabs.rs`): active = the 1px `FG_DEFAULT` bottom line (`:362`); hover `INK_FAINT`; pressed tracked (`:350`); dirty `--warn` dot unchanged.
- **File tree** (`sidebar/file_tree.rs`): selected `INK_LOW` (`:330`), hover `INK_FAINT` (`:332`); ✱ edge mark 2×14 at the row's left edge (`:341-342`); no row borders (§5.2).
- **Find/replace** (`editor/find_bar.rs`): fields take the Focused row — 1px `FG_DEFAULT` when focused, `INK_HI` otherwise (`field_frame` `:304-316`). Its red test is a §0.1 repair.
- **Status bar** (`status_bar/`): 24px. Clickable runs get the defined hover target — galley bounds +(`S1`, 2px), `R1`, `INK_FAINT` fill + `FG_DEFAULT` text (`selora_status.rs:41-42,194-200`; `workbench_zone.rs:19-20,158-160`).
- **Panels** (`sidebar/mod.rs`, `panel/`): hairlines `INK_HI`, fills, headers unchanged.

**Verify-only checklist** (the owned-but-unmentioned files; "verify-only" means: run the file's existing tests, audit that every color/space/type value comes from `crate::theme` — no hardcoded rgb/px literals — and touch nothing): `brand.rs` (no tests of its own — its paint is covered transitively by the splash/title-bar tests; audit only), `sidebar/icons.rs` (no tests; audit only — exercised by the activity-bar tests), `status_bar/cost_meter.rs` (9 tests), `editor/mod.rs` (module declarations only), `editor/split.rs` (11 tests). A failed existing test in any of these is a blocker to report, not to fix silently.

---

## 8. THE BORDER BEAM — NEW (the sixth port; the ZCode prompt-box signature)

Owner: **presence builder** (widget) + **composer builder** (the §2.3 hook). New file `crates/selora-ui/src/presence/beam.rs`; `presence/mod.rs` gains `pub mod beam;` + the re-export (inside the presence builder's files — `selora-ui/src/lib.rs` stays untouched). Reference: https://libraries.dev/beam — "an animated glow that rides the border of any card, button or input"; props `size`, `colorVariant` (`mono` is the only variant that maps), `strength` 0–1 (scales alpha), `active` (pauses), `theme` (we are always dark).

### 8.1 Widget contract

```rust
pub struct BorderBeam { strength: f32, active: bool, reduce_motion: bool, held_phase: f32 }
impl BorderBeam {
    pub fn new() -> Self;                       // strength 1.0, inactive
    pub fn set_strength(&mut self, s: f32);     // clamps 0..1
    pub fn set_active(&mut self, on: bool);
    pub fn set_reduce_motion(&mut self, on: bool);
    pub fn show(&mut self, ui: &Ui, rect: Rect); // paint-only; requests frames only while animating
}
```

Stateful (the `StreamGlow` precedent, not the value-type `ThinkingOrb`): `held_phase` freezes the lap when `active` flips false — the library's "pauses the animation", ported faithfully. While inactive the beam **still paints at `held_phase`** (a paused instrument, not a hidden one); the composer simply never shows a paused beam (§8.6).

The library's `size` prop — a variant enum (`'md' | 'sm' | 'line' | 'pulse-inner' | 'pulse-outside'`) — **is subsumed by rect-driven geometry and is not ported as a prop**: the beam rides whatever rect it is given at the fixed 2px weight (the `'md'` reading). The two pulse variants would be a second ambient loop on a resting control — the ready shimmer already owns that job (§2.2), and §9's loop budget admits no more.

### 8.2 Geometry and paint (file-local constants)

- Path: the rounded-rect perimeter of `rect` at corner `radius::R2`, arc-length parameterized (top edge left→right, then clockwise; **phase 0 = the top-left corner**, traveling right).
- `BEAM_ARC_FRAC: f32 = 0.125` — the beam spans 1/8 of the perimeter (≈103px at the default panel width).
- `BEAM_SEGS: usize = 24` — painted as 24 short line segments (fixed arrays; the orb/avatar no-per-frame-`Vec` discipline, `orb.rs:425`).
- `BEAM_ALPHAS: [f32; 4] = [0.85, 0.50, 0.28, 0.14]` — four stepped alpha bands over the 24 segments, head bright → tail gone (the `GOO_ALPHA` precedent, `liquid.rs:36`). All segments `color::FG_DEFAULT`, 2px stroke, centered on the perimeter path. `strength` multiplies every alpha.
- Monochrome proof: only `FG_DEFAULT` at stepped alpha — no hue, no blur, no gradient (§12, same reading as the glow rings and the MetalMark sheen).

### 8.3 Motion

- One lap = `motion::DUR_LAP_MS` (2400ms — **the one new token**, §1), **phase eased `EASE_ORBITAL`** exactly like the splash orbit (`splash.rs:146-148`): seamless wrap, calmest at the seam. The eased travel is the test's authority: head arc-fraction = `ease_bezier(EASE_ORBITAL, t / 2.4s)` — at +600ms ≈ **0.071**, at +1200ms = **0.500**, at +1800ms ≈ **0.929** (verified by bisection of the token curve, `selora-tokens/src/lib.rs:132-149`, matching the implementation bit-for-bit).
- Rationale for 2400ms over 1200ms, recorded: the composer's perimeter at the 360px default panel is ≈ `2·(336+76)` = 824px; a 1200ms lap moves the head ≈0.69px/ms — visibly brisk against every other loop; halving it reads calm and keeps the §12 ≥1200ms floor trivially.
- While animating: `request_anim_frame` (the 20ms `ANIM_FRAME` cadence, `presence/mod.rs:48`).

### 8.4 States

| State | Beam |
|---|---|
| `active`, normal motion | rides the perimeter, eased phase from the frame clock |
| `active` flips false | freezes at `held_phase`, still painted, no repaint requests |
| `strength = 0` | paints nothing, requests nothing |
| Reduce-motion (while shown) | static segment at the top-left corner (phase 0), no repaint requests |

### 8.5 Where it may paint

**The composer card only.** Not the splash, not modals, not toasts — one signature surface, the MetalMark discipline (`metal.rs:9-11`).

### 8.6 Composer choreography (owner's, verbatim)

Idle shows the plain hairline; focus blooms the §2.2 glow; the beam rides on top while a request is in flight (plain stream or agent turn): `beam.set_active(self.in_flight.is_some())`, painted only while in flight, after the card stroke, before the StreamGlow band. A disconnect mid-flight does not freeze the beam (in-flight requests complete — `selora_status.rs:6-8`).

---

## 9. MOTION DISCIPLINE + STANDING REPAINT COST

- Loops never faster than `DUR_LOOP_MS` (1200ms): orb states, ready shimmer, splash orbit, MetalMark sheen, border beam (2400ms lap). One-shots: `DUR_QUICK` (ring fade), `DUR_BASE` (chip bloom, pill melt), `DUR_LONG` (spark, splash fade). No ninth glyph; nothing < 80ms; nothing > 2000ms except the §7.2-amended lap.
- Reduce-motion (§7.4 + the owner's stricter glow rule, §11.5): loops freeze; one-shot `fade`/`bloom` kept; glows (rings, shimmer, beam) snap/freeze.
- **Standing repaint cost (stated, accepted, locked by test)**: an idle open chat panel repaints at `ANIM_FRAME` cadence — the header orb breathes; an empty transcript adds the 64px hero; focused+connected adds the shimmer. ≈50fps perpetual while the chat panel is open — the price of "orbs you can actually see", the owner's stated intent. Bounds: disconnected pauses the orbs; closing the panel stops everything; reduce-motion stops everything; the beam adds frames only while a request is in flight. `live_idle_chat_requests_animation_frames` (`chat/mod.rs:5067`) locks this in.

---

## 10. TESTS (headless paint, the `presence/mod.rs:176` pattern)

**Present and green** (verified in the §0 run — 352 passing): composer `composer_card_paints_its_chrome_at_rest` (`chat/mod.rs:4742`), `focused_composer_paints_five_glow_rings` (`:4803`), `hover_without_focus_lifts_only_the_hairline` (`:4855`), `disconnected_focus_paints_rings_but_no_shimmer` (`:4886`), `send_button_arms_ink_mid` (`:4916`), `frameless_edit_paints_no_own_frame` (`:4973`), `reduce_motion_composer_requests_no_frames` (`:5017`), `live_idle_chat_requests_animation_frames` (`:5067`), `header_orb_matches_the_status_line_mapping` (`:5087`), `welcome_hero_is_the_64px_breathing_orb` (`:5190`), `welcome_chips_bloom_in_three_staggered_steps` (`:5217`), `welcome_chips_paint_the_1px_border` (`:5253`), `footer_layout_and_hint_fit_or_hide` (`:5322`); presence `liveness_off_suppresses_the_ok_satellite_at_avatar_scale` (`orb.rs:661`), `ready_shimmer_breathes_within_bounds` / `_freezes_under_reduce_motion` / `band_caps_follow_the_set_cap_radius` / `not_ready_idle_still_paints_nothing` (`stream_glow.rs:547,579,607,632`); chrome `selected_pill_paints_the_edge_mark` (`activity_bar.rs:227`), `selected_row_paints_the_edge_mark` (`file_tree.rs:681`), `menu_button_hover_paints_ink_faint_background` (`title_bar.rs:268`).

**Present and red — the §0.1 repairs**: `splash.rs:731` `focused_key_input_paints_the_focus_border` and `find_bar.rs:437` `focused_query_field_paints_the_focus_border` (the Area sizing-pass + fade-in mechanism; fix = settle frame + advancing headless clock); `splash.rs:704` `debug_dump_rect_strokes` (delete). **Any new Area-overlay shape test must settle one frame and advance `RawInput.time` past the fade-in before asserting exact colors** — this is now the stated harness rule, so nobody writes a fourth red test.

**New — presence builder (`beam.rs`)**:
- `beam_advances_one_eased_lap_per_dur_lap` — head arc-fraction at t equals `ease_bezier(EASE_ORBITAL, t / DUR_LAP)`: identical at t and t+2400ms; ≈0.071 at +600ms, 0.500 at +1200ms, ≈0.929 at +1800ms (tolerance ±0.02). **Not** linear — §8.3 is authoritative.
- `beam_paints_only_stepped_fg_alpha` — every segment gray (r=g=b), alpha ∈ the four bands × strength, 24 segments, head brightest.
- `inactive_beam_freezes_in_place_and_requests_nothing` — flip `active` false mid-lap: identical shapes across two later frames; `repaint_delay == Duration::MAX`.
- `reduce_motion_beam_is_a_static_top_left_segment` — identical shapes across times; the segment straddles the rect's top-left corner; `repaint_delay == Duration::MAX`.
- `zero_strength_paints_nothing`.

**New — composer builder (`chat/mod.rs`)**:
- `in_flight_composer_paints_the_beam` — `begin_stream` → 2px `FG_DEFAULT`-hue segments on the card's perimeter; the idle frame paints none.
- `beam_stops_when_the_stream_completes` — after `complete_stream`: no beam segments paint. **Do not assert `repaint_delay == Duration::MAX` here**: `complete_stream` fires `stream_glow.note_turn_done()` (`chat/mod.rs:1176`) — the one-shot `--ok` spark runs `SPARK_S` = `DUR_LONG` = 1800ms (`stream_glow.rs:45`) and requests frames, and the connected header orb breathes regardless (§9). Assert beam-shape absence only; repaint attribution is out of scope for this test.

**New — tokens founder**: `dur_lap_is_exactly_two_dur_loops` — `DUR_LAP_MS == 2 * DUR_LOOP_MS` (and ≥ `DUR_LOOP_MS`, the §12 loop floor).

---

## 11. OWNER SIGN-OFF (DESIGN_TOKENS §12 bends)

**Renumbering note (so no verifier chases a dead link)**: spec v2's sign-off section was §10; in v3+ it is §11 — the bend numbers are unchanged (#1 composer glow, #2 chip entrance, #3 edge marks, #4 ready shimmer, #5 reduce-motion glow precedence). In-code cites of "§10 bend #N" are stale and are corrected by each file's owner (§12): `chat/mod.rs:136-138` and `:145` (composer builder), `activity_bar.rs:30-32` and `file_tree.rs:55-56` (chrome builder).

1. **Composer rest/focus glow** (§2.2): layered painted alpha rings + resting shimmer are decorative-adjacent under §12 and neighbor §6's no-shadow rule. Lawful form: stepped alpha on `FG_FAINT`/`FG_MUTED`, no blur, no hue, 5 layers (brief: "at most 8"). **Signed off — the owner asked for "the ZCode glow effect".**
2. **Staggered chip entrance** (§4): multi-element choreography beyond §7.3's single-widget one-shots; kept inside the letter: the `bloom` glyph, `DUR_BASE`/`EASE_PRECISE`, 80ms steps, one-shot, re-runnable. **Signed off — the owner asked.**
3. **2px leading edge marks** (§7): §5 says "All borders are 1px. There are no 2px borders." The mark is a painted accent rect, not a border, and stays `FG_MUTED`. **Signed off — the owner's direction text specifies it.**
4. **Ready shimmer while focused** (§2.2): an ambient loop on a resting control; bounded: 1200ms, α ≤ 0.12, only while connected AND focused, frozen under reduce-motion. **Signed off — the owner asked for the rest-state "ready" shimmer.**
5. **Reduce-motion precedence for glows** (§2.2, §8.4): the rings' 160ms `fade` is dropped and the beam freezes under reduce-motion although §7.4 keeps `fade` — the owner's revamp rule ("reduce-motion freezes every loop and every glow to its static resting state") is the stricter reading and governs glows; §7.4's keep-list still governs the welcome `bloom` and the bubble fade-in. **Signed off — it is the owner's own motion rule, recorded so the two documents' conflict resolves one way, permanently.**
6. **The border beam** (§8): a decorative-adjacent perimeter glow; lawful form: stepped `FG_DEFAULT` alpha only (the library's `mono` variant is the only one that maps), 2px stroke riding the border (not a border — the #3 rect precedent), ≥1200ms loop, reduce-motion frozen, one surface only. **Signed off — the owner asked for the ZCode prompt-box effect by name.**
   - **6a. `--dur-lap` = 2400ms** (§1, §8.3): exceeds §7.2's "no duration over 2000ms" — the owner explicitly authorized "a perimeter-lap duration"; DESIGN_TOKENS.md §7.2 gains the row and its cap line is amended. **Signed off by the ask.**
7. **img-fx — the one deliberate non-port**: the seventh library is a WebGL pixel-mosaic image loader. **Verified this session: no image-display surface exists** — `grep -rn "egui::Image|TextureHandle|load_texture" crates --include=*.rs` and `grep -ni "image|texture" UI_SPEC.md` both return nothing. There is no surface for the port to land on; recorded so the omission is a decision, not an oversight. **Non-port, documented.**

Not bends: the 64px orb's `--ok` satellite (§1.3 live dot); XXL/600 heading (§2.2/§2.3); italic subline (§2.5); the standing repaint cost (§9 — a cost statement, not a decoration).

---

## 12. FILE OWNERSHIP MAP

Edit ONLY your files; anything else is off-limits.

**Build order (parallelism constraint)**: (0) **§0.1 baseline repair lands first** — presence and chrome builders each fix their own red test; (1) **tokens founder** lands `DUR_LAP_MS` — `presence/beam.rs` cannot compile without it (a file-local `LAP_S` would violate §1); (2) **presence builder** lands `beam.rs`; (3) **composer builder** wires the §2.3 hook. The chrome builder's §7 verify-only pass is independent of (1)–(3).

- **tokens founder**: `crates/selora-tokens/src/lib.rs` (`motion::DUR_LAP_MS` + its §10 test), `crates/selora-ui/src/theme.rs`, `DESIGN_TOKENS.md` (§7.2 row + cap amendment). Owns §1.
- **composer builder**: `crates/selora-ui/src/sidebar/chat/` (`mod.rs`, `agent_cards.rs`, `markdown.rs`). Owns §2 (including the §2.3 beam hook), §3, §4, §5; refreshes the stale "§10 bend" cites to §11 in `mod.rs:136-138,145`. The v2 `picker/model_picker.rs` exception (`set_on_card`) is landed (`:238`) — `picker/` is untouched.
- **presence builder**: `crates/selora-ui/src/presence/` (all files — **including the new `beam.rs`** and the `mod.rs` module line), `crates/selora-ui/src/splash.rs`, `crates/selora-ui/src/empty_state.rs`, `crates/selora-ui/src/brand.rs`. Owns §6, §8, and the §0.1 splash repairs (delete `debug_dump_rect_strokes`, fix the focus-border test).
- **chrome builder**: `crates/selora-ui/src/title_bar.rs`, `crates/selora-ui/src/status_bar/`, `crates/selora-ui/src/sidebar/activity_bar.rs`, `crates/selora-ui/src/sidebar/file_tree.rs`, `crates/selora-ui/src/sidebar/icons.rs`, `crates/selora-ui/src/sidebar/mod.rs`, `crates/selora-ui/src/editor/`, `crates/selora-ui/src/panel/`, `crates/selora-ui/src/toasts.rs`. Owns §7 (verify-only per the checklist, plus the §0.1 find_bar repair); refreshes the stale bend cites in `activity_bar.rs:30-32` and `file_tree.rs:55-56`.
- **Explicitly out of scope** (they already ride the theme): `dialogs/`, `palette/`, `diff_view.rs`, `picker/`. The §10 matrix reaches them in a later pass if the owner wants it.
- Nobody edits `crates/selora-ui/src/lib.rs` (no new top-level modules); nobody touches `selora-ai`, `selora-app`, `selora-core`, `selora-editor`, `selora-fs`, `selora-git`, `selora-lsp`, `selora-settings`.
