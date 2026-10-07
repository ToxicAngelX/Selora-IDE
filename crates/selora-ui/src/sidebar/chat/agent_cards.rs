//! Agent Mode cards in the chat sidebar (UI_SPEC.md §4.2) — Phase 5d's
//! file.
//!
//! Everything the agent loop paints between bubbles:
//!
//! - the collapsed read-type tool card (a 24px one-liner that expands to
//!   the tool result),
//! - the diff approval card for `write_file` / `edit_file` (unified diff,
//!   `-` in `--err` / `+` in `--ok` — the chat's only saturated uses,
//!   semantic per DESIGN_TOKENS.md §12),
//! - the delete approval card for `delete_file` (no diff body; §17.1's
//!   git-recovery caveat as the body line),
//! - the bash command card (`$` command line, spin + Stop while running,
//!   `✓ exit 0` / `✕ exit N` parsed from the router's `(exit code N)`
//!   prefix; auto-approved allowlisted commands never show buttons),
//! - the web cards (`web_search` expands to a numbered title+URL list —
//!   a row click copies the URL; `web_fetch` expands to the fetched text
//!   with the source URL in the header),
//! - the agent status line (`thinking` / `reading …` /
//!   `awaiting your approval` / `running …` + Stop on the right),
//! - the turn summary footer
//!   (`6 tools · 3 approved · 1 denied · 12.4k tokens · $0.041`).
//!
//! State in, events out: the chat panel (stage 2) reduces
//! `AppMessage::ToolCallStarted` / `ToolResult` / `ApprovalNeeded` /
//! `AgentTurnDone` (ARCHITECTURE.md §17.6) into [`ToolCard`]s and calls
//! [`tool_card`]; clicks come back on [`CardOutcome`] (a decision goes to
//! `ChatPanelEvent::AgentDecision`, a web-result copy to
//! `ChatPanelEvent::CopyCode`). The loop never appears here — no
//! channels, no threads, pure view state (ARCHITECTURE.md §1/§2).
//!
//! Bash output cannot stream: the frozen loop emits no output deltas
//! (`AgentEvent` has no output variant; `run_bash` returns one
//! `ToolResult` at completion), so the card shows the spin header while
//! running and renders the full output (256KB-capped, the router's
//! truncation notice included) when the result lands.
//!
//! Every color, font, spacing, duration and curve comes from
//! [`crate::theme`] (the typed DESIGN_TOKENS.md §11 contract); nothing is
//! hardcoded (§12). Cards fade in over `--dur-quick` on `--ease-fade`;
//! `pulse`/`spin` loop on `--dur-loop` (`--ease-orbital` / linear per
//! §7.3) and become static dots under `ui.reduce_motion` (§7.4) — the
//! pure helpers take the flag so both modes are unit-testable. While an
//! animation is live the painters call `Context::request_repaint_after`.

use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};

use selora_ai::{AgentDecision, AgentSummary, ApprovalCard, ApprovalId, ToolCall, ToolResult};

use super::{code_font, with_alpha};
use crate::presence::avatar::{AgentAvatar, AvatarState, AVATAR_INLINE};
use crate::presence::orb::{OrbState, ThinkingOrb, ORB_INLINE};
use crate::theme::{color, font, motion, radius, space, type_scale};

// -----------------------------------------------------------------------------
// Geometry (spec pixel values, cited — the HEADER_HEIGHT precedent in
// chat/mod.rs)
// -----------------------------------------------------------------------------

/// Collapsed card height (UI_SPEC.md §4.2: "One line, 24px tall").
pub const CARD_HEIGHT: f32 = 24.0;
/// Result / output body viewport cap (§4.2: "max 200px tall, scrolls").
pub const BODY_MAX_HEIGHT: f32 = 200.0;
/// Diff body viewport cap (§4.2: "Diff body max 240px tall, scrolls").
pub const DIFF_BODY_MAX_HEIGHT: f32 = 240.0;
/// The agent status line height (§4.2: "a 24px line").
pub const STATUS_LINE_HEIGHT: f32 = 24.0;
/// The turn summary footer row height.
pub const SUMMARY_LINE_HEIGHT: f32 = 24.0;
/// The decision-button row's button height.
const BUTTON_HEIGHT: f32 = 24.0;
/// The bash command row height (the `--type-md` line-height,
/// DESIGN_TOKENS.md §2.2).
const COMMAND_ROW_HEIGHT: f32 = 20.0;
/// One diff body row (the `--type-sm` line-height, §2.2).
const DIFF_ROW_HEIGHT: f32 = 18.0;
/// The "expand full diff" link row (the `--type-xs` line-height, §2.2).
const LINK_ROW_HEIGHT: f32 = 16.0;
/// A web_search result row: title line (the `--type-sm` line-height).
const WEB_TITLE_ROW: f32 = 18.0;
/// A web_search result row: URL line (the `--type-xs` line-height).
const WEB_URL_ROW: f32 = 16.0;
/// Card inner horizontal padding.
const CARD_PAD: f32 = space::S2;
/// How far the spin/pulse animations request the next frame (one
/// `--dur-loop` sixtieth ≈ 20ms, smooth without busy-looping).
const ANIM_FRAME_MS: u64 = motion::DUR_LOOP_MS as u64 / 60;
/// The status glyph's reserved slot at the right edge of a card header.
const GLYPH_SLOT: f32 = 16.0;
/// Phase 6h: the avatar's reserved slot at the LEFT edge of a card
/// header (the glyph plus its gap to the tool name).
const AVATAR_SLOT: f32 = AVATAR_INLINE + space::S1;
/// The status-line avatar's left inset.
const STATUS_AVATAR_INSET: f32 = space::S2;

/// The result-status glyphs (§4.2: "✓ in `--ok`, ✕ in `--err`"; the
/// `✕` precedent lives in the settings dialog's selora section).
const GLYPH_OK: &str = "✓";
const GLYPH_ERR: &str = "✕";
/// The name ─ summary separator in card headers (the §4.2 wireframes).
const GLYPH_SEPARATOR: &str = "─";
/// The bash command prefix (§4.2: "prefixed `$` in `--fg-faint`").
const GLYPH_DOLLAR: &str = "$";

/// Decision button labels.
const LABEL_DENY: &str = "Deny";
const LABEL_APPROVE_ALL: &str = "Approve all";
const LABEL_APPROVE: &str = "Approve";
const LABEL_STOP: &str = "Stop";

/// The "expand full diff" affordance (§4.2's "expand full file", renamed:
/// the card holds the router's diff, not the file).
const LABEL_EXPAND_DIFF: &str = "expand full diff";
/// The collapse half of the diff toggle.
const LABEL_COLLAPSE_DIFF: &str = "collapse diff";
/// The status-line texts (§4.2:300).
const TEXT_THINKING: &str = "thinking";
const TEXT_AWAITING: &str = "awaiting your approval";
/// A read-type card expanded before its result arrived.
const TEXT_WAITING: &str = "waiting for the result…";
/// The delete card's body (§17.1: "git-tracked files recoverable via
/// git").
const DELETE_BODY_PREFIX: &str = "Deletes ";
const DELETE_BODY_SUFFIX: &str = ". Git-tracked files stay recoverable through git.";

/// The raw-arguments fallback in [`summarize_call`] is capped so a
/// pathological argument blob cannot stall the layout pass.
const SUMMARY_FALLBACK_MAX: usize = 120;

// -----------------------------------------------------------------------------
// Card state (owned by the panel's timeline; rendered here)
// -----------------------------------------------------------------------------

/// One tool call's live card state. The panel builds it on
/// `AppMessage::ToolCallStarted`, attaches the approval on
/// `ApprovalNeeded` (with the turn's 1-based approval ordinal — "Approve
/// all" appears from the second, §4.2), latches the user's click into
/// [`decided`](Self::decided) (no double-fire), and lands the result on
/// `AppMessage::ToolResult`.
#[derive(Clone, Debug)]
pub struct ToolCard {
    /// The call the loop started (`ToolCallStarted`'s payload).
    pub call: ToolCall,
    /// The parked approval, when the tool is side-effecting: the id the
    /// decision returns with, the router's card body, and the 1-based
    /// ordinal in the turn's approval sequence.
    pub approval: Option<(ApprovalId, ApprovalCard, u32)>,
    /// The finished result, once it arrived.
    pub result: Option<ToolResult>,
    /// The user's decision, once clicked. The buttons render only while
    /// this is `None` — the latch is the no-double-fire guarantee.
    pub decided: Option<AgentDecision>,
    /// Collapsed ↔ expanded (the header row toggles it); on the diff
    /// card it is the "expand full diff" toggle (the 240px cap).
    pub expanded: bool,
    /// When the card appeared (the fade-in anchor).
    pub appeared: Instant,
    /// Phase 6h: the header's agent identity glyph (its cross-fade clock
    /// lives here; the pose is derived per frame from the card state).
    pub avatar: AgentAvatar,
}

impl ToolCard {
    /// A fresh collapsed card for a just-started call.
    pub fn new(call: ToolCall) -> Self {
        let avatar = AgentAvatar::new(seed_from_id(&call.id));
        Self {
            call,
            approval: None,
            result: None,
            decided: None,
            expanded: false,
            appeared: Instant::now(),
            avatar,
        }
    }

    /// The approval id while the card carries one (pending or decided);
    /// the status-line Stop rides it (`turn.rs` honors Stop regardless of
    /// the id it names).
    pub fn approval_id(&self) -> Option<ApprovalId> {
        self.approval.as_ref().map(|(id, _, _)| *id)
    }

    /// `true` while the loop is parked on this card's approval (§17.4).
    pub fn is_pending(&self) -> bool {
        self.approval.is_some() && self.decided.is_none() && self.result.is_none()
    }
}

/// What the status line paints (§4.2:300). Derived from the turn's card
/// states by [`status_phase_for`] — the loop's `AgentEvent::State` is not
/// forwarded to the panel, so the card states are the source of truth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusPhase {
    /// Streaming the model's reply (`pulse` dot).
    Thinking,
    /// A tool is executing; the string is the verb line (`reading
    /// src/main.rs`, `running cargo test`) — `spin` glyph.
    Running(String),
    /// A card awaits the user's decision (`pulse`).
    AwaitingApproval,
}

/// Derive the status-line phase from the turn's cards (in arrival
/// order): a pending-undecided card wins (`awaiting your approval`);
/// otherwise the latest resultless card's verb line; otherwise
/// `thinking`. The panel calls this only while a turn is in flight.
pub fn status_phase_for<'a>(cards: impl IntoIterator<Item = &'a ToolCard>) -> StatusPhase {
    let mut last_running: Option<&ToolCard> = None;
    for card in cards {
        if card.is_pending() {
            return StatusPhase::AwaitingApproval;
        }
        if card.result.is_none() {
            last_running = Some(card);
        }
    }
    match last_running {
        Some(card) => StatusPhase::Running(status_verb(&card.call)),
        None => StatusPhase::Thinking,
    }
}

/// The running tool's name for the status line's orb (Phase 6h):
/// `None` when [`status_phase_for`] would not say `Running` (a pending
/// card parks the phase on AwaitingApproval; nothing running reads as
/// Thinking). Read-type vs write/bash drives searching vs solving.
pub fn status_running_tool<'a>(cards: impl IntoIterator<Item = &'a ToolCard>) -> Option<&'a str> {
    let mut last_running: Option<&ToolCard> = None;
    for card in cards {
        if card.is_pending() {
            return None; // the phase is AwaitingApproval, not Running
        }
        if card.result.is_none() {
            last_running = Some(card);
        }
    }
    last_running.map(|card| card.call.name.as_str())
}

/// The card header avatar's state (Phase 6h): sleeping while the gateway
/// is disconnected, working while the tool executes on a live turn,
/// default otherwise (a parked card waits patiently).
fn card_avatar_state(card: &ToolCard, turn_live: bool, gateway_connected: bool) -> AvatarState {
    if !gateway_connected {
        AvatarState::Sleeping
    } else if card.result.is_none() && !card.is_pending() && turn_live {
        AvatarState::Working
    } else {
        AvatarState::Default
    }
}

/// The avatar's roster seed from its card's call id (deterministic —
/// a row of cards never blinks in unison).
fn seed_from_id(id: &str) -> f32 {
    let mut hash = 0x811c9dc5u32;
    for byte in id.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    (hash % 1000) as f32 / 1000.0
}

// -----------------------------------------------------------------------------
// Outcomes (the headless-test aid pattern — ChatPanelOutcome's precedent)
// -----------------------------------------------------------------------------

/// What one [`tool_card`] paint produced. The panel folds `decision`
/// into `ChatPanelEvent::AgentDecision` (with the card's approval id) and
/// `copy` into `ChatPanelEvent::CopyCode` (the copy+toast seam).
#[derive(Debug)]
pub struct CardOutcome {
    /// A decision-button click this frame.
    pub decision: Option<AgentDecision>,
    /// A web_search row click — the URL to copy.
    pub copy: Option<String>,
    /// The card's outer rect.
    pub rect: Rect,
    /// The header row's rect (the expand toggle's click target).
    pub header_rect: Rect,
    /// `(label, rect)` for the decision buttons painted this frame.
    pub button_rects: Vec<(&'static str, Rect)>,
    /// The expanded body's rect, when painted.
    pub body_rect: Option<Rect>,
    /// The web_search result rows painted this frame (click = copy URL).
    pub row_rects: Vec<Rect>,
}

impl Default for CardOutcome {
    fn default() -> Self {
        Self {
            decision: None,
            copy: None,
            rect: Rect::ZERO,
            header_rect: Rect::ZERO,
            button_rects: Vec::new(),
            body_rect: None,
            row_rects: Vec::new(),
        }
    }
}

/// What one [`status_line`] paint produced.
#[derive(Debug)]
pub struct StatusLineOutcome {
    /// The Stop button was clicked this frame.
    pub stop_clicked: bool,
    /// The line's rect.
    pub rect: Rect,
    /// The Stop button's rect.
    pub stop_rect: Rect,
}

// -----------------------------------------------------------------------------
// Entry points
// -----------------------------------------------------------------------------

/// Paint one tool card (every kind dispatches here). `reduce_motion` is
/// `ui.reduce_motion` (DESIGN_TOKENS.md §7.4): pulse/spin become static
/// dots. `turn_live` is whether the card's own turn is still in flight —
/// a turn that ended while a tool ran (Stop/cap: the loop breaks without
/// a ToolResult for it) must freeze the card's spin and kill its Stop
/// button, or the card would request repaints forever. `gateway_connected`
/// drives the header avatar's sleeping state (Phase 6h).
pub fn tool_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    // Every card fades in over --dur-quick (§4.2:302); fade is kept
    // under reduced motion (§7.4).
    let elapsed = card.appeared.elapsed();
    let fade = fade_alpha(elapsed);
    if fade < 1.0 {
        ui.ctx().request_repaint_after(
            Duration::from_millis(motion::DUR_QUICK_MS as u64).saturating_sub(elapsed),
        );
    }

    match card_kind(card) {
        CardKind::Bash => {
            paint_bash_card(ui, card, fade, reduce_motion, turn_live, gateway_connected)
        }
        CardKind::DiffApproval => {
            paint_diff_card(ui, card, fade, reduce_motion, turn_live, gateway_connected)
        }
        CardKind::DeleteApproval => {
            paint_delete_card(ui, card, fade, reduce_motion, turn_live, gateway_connected)
        }
        CardKind::WebSearch => {
            paint_web_search_card(ui, card, fade, reduce_motion, turn_live, gateway_connected)
        }
        CardKind::WebFetch => {
            paint_web_fetch_card(ui, card, fade, reduce_motion, turn_live, gateway_connected)
        }
        CardKind::Collapsed => {
            paint_collapsed_card(ui, card, fade, reduce_motion, turn_live, gateway_connected)
        }
    }
}

/// The 24px agent status line (§4.2:300): the AgentAvatar, the
/// ThinkingOrb at the loop-mapped state, the phase text, and the Stop
/// button on the right (`--type-xs --fg-muted`, hover `--err`). Phase
/// 6h: the avatar + orb replace 5d's pulse/spin dot. The avatar sleeps
/// while the gateway is disconnected.
pub fn status_line(
    ui: &mut Ui,
    phase: &StatusPhase,
    orb: OrbState,
    avatar: &mut AgentAvatar,
    gateway_connected: bool,
    reduce_motion: bool,
) -> StatusLineOutcome {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), STATUS_LINE_HEIGHT),
        Sense::hover(),
    );
    let now = crate::presence::clock_s(ui.ctx());

    // The identity glyph (the status line only exists mid-turn: working,
    // or sleeping while the gateway is down).
    let avatar_rect = Rect::from_min_size(
        pos2(
            rect.left() + STATUS_AVATAR_INSET,
            rect.center().y - AVATAR_INLINE / 2.0,
        ),
        vec2(AVATAR_INLINE, AVATAR_INLINE),
    );
    avatar.set_reduce_motion(reduce_motion);
    avatar.set_state(
        if gateway_connected {
            AvatarState::Working
        } else {
            AvatarState::Sleeping
        },
        now,
    );
    avatar.paint(ui, avatar_rect, 1.0, true);

    // The thought indicator.
    let orb_rect = Rect::from_min_size(
        pos2(
            avatar_rect.right() + space::S1,
            rect.center().y - ORB_INLINE / 2.0,
        ),
        vec2(ORB_INLINE, ORB_INLINE),
    );
    ThinkingOrb::new(orb, ORB_INLINE)
        .reduce_motion(reduce_motion)
        .paint(ui, orb_rect);

    let text = match phase {
        StatusPhase::Thinking => TEXT_THINKING,
        StatusPhase::AwaitingApproval => TEXT_AWAITING,
        StatusPhase::Running(verb) => verb.as_str(),
    };
    ui.painter().text(
        pos2(orb_rect.right() + space::S2, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        sans_font(type_scale::SM),
        color::FG_MUTED,
    );

    // Stop (right): --type-xs --fg-muted, hover --err (§4.2:300 — the
    // spec's one saturated hover, semantic for a destructive action).
    let galley = ui.painter().layout_no_wrap(
        LABEL_STOP.to_owned(),
        sans_font(type_scale::XS),
        color::FG_MUTED,
    );
    let stop_width = galley.size().x + 2.0 * space::S1;
    let stop_rect = Rect::from_min_size(
        pos2(
            rect.right() - space::S2 - stop_width,
            rect.center().y - BUTTON_HEIGHT / 2.0,
        ),
        vec2(stop_width, BUTTON_HEIGHT),
    );
    let stop_response = ui.interact(
        stop_rect,
        egui::Id::new("selora_agent_status_stop"),
        Sense::click(),
    );
    ui.painter().text(
        stop_rect.center(),
        Align2::CENTER_CENTER,
        LABEL_STOP,
        sans_font(type_scale::XS),
        if stop_response.hovered() {
            color::ERR
        } else {
            color::FG_MUTED
        },
    );

    // The avatar and the orb request their own animation frames (the
    // presence widgets' discipline — nothing here repaints on its own).
    StatusLineOutcome {
        stop_clicked: stop_response.clicked(),
        rect,
        stop_rect,
    }
}

/// The turn summary footer (§4.2: `6 tools · 3 approved · 1 denied ·
/// 12.4k tokens · $0.041`) — `--type-xs --fg-faint`, left-aligned under
/// the final bubble. Returns the painted rect (the test aid).
pub fn summary_footer(ui: &mut Ui, summary: &AgentSummary) -> Rect {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), SUMMARY_LINE_HEIGHT),
        Sense::hover(),
    );
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        summary_line(summary),
        sans_font(type_scale::XS),
        color::FG_FAINT,
    );
    rect
}

/// The footer's text, pure for tests.
pub fn summary_line(summary: &AgentSummary) -> String {
    format!(
        "{} tool{} · {} approved · {} denied · {} tokens · ${:.3}",
        summary.tools_used,
        super::plural(summary.tools_used as usize),
        summary.approved,
        summary.denied,
        format_tokens(summary.tokens),
        summary.cost_usd,
    )
}

/// `12400` → `"12.4k"` (the footer's token count).
pub fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

// -----------------------------------------------------------------------------
// Card kinds (dispatch)
// -----------------------------------------------------------------------------

/// Which layout a card renders in, from its current state.
enum CardKind {
    /// `bash` — command row + output + buttons/Stop (pending/running/
    /// finished; auto-approved renders no buttons ever).
    Bash,
    /// A `write_file` / `edit_file` diff card before its result lands
    /// (pending or decided-executing). Post-result it collapses to
    /// [`CardKind::Collapsed`].
    DiffApproval,
    /// A `delete_file` card before its result lands. No diff body, no
    /// saturated color.
    DeleteApproval,
    /// `web_search` — collapsed one-liner, expands to the result list.
    WebSearch,
    /// `web_fetch` — collapsed one-liner, expands to the fetched text.
    WebFetch,
    /// Read-type tools and every finished side-effecting card: the 24px
    /// one-liner, expanding to the result (or, for a finished
    /// write/edit, to the approved diff).
    Collapsed,
}

fn card_kind(card: &ToolCard) -> CardKind {
    if card.call.name == "bash" {
        return CardKind::Bash;
    }
    match &card.approval {
        Some((_, ApprovalCard::Diff { .. }, _)) if card.result.is_none() => CardKind::DiffApproval,
        Some((_, ApprovalCard::Delete { .. }, _)) if card.result.is_none() => {
            CardKind::DeleteApproval
        }
        _ => match card.call.name.as_str() {
            "web_search" => CardKind::WebSearch,
            "web_fetch" => CardKind::WebFetch,
            _ => CardKind::Collapsed,
        },
    }
}

// -----------------------------------------------------------------------------
// The collapsed one-liner (read-type tools, finished side-effecting cards)
// -----------------------------------------------------------------------------

fn paint_collapsed_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    let mut outcome = CardOutcome::default();
    let width = ui.available_width();

    // The expand body: a finished write/edit shows the diff it was
    // approved on; every other card shows the tool result (§4.2:249).
    let body = if card.expanded {
        match &card.approval {
            Some((_, ApprovalCard::Diff { diff, .. }, _)) => Body::Diff(parse_unified_diff(diff)),
            _ => match &card.result {
                Some(result) => Body::Text(result.content.clone()),
                None => Body::Text(TEXT_WAITING.to_owned()),
            },
        }
    } else {
        Body::None
    };
    let body_height = body_height(ui, &body, width, BODY_MAX_HEIGHT, fade);

    let total_height = CARD_HEIGHT + body_height;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_height), Sense::hover());
    outcome.rect = rect;
    paint_chrome(ui, rect, fade);

    let header = Rect::from_min_size(rect.min, vec2(width, CARD_HEIGHT));
    let header_response = ui.interact(
        header,
        egui::Id::new(("selora_agent_card_header", &card.call.id)),
        Sense::click(),
    );
    if header_response.clicked() {
        card.expanded = !card.expanded;
    }
    paint_card_header(
        ui,
        header,
        card,
        None,
        fade,
        reduce_motion,
        turn_live,
        gateway_connected,
    );
    outcome.header_rect = header;

    if let Some(body_rect) = body_rect(rect, body_height) {
        paint_body(ui, &body, body_rect, &card.call.id, fade, &mut outcome);
        outcome.body_rect = Some(body_rect);
    }
    if card.result.is_none() && !reduce_motion && turn_live {
        // The running spin in the header's glyph slot.
        request_animation_frame(ui.ctx());
    }
    outcome
}

// -----------------------------------------------------------------------------
// The diff approval card (write_file / edit_file)
// -----------------------------------------------------------------------------

fn paint_diff_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    let mut outcome = CardOutcome::default();
    let width = ui.available_width();
    let (path, diff) = match &card.approval {
        Some((_, ApprovalCard::Diff { path, diff }, _)) => (path.clone(), diff.clone()),
        _ => (String::new(), String::new()), // kind() guards this
    };
    let lines = parse_unified_diff(&diff);
    let full_height = lines.len() as f32 * DIFF_ROW_HEIGHT;
    let body_height = if card.expanded {
        full_height
    } else {
        full_height.min(DIFF_BODY_MAX_HEIGHT)
    };
    // §4.2:267's "expand full file" link, renamed — the card holds the
    // router's diff, not the file. Shown while the diff exceeds the cap.
    let link_height = if full_height > DIFF_BODY_MAX_HEIGHT {
        LINK_ROW_HEIGHT
    } else {
        0.0
    };
    let pending = card.is_pending();
    let buttons_height = if pending {
        BUTTON_HEIGHT + space::S2
    } else {
        0.0
    };

    let total_height = CARD_HEIGHT + body_height + link_height + buttons_height;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_height), Sense::hover());
    outcome.rect = rect;
    paint_chrome(ui, rect, fade);

    let header = Rect::from_min_size(rect.min, vec2(width, CARD_HEIGHT));
    paint_card_header(
        ui,
        header,
        card,
        Some(&path),
        fade,
        reduce_motion,
        turn_live,
        gateway_connected,
    );
    outcome.header_rect = header;

    let body = Body::Diff(lines);
    let mut cursor = rect.top() + CARD_HEIGHT;
    if body_height > 0.0 {
        let body_rect = Rect::from_min_size(pos2(rect.left(), cursor), vec2(width, body_height));
        paint_body(ui, &body, body_rect, &card.call.id, fade, &mut outcome);
        outcome.body_rect = Some(body_rect);
        cursor += body_height;
    }

    if link_height > 0.0 {
        let link_rect = Rect::from_min_size(pos2(rect.left(), cursor), vec2(width, link_height));
        let label = if card.expanded {
            LABEL_COLLAPSE_DIFF
        } else {
            LABEL_EXPAND_DIFF
        };
        let response = ui.interact(
            link_rect,
            egui::Id::new(("selora_agent_diff_expand", &card.call.id)),
            Sense::click(),
        );
        ui.painter().text(
            pos2(link_rect.left() + CARD_PAD, link_rect.center().y),
            Align2::LEFT_CENTER,
            label,
            sans_font(type_scale::XS),
            with_alpha(
                if response.hovered() {
                    color::FG_DEFAULT
                } else {
                    color::FG_MUTED
                },
                fade,
            ),
        );
        if response.clicked() {
            card.expanded = !card.expanded;
        }
        cursor += link_height;
    }

    if pending {
        let row = Rect::from_min_size(
            pos2(rect.left(), cursor + space::S2),
            vec2(width, BUTTON_HEIGHT),
        );
        if let Some(decision) = paint_decision_row(ui, row, card, fade, &mut outcome) {
            card.decided = Some(decision);
            outcome.decision = Some(decision);
        }
    }
    outcome
}

// -----------------------------------------------------------------------------
// The delete approval card (delete_file) — no diff body, no saturation
// -----------------------------------------------------------------------------

fn paint_delete_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    let mut outcome = CardOutcome::default();
    let width = ui.available_width();
    let path = match &card.approval {
        Some((_, ApprovalCard::Delete { path }, _)) => path.clone(),
        _ => summarize_call(&card.call), // kind() guards this
    };
    let body_text = format!("{DELETE_BODY_PREFIX}{path}{DELETE_BODY_SUFFIX}");
    let wrap = (width - 2.0 * CARD_PAD).max(20.0);
    let galley = ui.painter().layout(
        body_text,
        sans_font(type_scale::SM),
        with_alpha(color::FG_MUTED, fade),
        wrap,
    );
    let body_height = galley.size().y + 2.0 * space::S1;
    let pending = card.is_pending();
    let buttons_height = if pending {
        BUTTON_HEIGHT + space::S2
    } else {
        0.0
    };

    let total_height = CARD_HEIGHT + body_height + buttons_height;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_height), Sense::hover());
    outcome.rect = rect;
    paint_chrome(ui, rect, fade);

    let header = Rect::from_min_size(rect.min, vec2(width, CARD_HEIGHT));
    paint_card_header(
        ui,
        header,
        card,
        Some(&path),
        fade,
        reduce_motion,
        turn_live,
        gateway_connected,
    );
    outcome.header_rect = header;

    let body_rect = Rect::from_min_size(
        pos2(rect.left(), rect.top() + CARD_HEIGHT),
        vec2(width, body_height),
    );
    ui.painter().galley(
        pos2(body_rect.left() + CARD_PAD, body_rect.top() + space::S1),
        galley,
        with_alpha(color::FG_MUTED, fade),
    );
    outcome.body_rect = Some(body_rect);

    if pending {
        let row = Rect::from_min_size(
            pos2(rect.left(), body_rect.bottom() + space::S2),
            vec2(width, BUTTON_HEIGHT),
        );
        if let Some(decision) = paint_decision_row(ui, row, card, fade, &mut outcome) {
            card.decided = Some(decision);
            outcome.decision = Some(decision);
        }
    }
    outcome
}

// -----------------------------------------------------------------------------
// The bash command card
// -----------------------------------------------------------------------------

fn paint_bash_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    let mut outcome = CardOutcome::default();
    let width = ui.available_width();
    let command = bash_command(card);
    let pending = card.is_pending();
    // Approved and executing, or auto-approved (no card ever arrived) —
    // and only while the card's own turn is still in flight (a turn that
    // ended mid-run froze the card; no spin, no Stop, no repaint loop).
    let running = card.result.is_none() && !pending && turn_live;
    let stop_row = running && card.approval.is_some();

    let output = card
        .result
        .as_ref()
        .map(|result| bash_output_body(&result.content).to_owned());
    let body = match &output {
        Some(text) => Body::Text(text.clone()),
        None => Body::None,
    };
    let body_height = body_height(ui, &body, width, BODY_MAX_HEIGHT, fade);
    let buttons_height = if pending || stop_row {
        BUTTON_HEIGHT + space::S2
    } else {
        0.0
    };

    let total_height = CARD_HEIGHT + COMMAND_ROW_HEIGHT + body_height + buttons_height;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_height), Sense::hover());
    outcome.rect = rect;
    paint_chrome(ui, rect, fade);

    // Header: the identity glyph, `bash` left; right — spin while
    // running, the exit status once finished (✓ exit 0 / ✕ exit N, §4.2).
    let header = Rect::from_min_size(rect.min, vec2(width, CARD_HEIGHT));
    {
        let now = crate::presence::clock_s(ui.ctx());
        let avatar_rect = Rect::from_min_size(
            pos2(
                header.left() + CARD_PAD,
                header.center().y - AVATAR_INLINE / 2.0,
            ),
            vec2(AVATAR_INLINE, AVATAR_INLINE),
        );
        card.avatar.set_reduce_motion(reduce_motion);
        card.avatar
            .set_state(card_avatar_state(card, turn_live, gateway_connected), now);
        card.avatar.paint(ui, avatar_rect, fade, turn_live);

        let painter = ui.painter();
        let clip = Rect::from_min_max(
            pos2(header.left() + CARD_PAD + AVATAR_SLOT, header.top()),
            pos2(header.right() - CARD_PAD, header.bottom()),
        );
        let p = painter.with_clip_rect(clip);
        p.text(
            pos2(clip.left(), header.center().y),
            Align2::LEFT_CENTER,
            &card.call.name,
            sans_font(type_scale::XS),
            with_alpha(color::FG_FAINT, fade),
        );
    }
    paint_bash_status(ui, header, card, fade, reduce_motion, turn_live);
    outcome.header_rect = header;

    // The command row: `$` --fg-faint + the command --type-md mono.
    let command_rect = Rect::from_min_size(
        pos2(rect.left(), rect.top() + CARD_HEIGHT),
        vec2(width, COMMAND_ROW_HEIGHT),
    );
    {
        let painter = ui.painter();
        let clip = command_rect.shrink2(vec2(CARD_PAD, 0.0));
        let p = painter.with_clip_rect(clip);
        let dollar = p.layout_no_wrap(
            GLYPH_DOLLAR.to_owned(),
            mono_font(type_scale::MD),
            with_alpha(color::FG_FAINT, fade),
        );
        let y = command_rect.center().y;
        p.text(
            pos2(clip.left(), y),
            Align2::LEFT_CENTER,
            GLYPH_DOLLAR,
            mono_font(type_scale::MD),
            with_alpha(color::FG_FAINT, fade),
        );
        p.text(
            pos2(clip.left() + dollar.size().x + space::S1, y),
            Align2::LEFT_CENTER,
            command,
            mono_font(type_scale::MD),
            with_alpha(color::FG_DEFAULT, fade),
        );
    }

    let mut cursor = command_rect.bottom();
    if let Some(body_rect) = body_rect(rect, body_height).map(|mut r| {
        r.min.y = cursor;
        r
    }) {
        paint_body(ui, &body, body_rect, &card.call.id, fade, &mut outcome);
        outcome.body_rect = Some(body_rect);
        cursor += body_height;
    }

    if pending {
        let row = Rect::from_min_size(
            pos2(rect.left(), cursor + space::S2),
            vec2(width, BUTTON_HEIGHT),
        );
        if let Some(decision) = paint_decision_row(ui, row, card, fade, &mut outcome) {
            card.decided = Some(decision);
            outcome.decision = Some(decision);
        }
    } else if stop_row {
        // A running approved command: Stop replaces Approve (§4.2:288).
        let row = Rect::from_min_size(
            pos2(rect.left(), cursor + space::S2),
            vec2(width, BUTTON_HEIGHT),
        );
        if paint_stop_button(ui, row, card, fade, &mut outcome) {
            outcome.decision = Some(AgentDecision::Stop);
        }
    }
    if running && !reduce_motion {
        request_animation_frame(ui.ctx());
    }
    outcome
}

/// The bash header's right side: spin while running, the parsed exit
/// status once the result landed.
fn paint_bash_status(
    ui: &mut Ui,
    header: Rect,
    card: &ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
) {
    let center = pos2(
        header.right() - CARD_PAD - GLYPH_SLOT / 2.0,
        header.center().y,
    );
    let painter = ui.painter();
    let elapsed_ms = (ui.ctx().input(|i| i.time) * 1000.0) as u64;
    match &card.result {
        None => {
            if card.approval.is_some() && card.decided.is_none() {
                return; // parked: no glyph until the decision
            }
            if turn_live {
                paint_spin(painter, center, elapsed_ms, reduce_motion);
            } else {
                // The turn ended mid-run (Stop/cap): the §7.4 static dot.
                painter.circle_filled(center, 3.0, with_alpha(color::FG_MUTED, fade));
            }
        }
        Some(result) => {
            let (glyph, glyph_color, note) = match parse_bash_exit(&result.content) {
                Some(0) => (GLYPH_OK, color::OK, " exit 0".to_owned()),
                Some(code) => (GLYPH_ERR, color::ERR, format!(" exit {code}")),
                None if result.ok => (GLYPH_OK, color::OK, String::new()),
                None => (GLYPH_ERR, color::ERR, String::new()),
            };
            // The glyph carries the saturation; the note stays ink.
            let note_galley = painter.layout_no_wrap(
                note.clone(),
                sans_font(type_scale::XS),
                with_alpha(color::FG_FAINT, fade),
            );
            painter.text(
                pos2(header.right() - CARD_PAD, header.center().y),
                Align2::RIGHT_CENTER,
                glyph,
                sans_font(type_scale::XS),
                with_alpha(glyph_color, fade),
            );
            let glyph_width = painter
                .layout_no_wrap(
                    glyph.to_owned(),
                    sans_font(type_scale::XS),
                    with_alpha(glyph_color, fade),
                )
                .size()
                .x;
            if !note.is_empty() {
                painter.galley(
                    pos2(
                        header.right() - CARD_PAD - glyph_width - space::S1 - note_galley.size().x,
                        header.center().y - note_galley.size().y / 2.0,
                    ),
                    note_galley,
                    with_alpha(color::FG_FAINT, fade),
                );
            }
        }
    }
}

/// The bash card's command: the parked card's verbatim command, else the
/// call's `command` argument, else the raw-arguments fallback.
fn bash_command(card: &ToolCard) -> String {
    if let Some((_, ApprovalCard::Command { command }, _)) = &card.approval {
        return command.clone();
    }
    serde_json::from_str::<serde_json::Value>(&card.call.arguments)
        .ok()
        .and_then(|args| args.get("command")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| summarize_call(&card.call))
}

/// The bash result's output: the `(exit code N)\n` prefix line (bash.rs)
/// is shown in the header, not the body.
fn bash_output_body(content: &str) -> &str {
    if let Some(rest) = content.strip_prefix("(exit code ") {
        if let Some(close) = rest.find(')') {
            let after = &rest[close + 1..];
            return after.strip_prefix('\n').unwrap_or(after);
        }
    }
    content
}

// -----------------------------------------------------------------------------
// The web cards
// -----------------------------------------------------------------------------

fn paint_web_search_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    let mut outcome = CardOutcome::default();
    let width = ui.available_width();

    let body = if card.expanded {
        match &card.result {
            Some(result) => match parse_web_search_results(&result.content) {
                WebSearchBody::Results(rows) => Body::WebRows(rows),
                WebSearchBody::Raw(text) => Body::Text(text),
            },
            None => Body::Text(TEXT_WAITING.to_owned()),
        }
    } else {
        Body::None
    };
    let body_height = body_height(ui, &body, width, BODY_MAX_HEIGHT, fade);

    let total_height = CARD_HEIGHT + body_height;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_height), Sense::hover());
    outcome.rect = rect;
    paint_chrome(ui, rect, fade);

    let header = Rect::from_min_size(rect.min, vec2(width, CARD_HEIGHT));
    let header_response = ui.interact(
        header,
        egui::Id::new(("selora_agent_card_header", &card.call.id)),
        Sense::click(),
    );
    if header_response.clicked() {
        card.expanded = !card.expanded;
    }
    paint_card_header(
        ui,
        header,
        card,
        None,
        fade,
        reduce_motion,
        turn_live,
        gateway_connected,
    );
    outcome.header_rect = header;

    if let Some(body_rect) = body_rect(rect, body_height) {
        paint_body(ui, &body, body_rect, &card.call.id, fade, &mut outcome);
        outcome.body_rect = Some(body_rect);
    }
    if card.result.is_none() && !reduce_motion && turn_live {
        request_animation_frame(ui.ctx());
    }
    outcome
}

fn paint_web_fetch_card(
    ui: &mut Ui,
    card: &mut ToolCard,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) -> CardOutcome {
    let mut outcome = CardOutcome::default();
    let width = ui.available_width();

    // The header shows the fetched page's final URL (web.rs's `Source:`
    // line) once the result landed; before that, the call's argument.
    let (source, body_text) = match &card.result {
        Some(result) => split_web_fetch_source(&result.content),
        None => (None, String::new()),
    };

    let body = if card.expanded {
        match &card.result {
            Some(_) => Body::Text(body_text),
            None => Body::Text(TEXT_WAITING.to_owned()),
        }
    } else {
        Body::None
    };
    let body_height = body_height(ui, &body, width, BODY_MAX_HEIGHT, fade);

    let total_height = CARD_HEIGHT + body_height;
    let (rect, _) = ui.allocate_exact_size(vec2(width, total_height), Sense::hover());
    outcome.rect = rect;
    paint_chrome(ui, rect, fade);

    let header = Rect::from_min_size(rect.min, vec2(width, CARD_HEIGHT));
    let header_response = ui.interact(
        header,
        egui::Id::new(("selora_agent_card_header", &card.call.id)),
        Sense::click(),
    );
    if header_response.clicked() {
        card.expanded = !card.expanded;
    }
    paint_card_header(
        ui,
        header,
        card,
        source.as_deref(),
        fade,
        reduce_motion,
        turn_live,
        gateway_connected,
    );
    outcome.header_rect = header;

    if let Some(body_rect) = body_rect(rect, body_height) {
        paint_body(ui, &body, body_rect, &card.call.id, fade, &mut outcome);
        outcome.body_rect = Some(body_rect);
    }
    if card.result.is_none() && !reduce_motion && turn_live {
        request_animation_frame(ui.ctx());
    }
    outcome
}

// -----------------------------------------------------------------------------
// Shared chrome
// -----------------------------------------------------------------------------

/// The card's background and 1px hairline (§4.2: "`--ink-canvas`
/// background, 1px `--ink-hi` border, `--radius-1`").
fn paint_chrome(ui: &mut Ui, rect: Rect, fade: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, radius::R1, with_alpha(color::INK_CANVAS, fade));
    painter.rect(
        rect,
        radius::R1,
        Color32::TRANSPARENT,
        egui::Stroke::new(1.0, with_alpha(color::INK_HI, fade)),
        egui::StrokeKind::Inside,
    );
}

/// The card header: the agent identity glyph (Phase 6h), tool name
/// (`--type-xs --fg-faint`), ` ─ ` separator, argument summary
/// (`--type-sm --fg-muted`), and the status glyph on the right (✓
/// `--ok` / ✕ `--err`, or the spin while the tool runs).
/// `summary_override` replaces the argument summary (the diff/delete
/// cards' path, the fetched page's final URL).
#[allow(clippy::too_many_arguments)]
fn paint_card_header(
    ui: &mut Ui,
    header: Rect,
    card: &mut ToolCard,
    summary_override: Option<&str>,
    fade: f32,
    reduce_motion: bool,
    turn_live: bool,
    gateway_connected: bool,
) {
    // The identity glyph at the header's left edge (its pose derives
    // from the card's state; a dead turn freezes it).
    let now = crate::presence::clock_s(ui.ctx());
    let avatar_rect = Rect::from_min_size(
        pos2(
            header.left() + CARD_PAD,
            header.center().y - AVATAR_INLINE / 2.0,
        ),
        vec2(AVATAR_INLINE, AVATAR_INLINE),
    );
    card.avatar.set_reduce_motion(reduce_motion);
    card.avatar
        .set_state(card_avatar_state(card, turn_live, gateway_connected), now);
    card.avatar.paint(ui, avatar_rect, fade, turn_live);

    let painter = ui.painter();
    let clip = Rect::from_min_max(
        pos2(header.left() + CARD_PAD + AVATAR_SLOT, header.top()),
        pos2(header.right() - CARD_PAD - GLYPH_SLOT, header.bottom()),
    );
    let p = painter.with_clip_rect(clip);
    let y = header.center().y;
    let mut x = clip.left();

    let name = p.layout_no_wrap(
        card.call.name.clone(),
        sans_font(type_scale::XS),
        with_alpha(color::FG_FAINT, fade),
    );
    p.galley(
        pos2(x, y - name.size().y / 2.0),
        name.clone(),
        with_alpha(color::FG_FAINT, fade),
    );
    x += name.size().x + space::S1;

    let summary = summary_override
        .map(str::to_owned)
        .unwrap_or_else(|| summarize_call(&card.call));
    if !summary.is_empty() {
        let separator = p.layout_no_wrap(
            GLYPH_SEPARATOR.to_owned(),
            sans_font(type_scale::XS),
            with_alpha(color::FG_FAINT, fade),
        );
        p.galley(
            pos2(x, y - separator.size().y / 2.0),
            separator.clone(),
            with_alpha(color::FG_FAINT, fade),
        );
        x += separator.size().x + space::S1;
        p.text(
            pos2(x, y),
            Align2::LEFT_CENTER,
            summary,
            sans_font(type_scale::SM),
            with_alpha(color::FG_MUTED, fade),
        );
    }

    // The status glyph (right of the header, outside the text clip).
    let center = pos2(header.right() - CARD_PAD - GLYPH_SLOT / 2.0, y);
    match &card.result {
        Some(result) if result.ok => {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                GLYPH_OK,
                sans_font(type_scale::XS),
                with_alpha(color::OK, fade),
            );
        }
        Some(_) => {
            painter.text(
                center,
                Align2::CENTER_CENTER,
                GLYPH_ERR,
                sans_font(type_scale::XS),
                with_alpha(color::ERR, fade),
            );
        }
        None => {
            let elapsed_ms = (ui.ctx().input(|i| i.time) * 1000.0) as u64;
            if turn_live {
                paint_spin(painter, center, elapsed_ms, reduce_motion);
            } else {
                // The turn ended before the result landed (Stop/cap) —
                // freeze to the §7.4 static dot instead of spinning
                // forever.
                painter.circle_filled(center, 3.0, with_alpha(color::FG_MUTED, fade));
            }
        }
    }
}

/// The right-aligned decision row (§4.2:268): `Approve` (`--ink-mid`
/// fill, `--fg-default`, hover one ink step up — the send-button
/// precedent), `Approve all` (ghost `--type-sm`, only from the turn's
/// second pending card), `Deny` (ghost `--fg-muted`, hover `--ink-faint`).
/// Returns the clicked decision, if any.
fn paint_decision_row(
    ui: &mut Ui,
    row: Rect,
    card: &ToolCard,
    fade: f32,
    outcome: &mut CardOutcome,
) -> Option<AgentDecision> {
    let ordinal = card
        .approval
        .as_ref()
        .map(|(_, _, ordinal)| *ordinal)
        .unwrap_or(0);
    let mut decision = None;
    let mut x = row.right() - CARD_PAD;
    // Painted right-to-left: Approve, Approve all, Deny.
    let buttons: [(&'static str, AgentDecision, bool); 3] = [
        (LABEL_APPROVE, AgentDecision::Approve, true),
        (
            LABEL_APPROVE_ALL,
            AgentDecision::ApproveAllThisTurn,
            ordinal >= 2,
        ),
        (LABEL_DENY, AgentDecision::Deny, true),
    ];
    for (label, this_decision, shown) in buttons {
        if !shown {
            continue;
        }
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            sans_font(type_scale::SM),
            with_alpha(color::FG_DEFAULT, fade),
        );
        let width = galley.size().x + 2.0 * space::S2;
        let rect = Rect::from_min_size(
            pos2(x - width, row.center().y - BUTTON_HEIGHT / 2.0),
            vec2(width, BUTTON_HEIGHT),
        );
        x = rect.left() - space::S1;
        let response = ui.interact(
            rect,
            egui::Id::new(("selora_agent_decision", &card.call.id, label)),
            Sense::click(),
        );
        let painter = ui.painter();
        match this_decision {
            AgentDecision::Approve => {
                let fill = if response.hovered() {
                    color::INK_HI
                } else {
                    color::INK_MID
                };
                painter.rect_filled(rect, radius::R1, with_alpha(fill, fade));
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    sans_font(type_scale::SM),
                    with_alpha(color::FG_DEFAULT, fade),
                );
            }
            _ => {
                if response.hovered() {
                    painter.rect_filled(rect, radius::R1, with_alpha(color::INK_FAINT, fade));
                }
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    sans_font(type_scale::SM),
                    with_alpha(
                        if response.hovered() {
                            color::FG_DEFAULT
                        } else {
                            color::FG_MUTED
                        },
                        fade,
                    ),
                );
            }
        }
        if response.clicked() {
            decision = Some(this_decision);
        }
        outcome.button_rects.push((label, rect));
    }
    decision
}

/// The running bash card's Stop button (§4.2:288: "a Stop button
/// replaces Approve"), right-aligned; the same `--type-xs --fg-muted`,
/// hover `--err` styling as the status line's Stop.
fn paint_stop_button(
    ui: &mut Ui,
    row: Rect,
    card: &ToolCard,
    fade: f32,
    outcome: &mut CardOutcome,
) -> bool {
    let galley = ui.painter().layout_no_wrap(
        LABEL_STOP.to_owned(),
        sans_font(type_scale::XS),
        with_alpha(color::FG_MUTED, fade),
    );
    let width = galley.size().x + 2.0 * space::S1;
    let rect = Rect::from_min_size(
        pos2(
            row.right() - CARD_PAD - width,
            row.center().y - BUTTON_HEIGHT / 2.0,
        ),
        vec2(width, BUTTON_HEIGHT),
    );
    let response = ui.interact(
        rect,
        egui::Id::new(("selora_agent_stop", &card.call.id)),
        Sense::click(),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        LABEL_STOP,
        sans_font(type_scale::XS),
        with_alpha(
            if response.hovered() {
                color::ERR
            } else {
                color::FG_MUTED
            },
            fade,
        ),
    );
    outcome.button_rects.push((LABEL_STOP, rect));
    response.clicked()
}

// -----------------------------------------------------------------------------
// Bodies (the scrollable region under a card header)
// -----------------------------------------------------------------------------

/// A card body, pre-parsed for measurement and painting.
enum Body {
    /// No body (collapsed).
    None,
    /// Monospace text (tool results, bash output, fetched pages).
    Text(String),
    /// A parsed unified diff (colored rows).
    Diff(Vec<DiffLine>),
    /// web_search's parsed (title, URL) rows.
    WebRows(Vec<(String, String)>),
}

/// The body viewport height for layout: content height capped at `cap`.
fn body_height(ui: &Ui, body: &Body, width: f32, cap: f32, fade: f32) -> f32 {
    let wrap = (width - 2.0 * CARD_PAD).max(20.0);
    match body {
        Body::None => 0.0,
        Body::Text(text) => {
            let galley = ui.painter().layout(
                text.clone(),
                code_font(),
                with_alpha(color::FG_DEFAULT, fade),
                wrap,
            );
            (galley.size().y + 2.0 * space::S1).min(cap)
        }
        Body::Diff(lines) => (lines.len() as f32 * DIFF_ROW_HEIGHT).min(cap),
        Body::WebRows(rows) => {
            let row_height = WEB_TITLE_ROW + WEB_URL_ROW + space::S1;
            (rows.len() as f32 * row_height + 2.0 * space::S1).min(cap)
        }
    }
}

/// The body rect under a card's header (0-height bodies get none).
fn body_rect(card_rect: Rect, height: f32) -> Option<Rect> {
    if height <= 0.0 {
        None
    } else {
        Some(Rect::from_min_size(
            pos2(card_rect.left(), card_rect.top() + CARD_HEIGHT),
            vec2(card_rect.width(), height),
        ))
    }
}

/// Paint a body into its viewport rect: a child `Ui` carrying a vertical
/// `ScrollArea` (bodies scroll, §4.2) whose content never out-paints the
/// card.
fn paint_body(
    ui: &mut Ui,
    body: &Body,
    rect: Rect,
    salt: &str,
    fade: f32,
    outcome: &mut CardOutcome,
) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    egui::ScrollArea::vertical()
        .id_salt(("selora_agent_card_body", salt))
        .auto_shrink([false, true])
        .show(&mut child, |ui| match body {
            Body::None => {}
            Body::Text(text) => {
                let wrap = (rect.width() - 2.0 * CARD_PAD).max(20.0);
                let galley = ui.painter().layout(
                    text.clone(),
                    code_font(),
                    with_alpha(color::FG_DEFAULT, fade),
                    wrap,
                );
                let (row, _) = ui.allocate_exact_size(
                    vec2(rect.width(), galley.size().y + 2.0 * space::S1),
                    Sense::hover(),
                );
                ui.painter().galley(
                    pos2(row.left() + CARD_PAD, row.top() + space::S1),
                    galley,
                    with_alpha(color::FG_DEFAULT, fade),
                );
            }
            Body::Diff(lines) => paint_diff_rows(ui, lines, rect.width(), fade),
            Body::WebRows(rows) => paint_web_rows(ui, rows, rect.width(), fade, outcome),
        });
}

/// The diff body: gutter line numbers (old for context/`-`, new for
/// `+`, §4.2:265) in `--fg-faint`, removed lines in `--err`, added in
/// `--ok`, context in `--fg-default`; header/hunk/notice rows span the
/// full width in `--fg-faint`.
fn paint_diff_rows(ui: &mut Ui, lines: &[DiffLine], width: f32, fade: f32) {
    let max_number = lines.iter().filter_map(gutter_number).max().unwrap_or(0);
    let digits = max_number.to_string().len().max(1);
    let gutter_width = ui
        .painter()
        .layout_no_wrap(
            "8".repeat(digits),
            code_font(),
            with_alpha(color::FG_FAINT, fade),
        )
        .size()
        .x
        + space::S2;
    let text_x = CARD_PAD + gutter_width + space::S1;

    for line in lines {
        let (row, _) = ui.allocate_exact_size(vec2(width, DIFF_ROW_HEIGHT), Sense::hover());
        let painter = ui.painter();
        let clipped = painter.with_clip_rect(row);
        let color = with_alpha(diff_line_color(line), fade);
        match line {
            DiffLine::Context { .. } | DiffLine::Removed { .. } | DiffLine::Added { .. } => {
                if let Some(number) = gutter_number(line) {
                    clipped.text(
                        pos2(CARD_PAD + gutter_width, row.center().y),
                        Align2::RIGHT_CENTER,
                        number.to_string(),
                        code_font(),
                        with_alpha(color::FG_FAINT, fade),
                    );
                }
                clipped.text(
                    pos2(text_x, row.center().y),
                    Align2::LEFT_CENTER,
                    diff_line_text(line),
                    code_font(),
                    color,
                );
            }
            _ => {
                // Header / hunk / truncation-notice rows: full width,
                // faint, no gutter (§4.2: a create's `--- /dev/null` is
                // a header, never a removed line).
                clipped.text(
                    pos2(CARD_PAD, row.center().y),
                    Align2::LEFT_CENTER,
                    diff_line_text(line),
                    code_font(),
                    color,
                );
            }
        }
    }
}

/// The web_search result list (§4.2: "numbered result list (title +
/// URL, click = copy URL)").
fn paint_web_rows(
    ui: &mut Ui,
    rows: &[(String, String)],
    width: f32,
    fade: f32,
    outcome: &mut CardOutcome,
) {
    ui.add_space(space::S1);
    for (index, (title, url)) in rows.iter().enumerate() {
        let row_height = WEB_TITLE_ROW + WEB_URL_ROW + space::S1;
        let (row, response) = ui.allocate_exact_size(vec2(width, row_height), Sense::click());
        if response.hovered() {
            ui.painter()
                .rect_filled(row, radius::R0, with_alpha(color::INK_FAINT, fade));
        }
        let painter = ui.painter();
        let clipped = painter.with_clip_rect(row);
        clipped.text(
            pos2(
                row.left() + CARD_PAD,
                row.top() + space::S1 + WEB_TITLE_ROW / 2.0,
            ),
            Align2::LEFT_CENTER,
            format!("{}. {}", index + 1, title),
            sans_font(type_scale::SM),
            with_alpha(color::FG_DEFAULT, fade),
        );
        clipped.text(
            pos2(
                row.left() + CARD_PAD,
                row.top() + space::S1 + WEB_TITLE_ROW + WEB_URL_ROW / 2.0,
            ),
            Align2::LEFT_CENTER,
            url,
            mono_font(type_scale::XS),
            with_alpha(color::FG_FAINT, fade),
        );
        if response.clicked() {
            outcome.copy = Some(url.clone());
        }
        outcome.row_rects.push(row);
    }
    ui.add_space(space::S1);
}

// -----------------------------------------------------------------------------
// Motion (DESIGN_TOKENS.md §7 — every duration and curve from theme::motion)
// -----------------------------------------------------------------------------

/// The `fade` glyph (§7.3) as a card entrance (§4.2: "`fade`
/// (`--dur-quick`)"): opacity 0 → 1 over `--dur-quick` on `--ease-fade`.
/// §7.4 keeps `fade` under reduced motion (short, essential for
/// affordance), so no flag here.
pub fn fade_alpha(elapsed: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() * 1000.0 / motion::DUR_QUICK_MS as f32).clamp(0.0, 1.0);
    cubic_bezier_y(motion::EASE_FADE, t)
}

/// The `pulse` glyph's opacity (§7.3: 1.0 → 0.7 → 1.0 over `--dur-loop`
/// on `--ease-orbital`). A static 1.0 under reduced motion (§7.4).
pub fn pulse_alpha(elapsed_ms: u64, reduce_motion: bool) -> f32 {
    if reduce_motion {
        return 1.0;
    }
    1.0 - 0.3 * pulse_eased(elapsed_ms)
}

/// The `pulse` glyph's scale (§7.3: 1.0 → 1.08 → 1.0). A static 1.0
/// under reduced motion (§7.4).
pub fn pulse_scale(elapsed_ms: u64, reduce_motion: bool) -> f32 {
    if reduce_motion {
        return 1.0;
    }
    1.0 + 0.08 * pulse_eased(elapsed_ms)
}

/// The `spin` glyph's leading angle (§7.3: 360° per `--dur-loop`,
/// linear), or `None` under reduced motion — the caller paints a static
/// dot instead (§7.4).
pub fn spin_angle(elapsed_ms: u64, reduce_motion: bool) -> Option<f32> {
    if reduce_motion {
        None
    } else {
        Some(
            (elapsed_ms % motion::DUR_LOOP_MS as u64) as f32 / motion::DUR_LOOP_MS as f32
                * std::f32::consts::TAU,
        )
    }
}

/// The pulse keyframes' easing position: a symmetric triangle wave (the
/// 0%↔100% and 50% keyframes) through `--ease-orbital`.
fn pulse_eased(elapsed_ms: u64) -> f32 {
    let t = (elapsed_ms % motion::DUR_LOOP_MS as u64) as f32 / motion::DUR_LOOP_MS as f32;
    let triangle = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
    cubic_bezier_y(motion::EASE_ORBITAL, triangle)
}

/// The y component of a `cubic-bezier(x1, y1, x2, y2)` curve at parameter
/// `t` (DESIGN_TOKENS.md §7.1). The curves parametrize by `t` directly
/// rather than solving for `x` — indistinguishable for these symmetric
/// loops and entrances, and pure/testable.
fn cubic_bezier_y(curve: (f32, f32, f32, f32), t: f32) -> f32 {
    let (_, y1, _, y2) = curve;
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    3.0 * u * u * t * y1 + 3.0 * u * t * t * y2 + t * t * t
}

/// The `spin` glyph: a 270° arc turning 360° per `--dur-loop`; under
/// reduced motion, the §7.4 static dot.
fn paint_spin(painter: &egui::Painter, center: Pos2, elapsed_ms: u64, reduce_motion: bool) {
    match spin_angle(elapsed_ms, reduce_motion) {
        None => {
            painter.circle_filled(center, 3.0, color::FG_MUTED);
        }
        Some(angle) => {
            let points: Vec<Pos2> = (0..=9)
                .map(|i| {
                    let segment = angle + (i as f32 / 9.0) * (std::f32::consts::TAU * 0.75);
                    center + Vec2::angled(segment) * 4.0
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.0, color::FG_FAINT),
            ));
        }
    }
}

/// Ask for the next animation frame while a pulse/spin is live.
fn request_animation_frame(ctx: &egui::Context) {
    ctx.request_repaint_after(Duration::from_millis(ANIM_FRAME_MS));
}

// -----------------------------------------------------------------------------
// Pure parsing helpers (the router's output formats; unit-tested)
// -----------------------------------------------------------------------------

/// One classified line of the router's unified diff (fs.rs emits `--- `
/// / `+++ ` headers, `@@ -a,b +c,d @@` hunks, `' '`/`-`/`+` body lines,
/// and a `… (diff truncated at N lines)` notice).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    /// `--- a/x`, `--- /dev/null`, `+++ b/x` — painted faint, never as a
    /// removed/added line.
    Header(String),
    /// `@@ -a,b +c,d @@` — painted faint; carries the gutter counters.
    Hunk(String),
    /// A context line (` text`) — both numbers advance.
    Context { old: u32, new: u32, text: String },
    /// A removed line (`-text`) — the old number advances.
    Removed { old: u32, text: String },
    /// An added line (`+text`) — the new number advances.
    Added { new: u32, text: String },
    /// The router's `… (diff truncated at N lines)` notice.
    Notice(String),
    /// Anything else (a blank row, a malformed line) — faint, no gutter.
    Other(String),
}

/// Parse the router's unified diff (fs.rs:743-818). Header rows are
/// classified BEFORE the single-char `'-'`/`'+'` test so a create's
/// `--- /dev/null` never lands as a removed line; the `@@` counters seed
/// the gutter numbers (old for context/`-`, new for `+`).
pub fn parse_unified_diff(diff: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    for raw in diff.lines() {
        if raw.starts_with("--- ") || raw.starts_with("+++ ") {
            lines.push(DiffLine::Header(raw.to_owned()));
            continue;
        }
        if raw.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_header(raw) {
                old_no = old_start;
                new_no = new_start;
            }
            lines.push(DiffLine::Hunk(raw.to_owned()));
            continue;
        }
        if raw.starts_with('…') {
            lines.push(DiffLine::Notice(raw.to_owned()));
            continue;
        }
        match raw.as_bytes().first() {
            Some(b' ') => {
                lines.push(DiffLine::Context {
                    old: old_no,
                    new: new_no,
                    text: raw.to_owned(),
                });
                old_no += 1;
                new_no += 1;
            }
            Some(b'-') => {
                lines.push(DiffLine::Removed {
                    old: old_no,
                    text: raw.to_owned(),
                });
                old_no += 1;
            }
            Some(b'+') => {
                lines.push(DiffLine::Added {
                    new: new_no,
                    text: raw.to_owned(),
                });
                new_no += 1;
            }
            _ => lines.push(DiffLine::Other(raw.to_owned())),
        }
    }
    lines
}

/// `@@ -41,3 +41,3 @@` → `Some((41, 41))`; tolerant of a missing
/// `,count` and of trailing context after the closing `@@`.
fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let inner = line.strip_prefix("@@")?.trim_start();
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let old_start = old.split(',').next()?.parse().ok()?;
    let new_start = new.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

/// The gutter number for a body line (old for context/`-`, new for
/// `+`); `None` for header/hunk/notice rows.
pub fn gutter_number(line: &DiffLine) -> Option<u32> {
    match line {
        DiffLine::Context { old, .. } | DiffLine::Removed { old, .. } => Some(*old),
        DiffLine::Added { new, .. } => Some(*new),
        _ => None,
    }
}

/// The line's text color (§4.2:266 — the chat's only saturated uses,
/// semantic): `-` in `--err`, `+` in `--ok`, context `--fg-default`,
/// header/hunk/notice rows `--fg-faint`.
pub fn diff_line_color(line: &DiffLine) -> Color32 {
    match line {
        DiffLine::Removed { .. } => color::ERR,
        DiffLine::Added { .. } => color::OK,
        DiffLine::Context { .. } => color::FG_DEFAULT,
        _ => color::FG_FAINT,
    }
}

/// The line's raw text for painting.
fn diff_line_text(line: &DiffLine) -> &str {
    match line {
        DiffLine::Header(text)
        | DiffLine::Hunk(text)
        | DiffLine::Notice(text)
        | DiffLine::Other(text) => text,
        DiffLine::Context { text, .. }
        | DiffLine::Removed { text, .. }
        | DiffLine::Added { text, .. } => text,
    }
}

/// The bash tool's exit code (bash.rs:331 prefixes every successful
/// result with `(exit code N)`); `None` for error results (guard
/// rejection, timeout, denial — no prefix).
pub fn parse_bash_exit(content: &str) -> Option<i32> {
    let rest = content.strip_prefix("(exit code ")?;
    let close = rest.find(')')?;
    rest[..close].trim().parse().ok()
}

/// web_search's result body (web.rs:457-459: a
/// `Web search results for "<query>":` header line, then `N. title` /
/// indented-URL pairs). Tolerant: any deviation (including the
/// `No results found for …` message) renders as raw text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebSearchBody {
    /// Parsed (title, URL) pairs, in order.
    Results(Vec<(String, String)>),
    /// The format deviated — render the text verbatim.
    Raw(String),
}

/// Parse a web_search result into numbered rows (see [`WebSearchBody`]).
pub fn parse_web_search_results(content: &str) -> WebSearchBody {
    let raw = || WebSearchBody::Raw(content.to_owned());
    let mut lines = content.lines();
    match lines.next() {
        Some(header) if header.starts_with("Web search results for ") => {}
        _ => return raw(),
    }
    let mut results = Vec::new();
    let lines = lines.by_ref();
    while let Some(title_line) = lines.next() {
        if title_line.is_empty() {
            continue;
        }
        let Some((number, title)) = title_line.split_once(". ") else {
            return raw();
        };
        if number.parse::<usize>().is_err() {
            return raw();
        }
        let Some(url_line) = lines.next() else {
            return raw();
        };
        let url = url_line.trim();
        if url.is_empty() || url_line.len() == url.len() {
            return raw(); // the URL line must be indented
        }
        results.push((title.trim().to_owned(), url.to_owned()));
    }
    if results.is_empty() {
        return raw();
    }
    WebSearchBody::Results(results)
}

/// Split a web_fetch result into the `Source: <url>` line (web.rs:534)
/// and the body text. No `Source:` line → `(None, the whole content)`.
pub fn split_web_fetch_source(content: &str) -> (Option<String>, String) {
    if let Some(rest) = content.strip_prefix("Source: ") {
        if let Some(newline) = rest.find('\n') {
            let url = rest[..newline].trim().to_owned();
            let after = &rest[newline + 1..];
            let body = after.strip_prefix('\n').unwrap_or(after);
            return (Some(url), body.to_owned());
        }
        return (Some(rest.trim().to_owned()), String::new());
    }
    (None, content.to_owned())
}

/// The collapsed header's argument summary (§4.2:249): the headline
/// argument per tool — `read_file`'s `path :start-end`, `bash`'s command
/// (first line), the quoted query for the find/search tools, the URL for
/// `web_fetch`. Garbage or missing arguments fall back to the (capped)
/// raw arguments string; this never panics.
pub fn summarize_call(call: &ToolCall) -> String {
    let args: Option<serde_json::Value> = serde_json::from_str(&call.arguments).ok();
    let arg = |key: &str| {
        args.as_ref()
            .and_then(|args| args.get(key))
            .and_then(serde_json::Value::as_str)
    };
    let arg_u64 = |key: &str| {
        args.as_ref()
            .and_then(|args| args.get(key))
            .and_then(serde_json::Value::as_u64)
    };
    let summary = match call.name.as_str() {
        "read_file" => {
            let path = arg("path").unwrap_or_default();
            match (arg_u64("start_line"), arg_u64("end_line")) {
                (Some(start), Some(end)) => format!("{path} :{start}-{end}"),
                (Some(start), None) => format!("{path} :{start}-"),
                (None, Some(end)) => format!("{path} :-{end}"),
                (None, None) => path.to_owned(),
            }
        }
        "list_dir" | "write_file" | "edit_file" | "delete_file" => {
            arg("path").unwrap_or_default().to_owned()
        }
        "fuzzy_find" | "search_files" | "web_search" => arg("query")
            .map(|query| format!("\"{query}\""))
            .unwrap_or_default(),
        "bash" => {
            let command = arg("command").unwrap_or_default();
            let mut lines = command.lines();
            let first = lines.next().unwrap_or_default();
            if lines.next().is_some() {
                format!("{first} …")
            } else {
                first.to_owned()
            }
        }
        "web_fetch" => arg("url").unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    if !summary.is_empty() {
        return summary;
    }
    // The raw fallback (unknown tool, unparseable arguments), capped so a
    // pathological blob cannot stall the layout pass.
    let raw = call.arguments.trim();
    if raw.is_empty() {
        return String::new();
    }
    truncate_chars(raw, SUMMARY_FALLBACK_MAX)
}

/// The status line's verb line for a running tool (§4.2:300: `reading
/// src/main.rs`, `running cargo test`).
pub fn status_verb(call: &ToolCall) -> String {
    let args: Option<serde_json::Value> = serde_json::from_str(&call.arguments).ok();
    let arg = |key: &str| {
        args.as_ref()
            .and_then(|args| args.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    };
    let verb = match call.name.as_str() {
        "read_file" => format!("reading {}", arg("path")),
        "list_dir" => format!("listing {}", arg("path")),
        "fuzzy_find" => format!("finding {}", arg("query")),
        "search_files" => format!("searching {}", arg("query")),
        "write_file" => format!("writing {}", arg("path")),
        "edit_file" => format!("editing {}", arg("path")),
        "delete_file" => format!("deleting {}", arg("path")),
        "bash" => format!(
            "running {}",
            arg("command").lines().next().unwrap_or_default()
        ),
        "web_search" => "searching the web".to_owned(),
        "web_fetch" => format!("fetching {}", arg("url")),
        other => format!("running {other}"),
    };
    verb.trim_end().to_owned()
}

/// Byte-safe truncation for the [`summarize_call`] raw fallback.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

// -----------------------------------------------------------------------------
// Fonts and small helpers
// -----------------------------------------------------------------------------

/// A sans label font at a type-scale step.
fn sans_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, font::FAMILY_SANS)
}

/// A monospace font at a type-scale step.
fn mono_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, font::FAMILY_MONO)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- fixtures ------------------------------------------------------------

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: format!("call_{name}_1"),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn result(id: &str, ok: bool, content: &str) -> ToolResult {
        ToolResult {
            id: id.to_owned(),
            ok,
            content: content.to_owned(),
        }
    }

    /// A card past its fade-in (deterministic full opacity).
    fn settled(mut card: ToolCard) -> ToolCard {
        card.appeared = Instant::now() - Duration::from_secs(10);
        card
    }

    fn approval_card(id: u32) -> ToolCard {
        let mut card = ToolCard::new(call(
            "edit_file",
            r#"{"path":"src/editor/mod.rs","old_str":"a","new_str":"b"}"#,
        ));
        card.approval = Some((
            ApprovalId::from_u128(id as u128),
            ApprovalCard::Diff {
                path: "src/editor/mod.rs".to_owned(),
                diff: DIFF.to_owned(),
            },
            id,
        ));
        settled(card)
    }

    const DIFF: &str = "--- a/src/editor/mod.rs\n\
                        +++ b/src/editor/mod.rs\n\
                        @@ -41,3 +41,3 @@\n\
                        \x20pub fn theme() -> Theme {\n\
                        -    Theme::from_tokens(TOKENS)\n\
                        +    Theme::from_tokens(TOKENS, dpi_scale)\n\
                        \x20}\n";

    const CREATE_DIFF: &str = "--- /dev/null\n\
                               +++ b/src/new.rs\n\
                               @@ -0,0 +1,2 @@\n\
                               +fn main() {}\n\
                               +// entry\n";

    // The headless viewport the click tests paint into (the chat panel's
    // own pattern).
    fn test_screen() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 600.0))
    }

    fn raw_input_with(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(test_screen()),
            events,
            ..Default::default()
        }
    }

    fn click_at(pos: Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]
    }

    /// One headless frame painting a single card; returns its outcome.
    fn card_frame(
        ctx: &egui::Context,
        input: egui::RawInput,
        card: &mut ToolCard,
        reduce_motion: bool,
    ) -> CardOutcome {
        card_frame_live(ctx, input, card, reduce_motion, true)
    }

    /// The same, with the turn-liveness knob (a dead turn freezes its
    /// resultless cards). The gateway reads as connected (the sleeping
    /// avatar's tests drive the flag directly).
    fn card_frame_live(
        ctx: &egui::Context,
        input: egui::RawInput,
        card: &mut ToolCard,
        reduce_motion: bool,
        turn_live: bool,
    ) -> CardOutcome {
        let mut input = input;
        if input.screen_rect.is_none() {
            input.screen_rect = Some(test_screen());
        }
        let mut outcome = CardOutcome::default();
        let mut output = ctx.run_ui(input, |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            outcome = tool_card(ui, card, reduce_motion, turn_live, true);
        });
        output.textures_delta.clear();
        outcome
    }

    /// One headless frame painting the status line.
    fn status_frame(
        ctx: &egui::Context,
        input: egui::RawInput,
        phase: &StatusPhase,
        reduce_motion: bool,
    ) -> StatusLineOutcome {
        let mut input = input;
        if input.screen_rect.is_none() {
            input.screen_rect = Some(test_screen());
        }
        let mut avatar = AgentAvatar::new(0.0);
        let mut outcome = StatusLineOutcome {
            stop_clicked: false,
            rect: Rect::ZERO,
            stop_rect: Rect::ZERO,
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            outcome = status_line(
                ui,
                phase,
                OrbState::Working,
                &mut avatar,
                true,
                reduce_motion,
            );
        });
        output.textures_delta.clear();
        outcome
    }

    // -- parse_unified_diff ----------------------------------------------------

    #[test]
    fn diff_parses_headers_hunks_and_numbered_body_lines() {
        let lines = parse_unified_diff(DIFF);
        assert_eq!(
            lines,
            vec![
                DiffLine::Header("--- a/src/editor/mod.rs".to_owned()),
                DiffLine::Header("+++ b/src/editor/mod.rs".to_owned()),
                DiffLine::Hunk("@@ -41,3 +41,3 @@".to_owned()),
                DiffLine::Context {
                    old: 41,
                    new: 41,
                    text: " pub fn theme() -> Theme {".to_owned(),
                },
                DiffLine::Removed {
                    old: 42,
                    text: "-    Theme::from_tokens(TOKENS)".to_owned(),
                },
                DiffLine::Added {
                    new: 42,
                    text: "+    Theme::from_tokens(TOKENS, dpi_scale)".to_owned(),
                },
                DiffLine::Context {
                    old: 43,
                    new: 43,
                    text: " }".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn create_diff_headers_are_never_removed_lines() {
        let lines = parse_unified_diff(CREATE_DIFF);
        assert_eq!(lines[0], DiffLine::Header("--- /dev/null".to_owned()));
        assert_eq!(lines[1], DiffLine::Header("+++ b/src/new.rs".to_owned()));
        assert!(
            !lines
                .iter()
                .any(|line| matches!(line, DiffLine::Removed { .. })),
            "a create diff is all additions: {lines:?}"
        );
        // The git `@@ -0,0 +1,n @@` convention: additions number from 1.
        assert_eq!(
            lines[3],
            DiffLine::Added {
                new: 1,
                text: "+fn main() {}".to_owned()
            }
        );
    }

    #[test]
    fn the_truncation_notice_is_a_notice_row() {
        let diff = format!("{DIFF}… (diff truncated at 4000 lines)\n");
        let lines = parse_unified_diff(&diff);
        assert_eq!(
            lines.last(),
            Some(&DiffLine::Notice(
                "… (diff truncated at 4000 lines)".to_owned()
            ))
        );
    }

    #[test]
    fn diff_line_colors_are_the_only_saturated_uses() {
        let lines = parse_unified_diff(DIFF);
        let color_of = |line: &DiffLine| diff_line_color(line);
        assert_eq!(color_of(&lines[0]), color::FG_FAINT); // header
        assert_eq!(color_of(&lines[2]), color::FG_FAINT); // hunk
        assert_eq!(color_of(&lines[3]), color::FG_DEFAULT); // context
        assert_eq!(color_of(&lines[4]), color::ERR); // removed
        assert_eq!(color_of(&lines[5]), color::OK); // added
    }

    #[test]
    fn hunk_header_tolerates_missing_counts() {
        let lines = parse_unified_diff("@@ -7 +9 @@\n+a\n b\n");
        assert_eq!(
            lines[1],
            DiffLine::Added {
                new: 9,
                text: "+a".to_owned()
            }
        );
        assert_eq!(
            lines[2],
            DiffLine::Context {
                old: 7,
                new: 10,
                text: " b".to_owned()
            }
        );
    }

    // -- bash -------------------------------------------------------------------

    #[test]
    fn parse_bash_exit_reads_the_router_prefix() {
        assert_eq!(parse_bash_exit("(exit code 0)\nall good"), Some(0));
        assert_eq!(parse_bash_exit("(exit code 3)\nfailed"), Some(3));
        assert_eq!(parse_bash_exit("(exit code -1)\nkilled"), Some(-1));
        assert_eq!(parse_bash_exit("user denied this action"), None);
        assert_eq!(parse_bash_exit(""), None);
    }

    #[test]
    fn bash_output_body_strips_the_prefix_line() {
        assert_eq!(bash_output_body("(exit code 0)\nhello\n"), "hello\n");
        assert_eq!(
            bash_output_body("user denied this action"),
            "user denied this action"
        );
    }

    // -- web --------------------------------------------------------------------

    #[test]
    fn web_search_results_parse_the_router_format() {
        let content = "Web search results for \"ropey crate\":\n\
                       1. ropey — Rope Science\n   \
                       https://docs.rs/ropey\n\
                       2. GitHub - ropey\n   \
                       https://github.com/cessen/ropey\n";
        match parse_web_search_results(content) {
            WebSearchBody::Results(rows) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].0, "ropey — Rope Science");
                assert_eq!(rows[0].1, "https://docs.rs/ropey");
                assert_eq!(rows[1].1, "https://github.com/cessen/ropey");
            }
            other => panic!("expected Results, got {other:?}"),
        }
    }

    #[test]
    fn web_search_falls_back_to_raw_on_deviation() {
        // The no-results message is not a parse failure — it just is not
        // the numbered list, so it renders as raw text.
        let empty = "No results found for \"zzz\".";
        assert_eq!(
            parse_web_search_results(empty),
            WebSearchBody::Raw(empty.to_owned())
        );
        // A title line without its indented URL line deviates → raw.
        let broken = "Web search results for \"x\":\n1. Title\nhttps://not-indented.rs\n";
        assert_eq!(
            parse_web_search_results(broken),
            WebSearchBody::Raw(broken.to_owned())
        );
    }

    #[test]
    fn web_fetch_source_splits_off_the_body() {
        let (url, body) = split_web_fetch_source("Source: https://example.com\n\nHello text\n");
        assert_eq!(url.as_deref(), Some("https://example.com"));
        assert_eq!(body, "Hello text\n");
        let (url, body) = split_web_fetch_source("web_fetch: HTTP 404");
        assert_eq!(url, None);
        assert_eq!(body, "web_fetch: HTTP 404");
    }

    // -- summarize_call / status_verb ---------------------------------------------

    #[test]
    fn summarize_extracts_the_headline_argument() {
        assert_eq!(
            summarize_call(&call(
                "read_file",
                r#"{"path":"src/editor/mod.rs","start_line":42,"end_line":88}"#
            )),
            "src/editor/mod.rs :42-88"
        );
        assert_eq!(
            summarize_call(&call("read_file", r#"{"path":"src/main.rs"}"#)),
            "src/main.rs"
        );
        assert_eq!(
            summarize_call(&call("bash", r#"{"command":"cargo test -p selora-core"}"#)),
            "cargo test -p selora-core"
        );
        assert_eq!(
            summarize_call(&call("web_search", r#"{"query":"ropey crate"}"#)),
            "\"ropey crate\""
        );
        assert_eq!(
            summarize_call(&call("web_fetch", r#"{"url":"https://example.com"}"#)),
            "https://example.com"
        );
        assert_eq!(
            summarize_call(&call("fuzzy_find", r#"{"query":"main"}"#)),
            "\"main\""
        );
    }

    #[test]
    fn summarize_handles_multiline_commands_and_garbage() {
        assert_eq!(
            summarize_call(&call("bash", r#"{"command":"cargo build\ncargo test"}"#)),
            "cargo build …"
        );
        // Garbage JSON: the raw fallback, never a panic.
        assert_eq!(summarize_call(&call("read_file", "{not json")), "{not json");
        // Unknown tool: raw as well.
        assert_eq!(
            summarize_call(&call("mystery_tool", r#"{"a":1}"#)),
            r#"{"a":1}"#
        );
        assert_eq!(summarize_call(&call("read_file", "")), "");
    }

    #[test]
    fn summarize_caps_the_raw_fallback() {
        let long = "x".repeat(500);
        let summary = summarize_call(&call("mystery", &long));
        assert!(summary.len() <= SUMMARY_FALLBACK_MAX + "…".len());
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn status_verb_maps_the_tools() {
        assert_eq!(
            status_verb(&call("read_file", r#"{"path":"src/main.rs"}"#)),
            "reading src/main.rs"
        );
        assert_eq!(
            status_verb(&call("bash", r#"{"command":"cargo test"}"#)),
            "running cargo test"
        );
        assert_eq!(
            status_verb(&call("edit_file", r#"{"path":"src/lib.rs"}"#)),
            "editing src/lib.rs"
        );
        assert_eq!(
            status_verb(&call("web_search", r#"{"query":"x"}"#)),
            "searching the web"
        );
        assert_eq!(
            status_verb(&call("web_fetch", r#"{"url":"https://example.com"}"#)),
            "fetching https://example.com"
        );
    }

    // -- motion helpers -----------------------------------------------------------

    #[test]
    fn pulse_honors_reduce_motion() {
        assert_eq!(pulse_alpha(600, true), 1.0);
        assert_eq!(pulse_scale(600, true), 1.0);
        // The keyframes (§7.3): 1.0 at the loop's edges, 0.7 / 1.08 at
        // the midpoint.
        assert!((pulse_alpha(0, false) - 1.0).abs() < 0.01);
        assert!((pulse_alpha(600, false) - 0.7).abs() < 0.01);
        assert!((pulse_scale(600, false) - 1.08).abs() < 0.01);
        // The loop wraps.
        assert!((pulse_alpha(1200, false) - pulse_alpha(0, false)).abs() < 0.01);
    }

    #[test]
    fn spin_loops_linearly_and_freezes_under_reduce_motion() {
        assert_eq!(spin_angle(600, true), None);
        let start = spin_angle(0, false).expect("angle");
        let half = spin_angle(600, false).expect("angle");
        assert!((start - 0.0).abs() < 0.01);
        assert!((half - std::f32::consts::PI).abs() < 0.01);
    }

    #[test]
    fn fade_ramps_over_dur_quick() {
        assert_eq!(fade_alpha(Duration::ZERO), 0.0);
        assert_eq!(
            fade_alpha(Duration::from_millis(motion::DUR_QUICK_MS as u64)),
            1.0
        );
        assert_eq!(fade_alpha(Duration::from_secs(10)), 1.0);
        let mid = fade_alpha(Duration::from_millis(motion::DUR_QUICK_MS as u64 / 2));
        assert!(mid > 0.0 && mid < 1.0, "mid-fade: {mid}");
    }

    // -- summary footer text -------------------------------------------------------

    #[test]
    fn summary_line_formats_the_spec_example() {
        let summary = AgentSummary {
            iterations: 3,
            tools_used: 6,
            approved: 3,
            denied: 1,
            tokens: 12_400,
            cost_usd: 0.0412,
            completed: true,
        };
        assert_eq!(
            summary_line(&summary),
            "6 tools · 3 approved · 1 denied · 12.4k tokens · $0.041"
        );
        let one = AgentSummary {
            tools_used: 1,
            tokens: 950,
            ..summary.clone()
        };
        assert_eq!(
            summary_line(&one),
            "1 tool · 3 approved · 1 denied · 950 tokens · $0.041"
        );
        assert_eq!(format_tokens(1_200_000), "1.2M");
    }

    // -- status_phase_for ------------------------------------------------------------

    #[test]
    fn status_phase_prefers_pending_then_running_then_thinking() {
        let mut pending = ToolCard::new(call("edit_file", r#"{"path":"a.rs"}"#));
        pending.approval = Some((
            ApprovalId::from_u128(1),
            ApprovalCard::Diff {
                path: "a.rs".to_owned(),
                diff: DIFF.to_owned(),
            },
            1,
        ));
        let running = ToolCard::new(call("bash", r#"{"command":"cargo test"}"#));
        let mut done = ToolCard::new(call("read_file", r#"{"path":"b.rs"}"#));
        done.result = Some(result(&done.call.id.clone(), true, "contents"));

        assert_eq!(
            status_phase_for([&pending, &running, &done]),
            StatusPhase::AwaitingApproval
        );
        assert_eq!(
            status_phase_for([&running, &done]),
            StatusPhase::Running("running cargo test".to_owned())
        );
        assert_eq!(status_phase_for([&done]), StatusPhase::Thinking);
        assert_eq!(status_phase_for([]), StatusPhase::Thinking);
    }

    // -- collapsed read-type card ------------------------------------------------------

    #[test]
    fn read_card_paints_collapsed_24px_and_expands_on_click() {
        let mut card = settled(ToolCard::new(call(
            "read_file",
            r#"{"path":"src/editor/mod.rs","start_line":42,"end_line":88}"#,
        )));
        card.result = Some(result(&card.call.id.clone(), true, "line 1\nline 2\n"));

        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(outcome.rect.height(), CARD_HEIGHT);
        assert_eq!(outcome.body_rect, None, "collapsed paints no body");
        assert!(!card.expanded);

        // Click the header: the card expands (the click latches this
        // frame; the body paints from the next frame — egui's
        // immediate-mode order).
        let header_center = outcome.header_rect.center();
        card_frame(
            &ctx,
            raw_input_with(click_at(header_center)),
            &mut card,
            false,
        );
        assert!(card.expanded);
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        let body = outcome.body_rect.expect("expanded paints a body");
        assert!(body.height() > 0.0 && body.height() <= BODY_MAX_HEIGHT);
        assert!(outcome.rect.height() > CARD_HEIGHT);

        // Click again: collapses.
        card_frame(
            &ctx,
            raw_input_with(click_at(header_center)),
            &mut card,
            false,
        );
        assert!(!card.expanded);
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(outcome.rect.height(), CARD_HEIGHT);
    }

    #[test]
    fn read_card_without_a_result_paints_the_running_state() {
        let card = settled(ToolCard::new(call("read_file", r#"{"path":"a.rs"}"#)));
        let mut card = card;
        let ctx = egui::Context::default();
        // Both motion modes paint the 24px one-liner (the spin is a
        // static dot under reduced motion, §7.4).
        for reduce_motion in [false, true] {
            let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, reduce_motion);
            assert_eq!(outcome.rect.height(), CARD_HEIGHT);
            assert!(outcome.button_rects.is_empty());
            assert!(outcome.decision.is_none());
        }
    }

    #[test]
    fn read_card_error_result_paints_collapsed() {
        let mut card = settled(ToolCard::new(call("read_file", r#"{"path":"nope.rs"}"#)));
        card.result = Some(result(
            &card.call.id.clone(),
            false,
            "read_file: escapes the workspace root: ../x",
        ));
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(outcome.rect.height(), CARD_HEIGHT);
        assert!(outcome.decision.is_none());
    }

    // -- diff approval card -------------------------------------------------------------

    #[test]
    fn diff_card_pending_paints_gutter_body_and_two_buttons_on_the_first_approval() {
        let mut card = approval_card(1);
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);

        let body = outcome.body_rect.expect("the diff body paints");
        assert!(body.height() > 0.0 && body.height() <= DIFF_BODY_MAX_HEIGHT);
        let labels: Vec<&str> = outcome.button_rects.iter().map(|(l, _)| *l).collect();
        assert_eq!(
            labels,
            vec![LABEL_APPROVE, LABEL_DENY],
            "Approve all appears only from the turn's second pending card"
        );
        // Right-aligned: Approve sits right of Deny.
        let approve = outcome.button_rects[0].1;
        let deny = outcome.button_rects[1].1;
        assert!(approve.left() > deny.left());
    }

    #[test]
    fn diff_card_approve_latches_the_decision_without_double_fire() {
        let mut card = approval_card(1);
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        let approve_center = outcome.button_rects[0].1.center();

        let outcome = card_frame(
            &ctx,
            raw_input_with(click_at(approve_center)),
            &mut card,
            false,
        );
        assert_eq!(outcome.decision, Some(AgentDecision::Approve));
        assert_eq!(card.decided, Some(AgentDecision::Approve));

        // The latch: next frame paints no buttons, so nothing can fire
        // again even on the same click position.
        let outcome = card_frame(
            &ctx,
            raw_input_with(click_at(approve_center)),
            &mut card,
            false,
        );
        assert!(outcome.button_rects.is_empty());
        assert_eq!(outcome.decision, None);
        // Decided but resultless: the diff body stays up (no buttons).
        assert!(outcome.body_rect.is_some());
    }

    #[test]
    fn diff_card_deny_emits_deny() {
        let mut card = approval_card(1);
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        let deny_center = outcome.button_rects[1].1.center();
        let outcome = card_frame(
            &ctx,
            raw_input_with(click_at(deny_center)),
            &mut card,
            false,
        );
        assert_eq!(outcome.decision, Some(AgentDecision::Deny));
        assert_eq!(card.decided, Some(AgentDecision::Deny));
    }

    #[test]
    fn approve_all_appears_on_the_second_pending_card_and_emits() {
        let mut card = approval_card(2);
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        let labels: Vec<&str> = outcome.button_rects.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec![LABEL_APPROVE, LABEL_APPROVE_ALL, LABEL_DENY]);
        let approve_all = outcome.button_rects[1].1.center();
        let outcome = card_frame(
            &ctx,
            raw_input_with(click_at(approve_all)),
            &mut card,
            false,
        );
        assert_eq!(outcome.decision, Some(AgentDecision::ApproveAllThisTurn));
    }

    #[test]
    fn diff_card_expand_link_uncaps_the_body() {
        // A diff taller than the 240px cap shows the link; the click
        // uncaps (§4.2:267's "expand full file", renamed — the card
        // holds the diff, not the file).
        let mut long_diff = String::from("--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,40 @@\n-old\n");
        for i in 0..40 {
            long_diff.push_str(&format!("+line {i}\n"));
        }
        let mut card = ToolCard::new(call("write_file", r#"{"path":"f.rs","content":"x"}"#));
        card.approval = Some((
            ApprovalId::from_u128(9),
            ApprovalCard::Diff {
                path: "f.rs".to_owned(),
                diff: long_diff,
            },
            1,
        ));
        let mut card = settled(card);

        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(
            outcome.body_rect.map(|r| r.height()),
            Some(DIFF_BODY_MAX_HEIGHT)
        );

        // The link row sits between the body and the buttons.
        let link_pos = pos2(
            outcome.rect.left() + CARD_PAD + 10.0,
            outcome.body_rect.unwrap().bottom() + LINK_ROW_HEIGHT / 2.0,
        );
        card_frame(&ctx, raw_input_with(click_at(link_pos)), &mut card, false);
        assert!(card.expanded);
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert!(
            outcome.body_rect.map(|r| r.height()) > Some(DIFF_BODY_MAX_HEIGHT),
            "expanded: {:?}",
            outcome.body_rect
        );
    }

    // -- delete approval card -------------------------------------------------------------

    #[test]
    fn delete_card_paints_the_muted_body_and_the_button_row() {
        let mut card = ToolCard::new(call("delete_file", r#"{"path":"old.rs"}"#));
        card.approval = Some((
            ApprovalId::from_u128(3),
            ApprovalCard::Delete {
                path: "old.rs".to_owned(),
            },
            1,
        ));
        let mut card = settled(card);

        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert!(outcome.body_rect.is_some(), "the caveat body paints");
        let labels: Vec<&str> = outcome.button_rects.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec![LABEL_APPROVE, LABEL_DENY]);

        let approve = outcome.button_rects[0].1.center();
        let outcome = card_frame(&ctx, raw_input_with(click_at(approve)), &mut card, false);
        assert_eq!(outcome.decision, Some(AgentDecision::Approve));
    }

    #[test]
    fn delete_card_collapses_to_the_one_liner_after_the_result() {
        let mut card = ToolCard::new(call("delete_file", r#"{"path":"old.rs"}"#));
        card.approval = Some((
            ApprovalId::from_u128(3),
            ApprovalCard::Delete {
                path: "old.rs".to_owned(),
            },
            1,
        ));
        card.decided = Some(AgentDecision::Approve);
        card.result = Some(result(&card.call.id.clone(), true, "deleted old.rs"));
        let mut card = settled(card);

        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(
            outcome.rect.height(),
            CARD_HEIGHT,
            "post-result: the one-liner"
        );
        assert!(outcome.button_rects.is_empty());
    }

    // -- bash card ---------------------------------------------------------------------------

    #[test]
    fn bash_card_pending_shows_buttons_running_shows_stop_finished_shows_none() {
        // Pending: Deny + Approve.
        let mut card = ToolCard::new(call("bash", r#"{"command":"cargo test"}"#));
        card.approval = Some((
            ApprovalId::from_u128(4),
            ApprovalCard::Command {
                command: "cargo test".to_owned(),
            },
            1,
        ));
        let mut card = settled(card);
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        let labels: Vec<&str> = outcome.button_rects.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec![LABEL_APPROVE, LABEL_DENY]);
        assert!(outcome.body_rect.is_none(), "no output while parked");

        // Approved and running: Stop replaces Approve.
        card.decided = Some(AgentDecision::Approve);
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        let labels: Vec<&str> = outcome.button_rects.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec![LABEL_STOP]);
        let stop = outcome.button_rects[0].1.center();
        let outcome = card_frame(&ctx, raw_input_with(click_at(stop)), &mut card, false);
        assert_eq!(outcome.decision, Some(AgentDecision::Stop));

        // Finished: ✓ exit 0 in the header, the output body, no buttons.
        card.result = Some(result(
            &card.call.id.clone(),
            true,
            "(exit code 0)\ntest result: ok. 4 passed\n",
        ));
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert!(outcome.button_rects.is_empty());
        assert!(outcome.body_rect.is_some(), "the output body paints");
    }

    #[test]
    fn bash_card_auto_approved_never_shows_buttons() {
        // No ApprovalCard ever arrived (the §17.5 allowlist): the card
        // shows the command, runs, finishes — no buttons at any point.
        let mut card = settled(ToolCard::new(call("bash", r#"{"command":"cargo check"}"#)));
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert!(
            outcome.button_rects.is_empty(),
            "auto-approved running: no buttons"
        );

        card.result = Some(result(
            &card.call.id.clone(),
            true,
            "(exit code 3)\nerror[E0308]: mismatched types\n",
        ));
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert!(outcome.button_rects.is_empty());
        assert!(outcome.body_rect.is_some());
    }

    /// One measured frame on a warmed context: the viewport's repaint
    /// delay (`Duration::MAX` = nothing requested this frame).
    fn measured_repaint_delay(
        ctx: &egui::Context,
        card: &mut ToolCard,
        turn_live: bool,
    ) -> Option<Duration> {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            tool_card(ui, card, false, turn_live, true);
        });
        let delay = output
            .viewport_output
            .values()
            .next()
            .map(|out| out.repaint_delay);
        output.textures_delta.clear();
        delay
    }

    #[test]
    fn a_dead_turns_running_bash_card_freezes() {
        // Approved and mid-run, but the turn is over (Stop/cap — the loop
        // emits no ToolResult for a killed tool): no Stop button, and no
        // animation repaint requests (a spin that outlives its turn would
        // request frames forever).
        let mut card = ToolCard::new(call("bash", r#"{"command":"cargo test"}"#));
        card.approval = Some((
            ApprovalId::from_u128(4),
            ApprovalCard::Command {
                command: "cargo test".to_owned(),
            },
            1,
        ));
        card.decided = Some(AgentDecision::Approve);
        let mut card = settled(card);

        let ctx = egui::Context::default();
        let outcome = card_frame_live(&ctx, egui::RawInput::default(), &mut card, false, false);
        assert!(outcome.button_rects.is_empty(), "no Stop on a dead turn");
        assert!(outcome.decision.is_none());

        // The repaint evidence. Warm the context first — egui's first
        // frames request follow-ups of their own, masking the card's.
        let mut live_card = card.clone();
        for _ in 0..3 {
            measured_repaint_delay(&ctx, &mut live_card, true);
            measured_repaint_delay(&ctx, &mut card, false);
        }
        let live = measured_repaint_delay(&ctx, &mut live_card, true);
        let dead = measured_repaint_delay(&ctx, &mut card, false);
        assert!(
            live.is_some_and(|delay| delay < Duration::from_secs(1)),
            "the live running card animates: {live:?}"
        );
        assert_eq!(
            dead,
            Some(Duration::MAX),
            "the dead turn's card never requests a frame"
        );
    }

    #[test]
    fn a_dead_turns_resultless_read_card_freezes() {
        // A read tool whose turn ended before the result landed: the
        // header's spin becomes the static dot, no repaint requests.
        let mut card = settled(ToolCard::new(call("read_file", r#"{"path":"a.rs"}"#)));
        let ctx = egui::Context::default();
        // Warm the context (egui's first frames request follow-ups of
        // their own), then measure.
        for _ in 0..3 {
            measured_repaint_delay(&ctx, &mut card, false);
        }
        let dead = measured_repaint_delay(&ctx, &mut card, false);
        assert_eq!(dead, Some(Duration::MAX), "dead read card: {dead:?}");
        measured_repaint_delay(&ctx, &mut card, true); // warm the live card
        assert!(
            measured_repaint_delay(&ctx, &mut card, true)
                .is_some_and(|delay| delay < Duration::from_secs(1)),
            "the same card animates while its turn is live"
        );
    }

    // -- web cards -----------------------------------------------------------------------------

    #[test]
    fn web_search_card_expands_to_rows_and_a_row_click_copies_the_url() {
        let mut card = settled(ToolCard::new(call(
            "web_search",
            r#"{"query":"ropey crate"}"#,
        )));
        card.result = Some(result(
            &card.call.id.clone(),
            true,
            "Web search results for \"ropey crate\":\n\
             1. ropey — Rope Science\n   https://docs.rs/ropey\n\
             2. GitHub - ropey\n   https://github.com/cessen/ropey\n",
        ));
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(outcome.rect.height(), CARD_HEIGHT);

        let header = outcome.header_rect.center();
        card_frame(&ctx, raw_input_with(click_at(header)), &mut card, false);
        assert!(card.expanded);
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(outcome.row_rects.len(), 2, "two numbered rows");

        let row = outcome.row_rects[1].center();
        let outcome = card_frame(&ctx, raw_input_with(click_at(row)), &mut card, false);
        assert_eq!(
            outcome.copy.as_deref(),
            Some("https://github.com/cessen/ropey"),
            "the row click copies the URL (the panel maps it to CopyCode)"
        );
    }

    #[test]
    fn web_fetch_card_paints_with_the_source_url_and_body() {
        let mut card = settled(ToolCard::new(call(
            "web_fetch",
            r#"{"url":"https://example.com"}"#,
        )));
        card.result = Some(result(
            &card.call.id.clone(),
            true,
            "Source: https://example.com\n\nThe fetched text body.\n",
        ));
        let ctx = egui::Context::default();
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert_eq!(outcome.rect.height(), CARD_HEIGHT);

        let header = outcome.header_rect.center();
        card_frame(&ctx, raw_input_with(click_at(header)), &mut card, false);
        assert!(card.expanded);
        let outcome = card_frame(&ctx, egui::RawInput::default(), &mut card, false);
        assert!(outcome.body_rect.is_some(), "the fetched text paints");
        assert!(outcome.row_rects.is_empty(), "no web rows on a fetch");
    }

    // -- status line -----------------------------------------------------------------------------

    #[test]
    fn status_line_paints_each_phase_and_stop_emits() {
        let ctx = egui::Context::default();
        for phase in [
            StatusPhase::Thinking,
            StatusPhase::Running("reading src/main.rs".to_owned()),
            StatusPhase::AwaitingApproval,
        ] {
            let outcome = status_frame(&ctx, egui::RawInput::default(), &phase, false);
            assert_eq!(outcome.rect.height(), STATUS_LINE_HEIGHT);
            assert!(!outcome.stop_clicked);
        }
        // Reduced motion: the static dots paint the same 24px line.
        let outcome = status_frame(
            &ctx,
            egui::RawInput::default(),
            &StatusPhase::Thinking,
            true,
        );
        assert_eq!(outcome.rect.height(), STATUS_LINE_HEIGHT);

        let outcome = status_frame(
            &ctx,
            egui::RawInput::default(),
            &StatusPhase::Thinking,
            false,
        );
        let stop = outcome.stop_rect.center();
        let outcome = status_frame(
            &ctx,
            raw_input_with(click_at(stop)),
            &StatusPhase::Thinking,
            false,
        );
        assert!(outcome.stop_clicked, "the Stop click lands");
    }

    // -- summary footer ----------------------------------------------------------------------------

    #[test]
    fn summary_footer_paints_left_aligned_24px() {
        let summary = AgentSummary {
            iterations: 3,
            tools_used: 6,
            approved: 3,
            denied: 1,
            tokens: 12_400,
            cost_usd: 0.0412,
            completed: true,
        };
        let mut rect = Rect::ZERO;
        egui::__run_test_ui(|ui| {
            rect = summary_footer(ui, &summary);
        });
        assert_eq!(rect.height(), SUMMARY_LINE_HEIGHT);
        assert_eq!(rect.left(), 0.0, "left-aligned (§4.2:298)");
    }

    // -- fade -----------------------------------------------------------------------------------------

    #[test]
    fn cards_fade_in_without_changing_geometry() {
        // appeared = now: the fade is ~0, but the card's layout is
        // identical to a settled card's.
        let mut fresh = ToolCard::new(call("read_file", r#"{"path":"a.rs"}"#));
        let mut settled_card = settled(ToolCard::new(call("read_file", r#"{"path":"a.rs"}"#)));
        let ctx = egui::Context::default();
        let fresh_outcome = card_frame(&ctx, egui::RawInput::default(), &mut fresh, false);
        let settled_outcome = card_frame(&ctx, egui::RawInput::default(), &mut settled_card, false);
        assert_eq!(fresh_outcome.rect, settled_outcome.rect);
    }

    // -- Phase 6h presence (avatar + orb) -------------------------------------

    #[test]
    fn status_running_tool_matches_the_phase_scan() {
        let mut pending = ToolCard::new(call("edit_file", r#"{"path":"a.rs"}"#));
        pending.approval = Some((
            ApprovalId::from_u128(1),
            ApprovalCard::Diff {
                path: "a.rs".to_owned(),
                diff: DIFF.to_owned(),
            },
            1,
        ));
        let running = ToolCard::new(call("bash", r#"{"command":"cargo test"}"#));
        let mut done = ToolCard::new(call("read_file", r#"{"path":"b.rs"}"#));
        done.result = Some(result(&done.call.id.clone(), true, "contents"));

        // A pending card parks the phase on AwaitingApproval: no running tool.
        assert_eq!(status_running_tool([&pending, &running, &done]), None);
        // Otherwise the latest resultless card's name.
        assert_eq!(status_running_tool([&running, &done]), Some("bash"));
        assert_eq!(status_running_tool([&done]), None);
    }

    #[test]
    fn the_card_header_paints_the_avatar_and_shifts_the_name_right() {
        let mut card = settled(ToolCard::new(call("read_file", r#"{"path":"a.rs"}"#)));
        card.result = Some(result(&card.call.id.clone(), true, "contents"));
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            tool_card(ui, &mut card, false, true, true);
        });
        // The avatar's open eyes: two --fg-muted dots (alpha-255 fills are
        // stored unpremultiplied).
        let eye_dots = output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.fill == color::FG_MUTED))
            .count();
        output.textures_delta.clear();
        assert_eq!(eye_dots, 2, "the avatar's two eyes paint");
    }

    #[test]
    fn a_disconnected_gateway_sleeps_the_card_avatar() {
        let mut card = settled(ToolCard::new(call("read_file", r#"{"path":"a.rs"}"#)));
        card.result = Some(result(&card.call.id.clone(), true, "contents"));
        let ctx = egui::Context::default();
        // Two frames: the first starts the Default → Sleeping cross-fade,
        // the second (a second later) is past it.
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            tool_card(ui, &mut card, false, false, false);
        });
        output.textures_delta.clear();
        let input = egui::RawInput {
            time: Some(1.0),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            tool_card(ui, &mut card, false, false, false);
        });
        // Sleeping: the eyes are hairlines, and there are no eye dots.
        let eye_dots = output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Circle(c) if c.fill == color::FG_MUTED))
            .count();
        output.textures_delta.clear();
        assert_eq!(eye_dots, 0, "no open eyes while asleep");
        assert_eq!(card.avatar.state(), AvatarState::Sleeping);
    }

    #[test]
    fn the_status_line_paints_avatar_orb_text_and_stop() {
        let ctx = egui::Context::default();
        let outcome = status_frame(
            &ctx,
            egui::RawInput::default(),
            &StatusPhase::Thinking,
            false,
        );
        assert_eq!(outcome.rect.height(), STATUS_LINE_HEIGHT);
        // The avatar left of the orb: the line's left edge hosts the
        // avatar's body (a filled rect), then the orb's dots.
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            let mut avatar = AgentAvatar::new(0.0);
            status_line(
                ui,
                &StatusPhase::Thinking,
                OrbState::Working,
                &mut avatar,
                true,
                false,
            );
        });
        let circles = output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Circle(_)))
            .count();
        output.textures_delta.clear();
        assert!(
            circles >= 5,
            "two avatar eyes + three orb dots at least: {circles}"
        );
    }
}
