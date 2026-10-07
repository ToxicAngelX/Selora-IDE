//! Chat sidebar content (UI_SPEC.md §4) — Phase 4c.
//!
//! The shell owns the right `egui::SidePanel` itself (360px default,
//! 240px min, 600px max, 1px hairline left border, `Ctrl+L` visibility
//! per UI_SPEC.md §2 and §15 — persisted in workspace state by the
//! shell); this widget paints the panel's content:
//!
//! - the 40px header: "CHAT" label, the model picker trigger
//!   (`claude-opus-5-5 ▾`) with its dropdown, and the `⋯` overflow menu
//!   (clear conversation, copy transcript, switch model),
//! - the conversation scroll: user bubbles (`--ink-mid`, right-aligned),
//!   assistant bubbles (`--ink-low`, left-aligned) with fenced code
//!   blocks (`--ink-canvas` + 1px `--ink-hi` border, JetBrains Mono, a
//!   hover "Copy" button top-right), and the streaming bubble with a
//!   comet fade-in on the first delta and the `wave` thinking indicator,
//! - the 60px composer: a single-line input ("Ask anything…"), Enter to
//!   send, Shift+Enter for a newline, and the `⏎` send button (disabled
//!   when the input is empty).
//!
//! The panel owns only *view* state (transcript, composer buffer,
//! in-flight id, picker open flags); the data flow is the effect/message
//! pattern: [`ChatPanelEvent::Send`] goes out, the shell builds a
//! [`selora_ai::ChatRequest`] from [`ChatPanel::conversation`] and
//! dispatches `Effect::SeloraChat`; the streamed `AppMessage::ChatDelta`
//! / `ChatDone` / `ChatError` values come back through
//! [`ChatPanel::push_assistant_delta`] /
//! [`ChatPanel::complete_stream`] / [`ChatPanel::fail_stream`], keyed by
//! [`ChatId`] (stale replies are dropped).
//!
//! The model picker dropdown is Phase 4b's widget
//! (`crate::picker::model_picker`, built in parallel); this module holds
//! a trait-free seam with the same event vocabulary
//! (`Select` / `Refresh` / close-on-outside-click) so the orchestrator
//! can swap the canonical picker in at merge. Persistence of the last 20
//! turns and of panel visibility lives in `selora-app`
//! (`workspace.json`); this module exposes the data via
//! [`ChatPanel::snapshot_turns`] and [`ChatPanel::restore_turns`].
//!
//! Every color, font, and spacing value comes from [`crate::theme`] (the
//! typed DESIGN_TOKENS.md contract); nothing is hardcoded here
//! (DESIGN_TOKENS.md §12). No color outside `--ok` / `--err` semantics.
//!
//! Agent Mode (Phase 5d; UI_SPEC.md §4.2, ARCHITECTURE.md §17): the
//! header's `agent` pill switches the composer submit from
//! [`ChatPanelEvent::Send`] to [`ChatPanelEvent::AgentSend`]; the
//! timeline then interleaves [`agent_cards`] tool cards and the turn
//! summary between the bubbles in arrival order, paints the 24px status
//! line under the last entry while the turn runs, and emits
//! [`ChatPanelEvent::AgentDecision`] for the card buttons and the Stop
//! affordances. Agent state arrives through the stale-gated inbound
//! methods ([`ChatPanel::tool_call_started`] and friends); cards and
//! summaries never enter [`ChatPanel::conversation`],
//! [`ChatPanel::snapshot_turns`], or [`ChatPanel::transcript`] (agent
//! transcripts are not persisted — the brief's DO-NOT).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui, Vec2};

use selora_ai::{
    AgentDecision, AgentSummary, ApprovalCard, ApprovalId, ChatMessage, Model, ModelId, Role,
    ToolCall, ToolResult,
};

use crate::sidebar::icons;
use crate::theme::{color, font, radius, space, type_scale};

/// Agent Mode cards (UI_SPEC.md §4.2) — Phase 5d's file.
pub mod agent_cards;

/// Header bar height (UI_SPEC.md §4: "Header bar (40px tall)").
pub const HEADER_HEIGHT: f32 = 40.0;
/// The panel's sidebar label (UI_SPEC.md §4).
pub const HEADER_LABEL: &str = "CHAT";
/// Composer height (UI_SPEC.md §4: "Composer (60px tall)").
pub const COMPOSER_HEIGHT: f32 = 60.0;
/// Conversation-area padding (UI_SPEC.md §4: "12px padding").
pub const AREA_PADDING: f32 = space::S3;
/// Bubble inset from the conversation-area edge on each side (UI_SPEC
/// §4: "Bubbles are full-width minus 16px on each side").
pub const BUBBLE_INSET: f32 = space::S4;
/// Bubble padding (UI_SPEC.md §4: "12px padding").
pub const BUBBLE_PADDING: f32 = space::S3;
/// Vertical gap between consecutive bubbles.
pub const BUBBLE_GAP: f32 = space::S2;
/// How many turns persist to `workspace.json` (Phase 4 brief, item 7).
pub const MAX_PERSISTED_TURNS: usize = 20;
/// Model picker dropdown width (UI_SPEC.md §4.1: 320px).
pub const MODEL_PICKER_WIDTH: f32 = 320.0;
/// Model picker row height (UI_SPEC.md §4.1: 32px).
pub const MODEL_ROW_HEIGHT: f32 = 32.0;
/// Model picker footer height (UI_SPEC.md §4.1: 24px).
pub const MODEL_FOOTER_HEIGHT: f32 = 24.0;
/// Model picker visible-row cap (UI_SPEC.md §4.1: scroll after 8).
pub const MODEL_PICKER_MAX_ROWS: usize = 8;
/// The composer input's placeholder (UI_SPEC.md §4).
const COMPOSER_HINT: &str = "Ask anything…";
/// The keyboard hint under the composer input (UI_SPEC.md §4 wireframe).
const COMPOSER_KEY_HINT: &str = "Enter to send, Shift+Enter for newline";
/// The send button's glyph (UI_SPEC.md §4: "⏎ glyph").
const SEND_GLYPH: &str = "⏎";
/// Send button size (width × height; a `--radius-1` chip).
const SEND_BUTTON_SIZE: Vec2 = Vec2::new(28.0, 20.0);
/// The overflow menu's glyph (UI_SPEC.md §4: "⋯ overflow menu").
const OVERFLOW_GLYPH: &str = "⋯";
/// The thinking wave indicator appears only after this long without a
/// delta (UI_SPEC.md §4: "longer than 800ms with no chunk").
const THINKING_AFTER: Duration = Duration::from_millis(800);
/// The streaming bubble's fade-in on the first delta (the `comet`
/// glyph's fade half; DESIGN_TOKENS.md §7.2 `--dur-base`).
const FADE_DURATION: Duration = Duration::from_millis(240);
/// The `wave` indicator's loop length (DESIGN_TOKENS.md §7.2
/// `--dur-loop`).
const WAVE_LOOP_MS: u64 = 1200;
/// The composer's stable widget id.
const COMPOSER_ID: &str = "selora_chat_composer";

// -----------------------------------------------------------------------------
// Ids and data types
// -----------------------------------------------------------------------------

/// Identifier of one chat request, assigned by the panel when the user
/// sends. Mirrors `selora_fs::search::SearchId`: a process-wide counter
/// the shell maps to the `Uuid` of the `Effect::SeloraChat` it
/// dispatches, and the streaming callbacks key on to drop stale replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChatId(u64);

impl ChatId {
    /// A fresh, never-before-returned id.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ChatId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// The raw counter value (debugging, logging).
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// One persisted conversation turn (the `workspace.json` unit; the shell
/// serializes the last [`MAX_PERSISTED_TURNS`] of these).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatTurn {
    pub role: Role,
    pub content: String,
}

impl ChatTurn {
    /// A user turn.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An assistant turn.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

// -----------------------------------------------------------------------------
// Events
// -----------------------------------------------------------------------------

/// What the panel tells the shell each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatPanelEvent {
    /// The user submitted composer text. The user bubble is already in
    /// the transcript and the stream is marked in-flight; the shell
    /// builds a `ChatRequest` (from [`ChatPanel::conversation`], which
    /// excludes the in-flight turn) and dispatches
    /// `Effect::SeloraChat(uuid, req)` while remembering its
    /// `ChatId -> uuid` mapping.
    Send { id: ChatId, text: String },
    /// The user submitted composer text with Agent Mode on (UI_SPEC.md
    /// §4.2): the shell builds the turn request (conversation + model +
    /// agent settings) and dispatches `Effect::AgentSend`
    /// (ARCHITECTURE.md §17.3/§17.6). Same id/stream discipline as
    /// [`Send`](Self::Send).
    AgentSend { id: ChatId, text: String },
    /// The user's answer to an approval card, or a Stop from the status
    /// line / a running bash card / Clear-during-an-agent-turn (§17.4).
    /// The shell reverse-maps `id` to the turn's `Uuid` and dispatches
    /// `Effect::AgentDecision`. `approval_id` is the pending card's id,
    /// or `Uuid::nil()` for a Stop that names no card (the loop honors
    /// Stop regardless).
    AgentDecision {
        id: ChatId,
        approval_id: ApprovalId,
        decision: AgentDecision,
    },
    /// The conversation was cleared (overflow menu); the shell should
    /// cancel any in-flight stream (`Effect::SeloraCancelChat`) and
    /// persist the now-empty transcript.
    Clear,
    /// A code block's Copy button was clicked; the code is already on
    /// the clipboard (`Context::copy_text`). Emitted so the shell can
    /// toast a confirmation.
    CopyCode(String),
    /// "Copy transcript" was chosen from the overflow menu; the text is
    /// already on the clipboard.
    CopyTranscript(String),
    /// A model was selected in the header's model picker. The panel has
    /// already switched its own header label; the shell persists the
    /// active model (next chat uses it).
    SelectModel(ModelId),
    /// The model picker's "live from gateway" footer was clicked; the
    /// shell re-fetches the catalog (`Effect::SeloraListModels`).
    RefreshModels,
}

/// Result of one `show` call. The `*_rects` fields are debugging /
/// headless-test aids (where the interactive elements landed this
/// frame); the shell ignores them — same precedent as
/// `SearchOutcome::rows_painted`.
#[derive(Debug)]
pub struct ChatPanelOutcome {
    /// Events for the shell, in frame order.
    pub events: Vec<ChatPanelEvent>,
    /// Bubbles painted this frame (user + assistant, streaming or not).
    pub bubbles_painted: usize,
    /// Code blocks painted inside assistant bubbles this frame.
    pub code_blocks_painted: usize,
    /// Rects of the code blocks painted this frame (the hover target for
    /// their Copy buttons).
    pub code_block_rects: Vec<Rect>,
    /// Rects of the Copy buttons actually shown this frame (code blocks
    /// under the pointer).
    pub copy_button_rects: Vec<Rect>,
    /// Rect of the composer's send button.
    pub send_button_rect: Rect,
    /// `(model id, row rect)` for the model picker's rows, when the
    /// dropdown is open.
    pub model_row_rects: Vec<(ModelId, Rect)>,
    /// `(item label, row rect)` for the overflow menu's items, when the
    /// menu is open.
    pub menu_item_rects: Vec<(&'static str, Rect)>,
    /// Tool cards painted this frame (Phase 5d).
    pub cards_painted: usize,
    /// `(label, rect)` for the card decision buttons painted this frame.
    pub decision_button_rects: Vec<(&'static str, Rect)>,
    /// The agent status line's rect, while a turn runs.
    pub status_line_rect: Option<Rect>,
    /// The status line's Stop button rect, while a turn runs.
    pub stop_button_rect: Option<Rect>,
    /// The header's agent-mode pill rect.
    pub agent_pill_rect: Rect,
    /// The turn summary footer's rect, when one painted this frame.
    pub summary_footer_rect: Option<Rect>,
}

impl Default for ChatPanelOutcome {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            bubbles_painted: 0,
            code_blocks_painted: 0,
            code_block_rects: Vec::new(),
            copy_button_rects: Vec::new(),
            send_button_rect: Rect::ZERO,
            model_row_rects: Vec::new(),
            menu_item_rects: Vec::new(),
            cards_painted: 0,
            decision_button_rects: Vec::new(),
            status_line_rect: None,
            stop_button_rect: None,
            agent_pill_rect: Rect::ZERO,
            summary_footer_rect: None,
        }
    }
}

/// Per-frame paint artifacts (internal accumulator; `show` folds it into
/// [`ChatPanelOutcome`]).
#[derive(Debug)]
struct FrameArtifacts {
    events: Vec<ChatPanelEvent>,
    bubbles_painted: usize,
    code_blocks_painted: usize,
    code_block_rects: Vec<Rect>,
    copy_button_rects: Vec<Rect>,
    send_button_rect: Rect,
    model_row_rects: Vec<(ModelId, Rect)>,
    menu_item_rects: Vec<(&'static str, Rect)>,
    cards_painted: usize,
    decision_button_rects: Vec<(&'static str, Rect)>,
    status_line_rect: Option<Rect>,
    stop_button_rect: Option<Rect>,
    agent_pill_rect: Rect,
    summary_footer_rect: Option<Rect>,
    /// Code copied this frame (applied by `show`: clipboard + event).
    copies: Vec<String>,
}

impl Default for FrameArtifacts {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            bubbles_painted: 0,
            code_blocks_painted: 0,
            code_block_rects: Vec::new(),
            copy_button_rects: Vec::new(),
            send_button_rect: Rect::ZERO,
            model_row_rects: Vec::new(),
            menu_item_rects: Vec::new(),
            cards_painted: 0,
            decision_button_rects: Vec::new(),
            status_line_rect: None,
            stop_button_rect: None,
            agent_pill_rect: Rect::ZERO,
            summary_footer_rect: None,
            copies: Vec::new(),
        }
    }
}

// -----------------------------------------------------------------------------
// Messages and rendering model
// -----------------------------------------------------------------------------

/// One transcript entry. A message that is mid-stream carries the
/// streaming flags; the fade/wave timers only apply while it is.
#[derive(Clone, Debug)]
struct Message {
    role: Role,
    content: String,
    /// An assistant reply currently being streamed.
    streaming: bool,
    /// When the stream started (the fade-in anchor, and the
    /// no-delta-yet thinking clock).
    stream_started: Option<Instant>,
    /// When the latest delta arrived (the wave indicator's window).
    last_delta: Option<Instant>,
    /// Why the stream failed, if it did.
    error: Option<String>,
}

impl Message {
    fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            streaming: false,
            stream_started: None,
            last_delta: None,
            error: None,
        }
    }

    fn completed_turn(&self) -> ChatTurn {
        ChatTurn {
            role: self.role,
            content: self.content.clone(),
        }
    }
}

/// One timeline row (Phase 5d): a chat bubble, an agent tool card, or a
/// turn summary footer. Cards and summaries interleave between bubbles in
/// arrival order (UI_SPEC.md §4.2); only bubbles are conversation context
/// and persistence.
#[derive(Clone, Debug)]
enum TimelineEntry {
    Bubble(Message),
    Card(agent_cards::ToolCard),
    Summary(AgentSummary),
}

/// One rendered piece of an assistant message: prose between fences, or
/// a fenced code block.
#[derive(Clone, Debug, PartialEq)]
enum Segment<'a> {
    /// Prose run between code fences.
    Prose(&'a str),
    /// A fenced code block. `complete` is `false` while the closing
    /// fence has not arrived yet (mid-stream).
    Code {
        lang: Option<String>,
        body: String,
        complete: bool,
    },
}

/// A code block accumulated line by line.
struct CodeAccumulator {
    lang: Option<String>,
    body: String,
}

/// Split an assistant message into prose and fenced code-block segments
/// (UI_SPEC.md §4: blocks are delimited by ``` fences; the language tag
/// after the opening fence is captured but not rendered). An open fence
/// at the end of the text yields an incomplete `Code` segment, which is
/// how a mid-stream block renders as it arrives.
fn parse_segments(content: &str) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut prose_start: Option<usize> = None;
    let mut prose_end = 0usize;
    let mut code: Option<CodeAccumulator> = None;

    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let next = offset + line.len();
        let bare = line.strip_suffix('\n').unwrap_or(line);
        match bare.trim_start().strip_prefix("```") {
            // A fence: closes an open block, or opens a new one.
            Some(rest) => match code.take() {
                Some(acc) => {
                    segments.push(Segment::Code {
                        lang: acc.lang,
                        body: acc.body,
                        complete: true,
                    });
                }
                None => {
                    if let Some(start) = prose_start.take() {
                        push_prose(&mut segments, &content[start..prose_end]);
                    }
                    let lang = rest.trim();
                    code = Some(CodeAccumulator {
                        lang: if lang.is_empty() {
                            None
                        } else {
                            Some(lang.to_owned())
                        },
                        body: String::new(),
                    });
                }
            },
            None => match code.as_mut() {
                Some(acc) => {
                    acc.body.push_str(bare);
                    acc.body.push('\n');
                }
                None => {
                    if prose_start.is_none() {
                        prose_start = Some(offset);
                    }
                    prose_end = next;
                }
            },
        }
        offset = next;
    }

    match code {
        // Still inside a fence (streaming): an incomplete block.
        Some(acc) => segments.push(Segment::Code {
            lang: acc.lang,
            body: acc.body,
            complete: false,
        }),
        None => {
            if let Some(start) = prose_start {
                push_prose(&mut segments, &content[start..prose_end]);
            }
        }
    }
    segments
}

/// Push a prose segment unless it is entirely whitespace.
fn push_prose<'a>(segments: &mut Vec<Segment<'a>>, text: &'a str) {
    if !text.trim().is_empty() {
        segments.push(Segment::Prose(text));
    }
}

/// One laid-out piece of a bubble.
enum Part {
    /// A wrapped prose galley.
    Prose { galley: Arc<egui::Galley> },
    /// A code block: its source (for Copy), the wrapped galley, the
    /// block height (with its 12px padding), and whether the fence
    /// closed (Copy only appears on closed blocks).
    Code {
        body: String,
        galley: Arc<egui::Galley>,
        block_height: f32,
        complete: bool,
    },
}

// -----------------------------------------------------------------------------
// ChatPanel
// -----------------------------------------------------------------------------

/// The chat sidebar content. Create one per app and call [`show`] every
/// frame from the shell's right `SidePanel`.
pub struct ChatPanel {
    /// The timeline (user/assistant bubbles interleaved with agent tool
    /// cards and turn summaries, in arrival order — Phase 5d).
    entries: Vec<TimelineEntry>,
    /// The composer's buffer (single-line edit; Shift+Enter appends).
    composer: String,
    /// Focus the composer on the next frame (panel-open wiring).
    want_focus: bool,
    /// The in-flight (or just-finished) request's id.
    in_flight: Option<ChatId>,
    /// The active model id (the header's label; set by the shell from
    /// settings and re-set when the picker selects).
    model: Option<ModelId>,
    /// The model catalog, fed by the shell from
    /// `AppMessage::ModelsLoaded`.
    models: Vec<Model>,
    /// Whether the model picker dropdown is open.
    picker_open: bool,
    /// Whether the `⋯` overflow menu is open.
    menu_open: bool,
    /// Agent Mode (the header pill, UI_SPEC.md §4.2): submits start agent
    /// turns instead of plain chats. Persisted in workspace.json by the
    /// shell (the `chat_open` precedent).
    agent_mode: bool,
    /// `ui.reduce_motion` (DESIGN_TOKENS.md §7.4), threaded by the shell:
    /// the cards' pulse/spin become static dots.
    reduce_motion: bool,
    /// An agent turn is in flight (set by an agent-mode submit; cleared
    /// by [`agent_turn_done`](Self::agent_turn_done) /
    /// [`clear`](Self::clear)).
    agent_in_flight: bool,
    /// The in-flight turn's approval counter (drives "Approve all"
    /// appearing from the second pending card, §4.2).
    turn_approvals: u32,
    /// The entries index where the in-flight turn's cards begin — the
    /// status line and its Stop consider only this turn's cards (a
    /// previous turn's latched card must never leak into the phase).
    turn_cards_start: usize,
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatPanel {
    /// An empty panel with no transcript and no model.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            composer: String::new(),
            want_focus: false,
            in_flight: None,
            model: None,
            models: Vec::new(),
            picker_open: false,
            menu_open: false,
            agent_mode: false,
            reduce_motion: false,
            agent_in_flight: false,
            turn_approvals: 0,
            turn_cards_start: 0,
        }
    }

    // -- shell-facing state seams ---------------------------------------------

    /// The active model id (drives the header label and the picker's
    /// selected row).
    pub fn model(&self) -> Option<&ModelId> {
        self.model.as_ref()
    }

    /// Set the active model (the shell applies
    /// `settings.selora.active_model` on boot, and status-bar /
    /// command-palette model switches).
    pub fn set_model(&mut self, model: impl Into<ModelId>) {
        self.model = Some(model.into());
    }

    /// The model catalog (fed from `AppMessage::ModelsLoaded`; the
    /// picker renders it).
    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// Install the model catalog.
    pub fn set_models(&mut self, models: Vec<Model>) {
        self.models = models;
    }

    /// Whether Agent Mode is on (the header pill; UI_SPEC.md §4.2). The
    /// shell reads this for the workspace.json snapshot.
    pub fn is_agent_mode(&self) -> bool {
        self.agent_mode
    }

    /// Set Agent Mode (the shell's boot restore; the pill toggles it
    /// in-place). Takes effect on the next submit — a running turn is
    /// untouched.
    pub fn set_agent_mode(&mut self, on: bool) {
        self.agent_mode = on;
    }

    /// Thread `ui.reduce_motion` (DESIGN_TOKENS.md §7.4) into the cards:
    /// pulse/spin become static dots. Called by the shell on settings
    /// load and on dialog adopt.
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
    }

    /// Whether reduced motion is on (the shell's threading assertions).
    pub fn is_reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// The in-flight turn's pending approval id, when a card is parked
    /// (the status-line Stop names it; the shell's tests drive
    /// `Effect::AgentDecision` for the parked card through it).
    pub fn pending_approval_id(&self) -> Option<ApprovalId> {
        self.current_pending_approval()
    }

    /// Open the model picker dropdown (the command palette's
    /// "Selora: Switch Model…" command lands here).
    pub fn open_model_picker(&mut self) {
        self.picker_open = true;
    }

    /// Whether the model picker dropdown is open.
    pub fn is_picker_open(&self) -> bool {
        self.picker_open
    }

    /// Whether a request is in flight (the composer is gated on this).
    pub fn is_streaming(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Mark `id` as the in-flight request. The panel does this itself on
    /// submit; this is the programmatic seam (tests, and later agent-mode
    /// turns that start a stream without the composer).
    pub fn begin_stream(&mut self, id: ChatId) {
        self.in_flight = Some(id);
    }

    /// The in-flight request's id (the shell matches it against the
    /// `ChatId -> uuid` map it keeps when dispatching).
    pub fn in_flight(&self) -> Option<ChatId> {
        self.in_flight
    }

    /// Focus the composer on the next frame (the `Ctrl+L` /
    /// activity-bar chat toggle).
    pub fn focus_composer(&mut self) {
        self.want_focus = true;
    }

    /// The composer's current text (tests, agent-mode injection).
    pub fn composer_text(&self) -> &str {
        &self.composer
    }

    /// Set the composer text programmatically (tests, prefill).
    pub fn set_composer_text(&mut self, text: &str) {
        self.composer = text.to_owned();
    }

    /// Number of timeline entries (bubbles, cards, summaries; including
    /// the streaming bubble).
    pub fn turn_count(&self) -> usize {
        self.entries.len()
    }

    /// The full transcript as wire-ready [`ChatMessage`]s for the next
    /// `ChatRequest`. The in-progress streaming bubble (if any) is
    /// excluded — it cannot be part of its own request — and so are
    /// failed turns (the shell surfaces those as toasts, not context).
    /// Cards and summaries are never context (§17.6: the loop tracks its
    /// own tool-call messages).
    pub fn conversation(&self) -> Vec<ChatMessage> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                TimelineEntry::Bubble(message) => Some(message),
                _ => None,
            })
            .filter(|message| !message.streaming && message.error.is_none())
            .map(|message| ChatMessage {
                role: message.role,
                content: message.content.clone(),
                // Panel turns are plain bubbles; agent tool calls and
                // results are the loop's business (Phase 5c), never
                // re-derived from the transcript.
                tool_calls: None,
                tool_call_id: None,
            })
            .collect()
    }

    /// The last [`MAX_PERSISTED_TURNS`] turns for `workspace.json`
    /// (conversations persist as the last 20 turns; Phase 4 brief,
    /// item 7). Mid-stream (failed or still-streaming) bubbles are not
    /// stable state and are skipped — as are cards and summaries (agent
    /// transcripts are not persisted). The shell serializes whatever this
    /// returns.
    pub fn snapshot_turns(&self) -> Vec<ChatTurn> {
        let stable: Vec<&Message> = self
            .entries
            .iter()
            .filter_map(|entry| match entry {
                TimelineEntry::Bubble(message) => Some(message),
                _ => None,
            })
            .filter(|message| !message.streaming && message.error.is_none())
            .collect();
        let start = stable.len().saturating_sub(MAX_PERSISTED_TURNS);
        stable[start..]
            .iter()
            .map(|message| message.completed_turn())
            .collect()
    }

    /// Replace the transcript from persisted turns (app boot, before the
    /// first frame). Restored turns are all complete bubbles; nothing
    /// streams, no turn is in flight.
    pub fn restore_turns(&mut self, turns: Vec<ChatTurn>) {
        self.entries = turns
            .into_iter()
            .map(|turn| TimelineEntry::Bubble(turn_into_message(turn)))
            .collect();
        self.in_flight = None;
        self.agent_in_flight = false;
        self.turn_approvals = 0;
        self.turn_cards_start = 0;
    }

    /// The transcript as a plain-text transcript ("You: …" /
    /// "Selora: …" blocks) — the overflow menu's "Copy transcript"
    /// payload. Bubbles only: the cards' detail is the loop's business.
    pub fn transcript(&self) -> String {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                TimelineEntry::Bubble(message) => Some(message),
                _ => None,
            })
            .map(|message| format!("{}: {}", role_label(message.role), message.content.trim()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    // -- streaming lifecycle (the shell drives these) --------------------------

    /// Append a user bubble (programmatic injection — the Enter path
    /// already echoes its own; do not call both for one message).
    pub fn push_user(&mut self, text: &str) {
        self.entries
            .push(TimelineEntry::Bubble(Message::user(text)));
    }

    /// Feed one streamed delta into the assistant bubble (from
    /// `AppMessage::ChatDelta`). Stale ids — anything but the in-flight
    /// request — are dropped. The first delta opens the bubble and
    /// starts its fade-in. When the last entry isn't a streaming
    /// assistant bubble (e.g. a tool card just interrupted), a fresh
    /// bubble opens — agent rounds separate for free.
    pub fn push_assistant_delta(&mut self, id: ChatId, delta: &str) {
        if self.in_flight != Some(id) {
            return;
        }
        let now = Instant::now();
        let opens_bubble = !matches!(
            self.entries.last(),
            Some(TimelineEntry::Bubble(message)) if message.streaming && message.role == Role::Assistant
        );
        if opens_bubble {
            self.entries.push(TimelineEntry::Bubble(Message {
                role: Role::Assistant,
                content: delta.to_owned(),
                streaming: true,
                stream_started: Some(now),
                last_delta: Some(now),
                error: None,
            }));
            return;
        }
        match self.entries.last_mut() {
            Some(TimelineEntry::Bubble(message)) => {
                message.content.push_str(delta);
                message.last_delta = Some(now);
            }
            _ => unreachable!("checked above"),
        }
    }

    /// End the stream (from `AppMessage::ChatDone`): the bubble stops
    /// streaming; a bubble that never received a delta is dropped.
    pub fn complete_stream(&mut self, id: ChatId) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        if let Some(TimelineEntry::Bubble(message)) = self.entries.last_mut() {
            message.streaming = false;
        }
        let is_empty_assistant = matches!(
            self.entries.last(),
            Some(TimelineEntry::Bubble(message)) if message.role == Role::Assistant && message.content.is_empty()
        );
        if is_empty_assistant {
            self.entries.pop();
        }
    }

    /// Fail the stream (from `AppMessage::ChatError`): the error is kept
    /// on the bubble and rendered under it; the composer unlocks.
    pub fn fail_stream(&mut self, id: ChatId, error: &str) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        match self.entries.last_mut() {
            Some(TimelineEntry::Bubble(message)) if message.role == Role::Assistant => {
                message.streaming = false;
                message.error = Some(error.to_owned());
            }
            _ => self.entries.push(TimelineEntry::Bubble(Message {
                role: Role::Assistant,
                content: String::new(),
                streaming: false,
                stream_started: None,
                last_delta: None,
                error: Some(error.to_owned()),
            })),
        }
    }

    // -- agent turn lifecycle (Phase 5d; the shell reduces the §17.6
    //    AppMessages into these, keyed by the same in-flight ChatId) ----

    /// `AppMessage::ToolCallStarted`: open the tool's card. Stale ids
    /// (anything but the in-flight request) are dropped, like deltas.
    pub fn tool_call_started(&mut self, id: ChatId, call: ToolCall) {
        if self.in_flight != Some(id) {
            return;
        }
        // A card interrupts the streaming bubble above it: finalize the
        // bubble so the wave stops and the round's text re-enters the
        // next request's context.
        self.finalize_last_streaming_bubble();
        self.entries
            .push(TimelineEntry::Card(agent_cards::ToolCard::new(call)));
    }

    /// `AppMessage::ApprovalNeeded`: attach the parked approval to the
    /// card the loop just started (the loop emits ApprovalNeeded right
    /// after its ToolCallStarted) and count the turn's approval ordinal
    /// ("Approve all" appears from the second pending card, §4.2).
    pub fn approval_needed(&mut self, id: ChatId, approval_id: ApprovalId, card: ApprovalCard) {
        if self.in_flight != Some(id) {
            return;
        }
        self.turn_approvals += 1;
        let ordinal = self.turn_approvals;
        for entry in self.entries.iter_mut().rev() {
            if let TimelineEntry::Card(tool_card) = entry {
                if tool_card.result.is_none() && tool_card.approval.is_none() {
                    tool_card.approval = Some((approval_id, card, ordinal));
                    return;
                }
            }
        }
    }

    /// `AppMessage::ToolResult`: land the result on its card (matched by
    /// the tool-call id the result carries).
    pub fn tool_result(&mut self, id: ChatId, result: ToolResult) {
        if self.in_flight != Some(id) {
            return;
        }
        for entry in self.entries.iter_mut().rev() {
            if let TimelineEntry::Card(card) = entry {
                if card.call.id == result.id {
                    card.result = Some(result);
                    return;
                }
            }
        }
    }

    /// `AppMessage::AgentTurnDone`: the turn ends; the summary footer
    /// lands under the final bubble (§4.2). EVERY streaming assistant
    /// bubble of the turn finalizes (complete_stream only clears the
    /// last one — earlier rounds' text would otherwise never re-enter
    /// the next request's context and their wave would animate forever),
    /// and cards still parked at the end (a Stop or cap while parked
    /// emits no ToolResult — the loop breaks out) latch so their buttons
    /// die with the turn.
    pub fn agent_turn_done(&mut self, id: ChatId, summary: AgentSummary) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        self.agent_in_flight = false;
        for entry in &mut self.entries {
            match entry {
                TimelineEntry::Bubble(message) => {
                    if message.streaming && message.role == Role::Assistant {
                        message.streaming = false;
                    }
                }
                TimelineEntry::Card(card) => {
                    if card.approval.is_some() && card.decided.is_none() && card.result.is_none() {
                        card.decided = Some(AgentDecision::Stop);
                    }
                }
                TimelineEntry::Summary(_) => {}
            }
        }
        self.entries.push(TimelineEntry::Summary(summary));
    }

    /// A Card or Summary entry interrupts the streaming bubble above it:
    /// finalize the bubble so the wave stops mid-turn.
    fn finalize_last_streaming_bubble(&mut self) {
        if let Some(TimelineEntry::Bubble(message)) = self.entries.last_mut() {
            if message.streaming {
                message.streaming = false;
            }
        }
    }

    /// The in-flight turn's pending approval id (the status-line Stop
    /// names it when a card is parked).
    fn current_pending_approval(&self) -> Option<ApprovalId> {
        self.entries[self.turn_cards_start.min(self.entries.len())..]
            .iter()
            .filter_map(|entry| match entry {
                TimelineEntry::Card(card) => Some(card),
                _ => None,
            })
            .find(|card| card.is_pending())
            .and_then(|card| card.approval_id())
    }

    /// Empty the transcript (the overflow menu's "Clear conversation").
    /// The shell still gets the [`ChatPanelEvent::Clear`] event and is
    /// responsible for cancelling any in-flight stream.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.in_flight = None;
        self.agent_in_flight = false;
        self.turn_approvals = 0;
        self.turn_cards_start = 0;
    }

    // -- painting ---------------------------------------------------------------

    /// Paint the panel content. Returns the frame's events plus the
    /// headless-test aids.
    ///
    /// The composer is reserved first (a bottom bar) and the header
    /// second (a top bar) so the transcript scroll — which claims all
    /// remaining space — can never push them off the panel.
    pub fn show(&mut self, ui: &mut Ui) -> ChatPanelOutcome {
        let mut frame = FrameArtifacts::default();
        egui::Panel::bottom(egui::Id::new("selora_chat_composer_bar"))
            .frame(egui::Frame::default())
            .exact_size(COMPOSER_HEIGHT)
            .show_separator_line(false)
            .show(ui, |ui| self.paint_composer(ui, &mut frame));
        egui::Panel::top(egui::Id::new("selora_chat_header_bar"))
            .frame(egui::Frame::default())
            .exact_size(HEADER_HEIGHT)
            .show_separator_line(false)
            .show(ui, |ui| self.paint_header(ui, &mut frame));
        self.paint_conversation(ui, &mut frame);
        for code in std::mem::take(&mut frame.copies) {
            ui.ctx().copy_text(code.clone());
            frame.events.push(ChatPanelEvent::CopyCode(code));
        }
        ChatPanelOutcome {
            events: frame.events,
            bubbles_painted: frame.bubbles_painted,
            code_blocks_painted: frame.code_blocks_painted,
            code_block_rects: frame.code_block_rects,
            copy_button_rects: frame.copy_button_rects,
            send_button_rect: frame.send_button_rect,
            model_row_rects: frame.model_row_rects,
            menu_item_rects: frame.menu_item_rects,
            cards_painted: frame.cards_painted,
            decision_button_rects: frame.decision_button_rects,
            status_line_rect: frame.status_line_rect,
            stop_button_rect: frame.stop_button_rect,
            agent_pill_rect: frame.agent_pill_rect,
            summary_footer_rect: frame.summary_footer_rect,
        }
    }

    /// The 40px header: "CHAT" label, the model picker trigger, the `⋯`
    /// overflow trigger, and both dropdowns (anchored overlays).
    fn paint_header(&mut self, ui: &mut Ui, frame: &mut FrameArtifacts) {
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
        let painter = ui.painter();
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            egui::Stroke::new(1.0, color::INK_HI),
        );
        painter.text(
            pos2(rect.left() + space::S3, rect.center().y),
            Align2::LEFT_CENTER,
            HEADER_LABEL,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );

        // -- model picker trigger (center) ---------------------------------
        let label = self.model.clone().unwrap_or_else(|| "no model".to_owned());
        let model_font = egui::FontId::new(type_scale::MD, font::FAMILY_SANS);
        let label_width = painter
            .layout_no_wrap(label.clone(), model_font.clone(), color::FG_DEFAULT)
            .size()
            .x;
        let chevron_slot = 16.0;
        let button_width = label_width + chevron_slot + space::S2;
        let button_rect = Rect::from_min_size(
            pos2(rect.center().x - button_width / 2.0, rect.top() + 8.0),
            vec2(button_width, 24.0),
        );
        let model_response = ui.interact(
            button_rect,
            egui::Id::new("selora_chat_model_button"),
            Sense::click(),
        );
        if model_response.hovered() {
            painter.rect_filled(button_rect, radius::R1, color::INK_FAINT);
        }
        painter.text(
            pos2(button_rect.left(), button_rect.center().y),
            Align2::LEFT_CENTER,
            label,
            model_font,
            color::FG_DEFAULT,
        );
        icons::paint_chevron(
            painter,
            Rect::from_min_size(
                pos2(
                    button_rect.right() - chevron_slot,
                    button_rect.center().y - 8.0,
                ),
                vec2(chevron_slot, 16.0),
            ),
            color::FG_FAINT,
            self.picker_open,
        );
        if model_response.clicked() {
            self.picker_open = !self.picker_open;
            self.menu_open = false;
        }

        // -- overflow trigger (right) ---------------------------------------
        let overflow_rect = Rect::from_min_size(
            pos2(rect.right() - 28.0, rect.top() + 8.0),
            vec2(24.0, 24.0),
        );
        let overflow_response = ui.interact(
            overflow_rect,
            egui::Id::new("selora_chat_overflow"),
            Sense::click(),
        );
        if overflow_response.hovered() {
            painter.rect_filled(overflow_rect, radius::R1, color::INK_FAINT);
        }
        painter.text(
            overflow_rect.center(),
            Align2::CENTER_CENTER,
            OVERFLOW_GLYPH,
            egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        if overflow_response.clicked() {
            self.menu_open = !self.menu_open;
            self.picker_open = false;
        }

        // -- agent-mode pill (UI_SPEC.md §4.2:241), left of the overflow --
        // A 24px chip: `--type-xs --fg-muted`, `--radius-pill`, 1px
        // `--ink-hi` border; on = an `--ok` dot left of the label.
        let pill_label = "agent";
        let pill_galley = painter.layout_no_wrap(
            pill_label.to_owned(),
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_MUTED,
        );
        let dot_slot = if self.agent_mode {
            6.0 + space::S1
        } else {
            0.0
        };
        let pill_width = dot_slot + pill_galley.size().x + 2.0 * space::S2;
        let pill_rect = Rect::from_min_size(
            pos2(
                overflow_rect.left() - space::S1 - pill_width,
                rect.top() + 8.0,
            ),
            vec2(pill_width, 24.0),
        );
        let pill_response = ui.interact(
            pill_rect,
            egui::Id::new("selora_chat_agent_pill"),
            Sense::click(),
        );
        if pill_response.hovered() {
            painter.rect_filled(pill_rect, radius::PILL, color::INK_FAINT);
        }
        painter.rect(
            pill_rect,
            radius::PILL,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(1.0, color::INK_HI),
            egui::StrokeKind::Inside,
        );
        let mut pill_text_x = pill_rect.left() + space::S2;
        if self.agent_mode {
            painter.circle_filled(
                pos2(pill_text_x + 3.0, pill_rect.center().y),
                3.0,
                color::OK,
            );
            pill_text_x += dot_slot;
        }
        painter.galley(
            pos2(
                pill_text_x,
                pill_rect.center().y - pill_galley.size().y / 2.0,
            ),
            pill_galley,
            color::FG_MUTED,
        );
        if pill_response.clicked() {
            self.agent_mode = !self.agent_mode;
        }
        frame.agent_pill_rect = pill_rect;

        self.paint_model_picker(&model_response, frame);
        self.paint_overflow_menu(ui, &overflow_response, frame);
    }

    /// The model picker dropdown (UI_SPEC.md §4.1): 320px wide, 32px rows
    /// with the selected row on `--ink-low` plus an `--ok` dot, and a
    /// 24px "live from gateway" footer that doubles as the refresh
    /// affordance (the picker seam's `Refresh` event). Anchored overlay
    /// via [`egui::Popup`] so it floats over the conversation instead of
    /// pushing it down.
    fn paint_model_picker(&mut self, anchor: &egui::Response, frame: &mut FrameArtifacts) {
        let mut open = self.picker_open;
        let models = self.models.clone();
        let selected = self.model.clone();
        let mut picked: Option<ModelId> = None;
        let mut refresh = false;

        let popup = egui::Popup::from_response(anchor)
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(MODEL_PICKER_WIDTH);
        let _ = popup.show(|popup_ui| {
            popup_ui.set_min_width(MODEL_PICKER_WIDTH);
            if models.is_empty() {
                let (empty_rect, _) = popup_ui.allocate_exact_size(
                    vec2(popup_ui.available_width(), MODEL_ROW_HEIGHT),
                    Sense::hover(),
                );
                popup_ui.painter().text(
                    pos2(empty_rect.left() + space::S3, empty_rect.center().y),
                    Align2::LEFT_CENTER,
                    "no models — connect to load the catalog",
                    egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                    color::FG_FAINT,
                );
            } else {
                let max_height = MODEL_ROW_HEIGHT * MODEL_PICKER_MAX_ROWS as f32;
                egui::ScrollArea::vertical()
                    .id_salt("selora_chat_model_rows")
                    .auto_shrink([false, true])
                    .max_height(max_height)
                    .show(popup_ui, |scroll| {
                        for model in &models {
                            let (row_rect, response) = scroll.allocate_exact_size(
                                vec2(scroll.available_width(), MODEL_ROW_HEIGHT),
                                Sense::click(),
                            );
                            let is_selected = selected.as_deref() == Some(model.id.as_str());
                            if is_selected {
                                scroll
                                    .painter()
                                    .rect_filled(row_rect, radius::R0, color::INK_LOW);
                            } else if response.hovered() {
                                scroll.painter().rect_filled(
                                    row_rect,
                                    radius::R0,
                                    color::INK_FAINT,
                                );
                            }
                            let text_color = if is_selected || response.hovered() {
                                color::FG_DEFAULT
                            } else {
                                color::FG_MUTED
                            };
                            scroll.painter().text(
                                pos2(row_rect.left() + space::S3, row_rect.center().y),
                                Align2::LEFT_CENTER,
                                model.id.as_str(),
                                egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
                                text_color,
                            );
                            // Family tag on the right (XS, faint); the
                            // --ok selected dot sits just left of it.
                            let tag_width = family_tag_width(scroll.painter(), model);
                            if tag_width > 0.0 {
                                scroll.painter().text(
                                    pos2(row_rect.right() - space::S3, row_rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    model.family.as_str(),
                                    egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                                    color::FG_FAINT,
                                );
                            }
                            if is_selected {
                                scroll.painter().circle_filled(
                                    pos2(
                                        row_rect.right() - space::S3 - tag_width - space::S2,
                                        row_rect.center().y,
                                    ),
                                    3.0,
                                    color::OK,
                                );
                            }
                            if response.clicked() {
                                picked = Some(model.id.clone());
                            }
                            frame.model_row_rects.push((model.id.clone(), row_rect));
                        }
                    });
            }

            // Footer: "N models · live from gateway" (the refresh
            // trigger — the picker seam's Refresh event).
            let (footer_rect, footer_response) = popup_ui.allocate_exact_size(
                vec2(popup_ui.available_width(), MODEL_FOOTER_HEIGHT),
                Sense::click(),
            );
            let count = models.len();
            let footer_text = format!("{count} model{} · live from gateway", plural(count));
            popup_ui.painter().text(
                pos2(footer_rect.left() + space::S3, footer_rect.center().y),
                Align2::LEFT_CENTER,
                footer_text,
                egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                color::FG_FAINT,
            );
            if footer_response.clicked() {
                refresh = true;
            }
        });

        if let Some(model) = picked {
            self.model = Some(model.clone());
            frame.events.push(ChatPanelEvent::SelectModel(model));
            open = false;
        }
        if refresh {
            frame.events.push(ChatPanelEvent::RefreshModels);
        }
        self.picker_open = open;
    }

    /// The `⋯` overflow menu (UI_SPEC.md §4: clear conversation, copy
    /// transcript, switch model).
    fn paint_overflow_menu(
        &mut self,
        ui: &mut Ui,
        anchor: &egui::Response,
        frame: &mut FrameArtifacts,
    ) {
        let items: [(&'static str, OverflowAction); 3] = [
            ("Clear conversation", OverflowAction::Clear),
            ("Copy transcript", OverflowAction::CopyTranscript),
            ("Switch model", OverflowAction::SwitchModel),
        ];
        let mut open = self.menu_open;
        let mut action: Option<OverflowAction> = None;

        let popup = egui::Popup::from_response(anchor)
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let _ = popup.show(|menu_ui| {
            menu_ui.set_min_width(160.0);
            for (label, item) in items {
                let (row_rect, response) = menu_ui
                    .allocate_exact_size(vec2(menu_ui.available_width(), 28.0), Sense::click());
                if response.hovered() {
                    menu_ui
                        .painter()
                        .rect_filled(row_rect, radius::R0, color::INK_FAINT);
                }
                menu_ui.painter().text(
                    pos2(row_rect.left() + space::S3, row_rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                    if response.hovered() {
                        color::FG_DEFAULT
                    } else {
                        color::FG_MUTED
                    },
                );
                if response.clicked() {
                    action = Some(item);
                }
                frame.menu_item_rects.push((label, row_rect));
            }
        });

        match action {
            Some(OverflowAction::Clear) => {
                // Clear's contract: the shell cancels in-flight work. For
                // an agent turn that cancel IS the Stop decision — else
                // the loop would keep burning tokens invisibly (§17.4).
                if self.agent_in_flight {
                    if let Some(id) = self.in_flight {
                        frame.events.push(ChatPanelEvent::AgentDecision {
                            id,
                            approval_id: ApprovalId::nil(),
                            decision: AgentDecision::Stop,
                        });
                    }
                }
                self.clear();
                frame.events.push(ChatPanelEvent::Clear);
                open = false;
            }
            Some(OverflowAction::CopyTranscript) => {
                let transcript = self.transcript();
                ui.ctx().copy_text(transcript.clone());
                frame
                    .events
                    .push(ChatPanelEvent::CopyTranscript(transcript));
                open = false;
            }
            Some(OverflowAction::SwitchModel) => {
                self.picker_open = true;
                open = false;
            }
            None => {}
        }
        self.menu_open = open;
    }

    /// The conversation scroll: bubbles, tool cards, the summary footer,
    /// and the agent status line while a turn runs (§4.2).
    fn paint_conversation(&mut self, ui: &mut Ui, frame: &mut FrameArtifacts) {
        let reduce_motion = self.reduce_motion;
        let in_flight = self.in_flight;
        let agent_in_flight = self.agent_in_flight;
        egui::ScrollArea::vertical()
            .id_salt("selora_chat_transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |scroll| {
                scroll.add_space(AREA_PADDING);
                let content_width = scroll.available_width();
                for index in 0..self.entries.len() {
                    match &mut self.entries[index] {
                        TimelineEntry::Bubble(message) => {
                            paint_message(scroll, message, index, content_width, frame);
                        }
                        TimelineEntry::Card(card) => {
                            // A card is live only if its OWN turn is: the
                            // in-flight turn's cards (index ≥ the turn's
                            // start) while it runs. A dead turn's
                            // resultless card freezes (no spin, no Stop).
                            let turn_live = agent_in_flight && index >= self.turn_cards_start;
                            let outcome =
                                agent_cards::tool_card(scroll, card, reduce_motion, turn_live);
                            if let (Some(decision), Some(id)) = (outcome.decision, in_flight) {
                                let approval_id =
                                    card.approval_id().unwrap_or_else(ApprovalId::nil);
                                frame.events.push(ChatPanelEvent::AgentDecision {
                                    id,
                                    approval_id,
                                    decision,
                                });
                            }
                            if let Some(url) = outcome.copy {
                                // The copy+toast seam: the shell toasts
                                // CopyCode; the clipboard is set by show().
                                frame.copies.push(url);
                            }
                            frame.cards_painted += 1;
                            frame.decision_button_rects.extend(outcome.button_rects);
                        }
                        TimelineEntry::Summary(summary) => {
                            let rect = agent_cards::summary_footer(scroll, summary);
                            frame.summary_footer_rect = Some(rect);
                        }
                    }
                    scroll.add_space(BUBBLE_GAP);
                }
                if agent_in_flight {
                    // The 24px status line under the last entry (§4.2):
                    // the phase derives from this turn's card states.
                    let phase = agent_cards::status_phase_for(
                        self.entries[self.turn_cards_start.min(self.entries.len())..]
                            .iter()
                            .filter_map(|entry| match entry {
                                TimelineEntry::Card(card) => Some(card),
                                _ => None,
                            }),
                    );
                    let status = agent_cards::status_line(scroll, &phase, reduce_motion);
                    frame.status_line_rect = Some(status.rect);
                    frame.stop_button_rect = Some(status.stop_rect);
                    if status.stop_clicked {
                        if let Some(id) = in_flight {
                            let approval_id = self
                                .current_pending_approval()
                                .unwrap_or_else(ApprovalId::nil);
                            frame.events.push(ChatPanelEvent::AgentDecision {
                                id,
                                approval_id,
                                decision: AgentDecision::Stop,
                            });
                        }
                    }
                }
                scroll.add_space(AREA_PADDING);
            });
    }

    /// The 60px composer: single-line input + `⏎` send button, then the
    /// keyboard hint. Enter sends (the single-line edit surrenders focus
    /// on Enter; `lost_focus` + the key still down is the documented
    /// submit signal); Shift+Enter appends a newline to the buffer.
    fn paint_composer(&mut self, ui: &mut Ui, frame: &mut FrameArtifacts) {
        let composer_top = ui.max_rect().top();
        ui.painter().hline(
            ui.max_rect().x_range(),
            composer_top,
            egui::Stroke::new(1.0, color::INK_HI),
        );
        let want_focus = self.want_focus;
        let mut enter = false;
        let mut shift_enter = false;
        let mut send_clicked = false;

        ui.add_space(AREA_PADDING);
        ui.horizontal(|row| {
            row.set_min_height(SEND_BUTTON_SIZE.y);
            row.add_space(AREA_PADDING);
            let response = row.add(
                egui::TextEdit::singleline(&mut self.composer)
                    .id(egui::Id::new(COMPOSER_ID))
                    .hint_text(COMPOSER_HINT)
                    .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS))
                    .desired_width(
                        (row.available_width() - SEND_BUTTON_SIZE.x - 2.0 * AREA_PADDING).max(40.0),
                    ),
            );
            if want_focus {
                response.request_focus();
            }

            // Enter (with or without Shift — egui's logical key match
            // ignores extra modifiers) surrenders the single-line edit's
            // focus; `lost_focus` with the key still pressed is the
            // submit signal. Whether that Enter carried Shift is read
            // off the key event itself (the live modifier state only
            // tracks held modifier keys, not event modifiers).
            enter = response.lost_focus() && row.input(|i| i.key_pressed(Key::Enter));
            shift_enter = enter
                && row.input(|i| {
                    i.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: Key::Enter,
                                modifiers,
                                ..
                            } if modifiers.shift
                        )
                    })
                });

            // [⏎] send button — --ink-faint, hover --ink-low, disabled
            // (FG_DISABLED, no fill) when empty or mid-stream.
            let send_enabled = !self.composer.trim().is_empty() && self.in_flight.is_none();
            let (send_rect, send_response) =
                row.allocate_exact_size(SEND_BUTTON_SIZE, Sense::click());
            if send_enabled {
                let fill = if send_response.hovered() {
                    color::INK_LOW
                } else {
                    color::INK_FAINT
                };
                row.painter().rect_filled(send_rect, radius::R1, fill);
            }
            row.painter().text(
                send_rect.center(),
                Align2::CENTER_CENTER,
                SEND_GLYPH,
                egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
                if send_enabled {
                    color::FG_DEFAULT
                } else {
                    color::FG_DISABLED
                },
            );
            frame.send_button_rect = send_rect;
            if send_response.clicked() && send_enabled {
                send_clicked = true;
            }
            row.add_space(AREA_PADDING);
        });
        self.want_focus = false;

        if shift_enter {
            // Appended at the buffer's end: the composer is single-line,
            // so the cursor sits at the end in normal typing flow.
            self.composer.push('\n');
        }
        // A plain Enter or the send button submits; a Shift+Enter only
        // inserted the newline above.
        if (enter && !shift_enter) || send_clicked {
            self.submit(&mut frame.events);
        }
        if enter {
            // Keep typing after a send (and after a Shift+Enter newline:
            // the edit surrendered focus on that Enter too).
            self.want_focus = true;
        }

        ui.add_space(space::S1);
        let (hint_rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
        ui.painter().text(
            pos2(hint_rect.left() + AREA_PADDING, hint_rect.center().y),
            Align2::LEFT_CENTER,
            COMPOSER_KEY_HINT,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );
    }

    /// Submit the composer: assign the id, echo the user bubble, clear
    /// the input, emit [`ChatPanelEvent::Send`] — or, with Agent Mode on,
    /// [`ChatPanelEvent::AgentSend`] (ARCHITECTURE.md §17.3). A no-op
    /// while empty or mid-stream.
    fn submit(&mut self, events: &mut Vec<ChatPanelEvent>) {
        if self.in_flight.is_some() {
            return;
        }
        let text = self.composer.trim().to_owned();
        if text.is_empty() {
            return;
        }
        let id = ChatId::next();
        self.in_flight = Some(id);
        self.push_user(&text);
        self.composer.clear();
        if self.agent_mode {
            self.agent_in_flight = true;
            self.turn_approvals = 0;
            self.turn_cards_start = self.entries.len();
            events.push(ChatPanelEvent::AgentSend { id, text });
        } else {
            events.push(ChatPanelEvent::Send { id, text });
        }
    }
}

/// The overflow menu's actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverflowAction {
    Clear,
    CopyTranscript,
    SwitchModel,
}

/// Restore a persisted turn as a (complete) transcript message.
fn turn_into_message(turn: ChatTurn) -> Message {
    Message {
        role: turn.role,
        content: turn.content,
        streaming: false,
        stream_started: None,
        last_delta: None,
        error: None,
    }
}

/// The transcript's speaker label for "Copy transcript".
fn role_label(role: Role) -> &'static str {
    match role {
        Role::User => "You",
        Role::Assistant => "Selora",
        Role::System => "System",
        // Tool results render as cards (Phase 5d), never as bubbles —
        // this label never appears for them.
        Role::Tool => "Tool",
    }
}

/// `""` for 1, `"s"` otherwise.
fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// The family tag's measured width (for placing the selected `--ok` dot
/// left of it without overlap).
fn family_tag_width(painter: &egui::Painter, model: &Model) -> f32 {
    if model.family.is_empty() {
        0.0
    } else {
        painter
            .layout_no_wrap(
                model.family.clone(),
                egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                color::FG_FAINT,
            )
            .size()
            .x
    }
}

// -----------------------------------------------------------------------------
// Bubble painting (free functions: the paint loop borrows the transcript
// immutably while click targets are collected for later application)
// -----------------------------------------------------------------------------

/// Paint one bubble. User bubbles are fitted and right-aligned on
/// `--ink-mid`; assistant (and system) bubbles span the full width minus
/// the insets on `--ink-low`, with fenced code blocks inside and the
/// streaming fade / wave indicators when mid-stream.
fn paint_message(
    ui: &mut Ui,
    message: &Message,
    message_index: usize,
    content_width: f32,
    frame: &mut FrameArtifacts,
) {
    let is_user = message.role == Role::User;
    let max_bubble_width = (content_width - 2.0 * BUBBLE_INSET).max(60.0);

    // The fade-in (the comet glyph's fade half) over the first moments
    // of a streaming bubble.
    let fade = match (message.streaming, message.stream_started) {
        (true, Some(started)) => {
            (started.elapsed().as_secs_f32() / FADE_DURATION.as_secs_f32()).clamp(0.0, 1.0)
        }
        _ => 1.0,
    };
    let bubble_fill = with_alpha(
        if is_user {
            color::INK_MID
        } else {
            color::INK_LOW
        },
        fade,
    );

    if is_user {
        // Fitted bubble: measure the wrapped text, then size to it.
        let wrap = (max_bubble_width - 2.0 * BUBBLE_PADDING).max(20.0);
        let galley = ui.painter().layout(
            message.content.clone(),
            prose_font(),
            with_alpha(color::FG_DEFAULT, fade),
            wrap,
        );
        let bubble_size = galley.size() + vec2(2.0 * BUBBLE_PADDING, 2.0 * BUBBLE_PADDING);
        let (row_rect, _) =
            ui.allocate_exact_size(vec2(content_width, bubble_size.y), Sense::hover());
        let bubble_rect = Rect::from_min_size(
            pos2(
                row_rect.right() - BUBBLE_INSET - bubble_size.x,
                row_rect.top(),
            ),
            bubble_size,
        );
        ui.painter()
            .rect_filled(bubble_rect, radius::R1, bubble_fill);
        ui.painter().galley(
            pos2(
                bubble_rect.left() + BUBBLE_PADDING,
                bubble_rect.top() + BUBBLE_PADDING,
            ),
            galley,
            with_alpha(color::FG_DEFAULT, fade),
        );
        frame.bubbles_painted += 1;
        return;
    }

    // Assistant: lay out the segments, then paint.
    let wrap = (max_bubble_width - 2.0 * BUBBLE_PADDING).max(20.0);
    let code_wrap = (wrap - 2.0 * BUBBLE_PADDING).max(20.0);
    let mut parts: Vec<Part> = Vec::new();
    let mut content_height = 0.0;
    for segment in parse_segments(&message.content) {
        if content_height > 0.0 {
            content_height += BUBBLE_GAP; // gap between prose and blocks
        }
        match segment {
            Segment::Prose(text) => {
                let galley = ui.painter().layout(
                    text.to_owned(),
                    prose_font(),
                    with_alpha(color::FG_DEFAULT, fade),
                    wrap,
                );
                content_height += galley.size().y;
                parts.push(Part::Prose { galley });
            }
            Segment::Code { body, complete, .. } => {
                let galley = ui.painter().layout(
                    body.clone(),
                    code_font(),
                    with_alpha(color::FG_DEFAULT, fade),
                    code_wrap,
                );
                let block_height = galley.size().y + 2.0 * BUBBLE_PADDING;
                content_height += block_height;
                parts.push(Part::Code {
                    body,
                    galley,
                    block_height,
                    complete,
                });
                frame.code_blocks_painted += 1;
            }
        }
    }

    // The wave indicator (three pulsing dots): after 800ms without a
    // delta, or while waiting for the first one. An empty bubble
    // reserves the wave's line so its height is stable as it streams.
    let thinking = message
        .last_delta
        .is_some_and(|delta| delta.elapsed() > THINKING_AFTER);
    let show_wave = message.streaming && (thinking || message.content.is_empty());
    if show_wave {
        content_height += 12.0;
    }

    let bubble_height = content_height + 2.0 * BUBBLE_PADDING;
    let (row_rect, _) = ui.allocate_exact_size(vec2(content_width, bubble_height), Sense::hover());
    let bubble_rect = Rect::from_min_size(
        pos2(row_rect.left() + BUBBLE_INSET, row_rect.top()),
        vec2(max_bubble_width, bubble_height),
    );
    ui.painter()
        .rect_filled(bubble_rect, radius::R1, bubble_fill);

    let mut cursor = bubble_rect.top() + BUBBLE_PADDING;
    let mut block_index = 0usize;
    for part in &parts {
        match part {
            Part::Prose { galley } => {
                ui.painter().galley(
                    pos2(bubble_rect.left() + BUBBLE_PADDING, cursor),
                    galley.clone(),
                    with_alpha(color::FG_DEFAULT, fade),
                );
                cursor += galley.size().y;
            }
            Part::Code {
                body,
                galley,
                block_height,
                complete,
            } => {
                let block_rect = Rect::from_min_size(
                    pos2(bubble_rect.left() + BUBBLE_PADDING, cursor),
                    vec2(bubble_rect.width() - 2.0 * BUBBLE_PADDING, *block_height),
                );
                let this_block = block_index;
                block_index += 1;
                ui.painter().rect_filled(
                    block_rect,
                    radius::R1,
                    with_alpha(color::INK_CANVAS, fade),
                );
                ui.painter().rect(
                    block_rect,
                    radius::R1,
                    with_alpha(color::INK_CANVAS, fade),
                    egui::Stroke::new(1.0, with_alpha(color::INK_HI, fade)),
                    egui::StrokeKind::Inside,
                );
                ui.painter().galley(
                    pos2(
                        block_rect.left() + BUBBLE_PADDING,
                        block_rect.top() + BUBBLE_PADDING,
                    ),
                    galley.clone(),
                    with_alpha(color::FG_DEFAULT, fade),
                );
                frame.code_block_rects.push(block_rect);

                // Copy button (top-right, only on hover of the block,
                // only on closed blocks).
                let block_hovered = ui.rect_contains_pointer(block_rect);
                if block_hovered && *complete {
                    let copy_rect = Rect::from_min_size(
                        pos2(block_rect.right() - 4.0 - 40.0, block_rect.top() + 4.0),
                        vec2(40.0, 16.0),
                    );
                    let copy_response = ui.interact(
                        copy_rect,
                        egui::Id::new(("selora_chat_code_copy", message_index, this_block)),
                        Sense::click(),
                    );
                    let text_color = if copy_response.hovered() {
                        color::FG_DEFAULT
                    } else {
                        color::FG_MUTED
                    };
                    ui.painter().text(
                        copy_rect.center(),
                        Align2::CENTER_CENTER,
                        "Copy",
                        egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                        text_color,
                    );
                    frame.copy_button_rects.push(copy_rect);
                    if copy_response.clicked() {
                        frame.copies.push(body.clone());
                    }
                }
                cursor += *block_height;
            }
        }
        cursor += BUBBLE_GAP;
    }

    if show_wave {
        let anchor = message.last_delta.or(message.stream_started);
        if let Some(anchor) = anchor {
            paint_wave(
                ui.painter(),
                pos2(bubble_rect.left() + BUBBLE_PADDING, cursor),
                anchor.elapsed().as_millis() as u64,
            );
        }
    }

    // A failed stream renders its note under the bubble (the only red
    // outside diagnostics: --err, per the phase's color discipline).
    if let Some(error) = &message.error {
        let note = format!("stream failed: {error}");
        let galley = ui.painter().layout_no_wrap(
            note,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::ERR,
        );
        let (note_rect, _) = ui.allocate_exact_size(vec2(content_width, 16.0), Sense::hover());
        ui.painter().galley(
            pos2(
                note_rect.left() + BUBBLE_INSET,
                note_rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            color::ERR,
        );
    }

    frame.bubbles_painted += 1;
}

/// The three-dot `wave` indicator: each dot bobs on its own 200ms phase
/// of the 1200ms `--dur-loop` (DESIGN_TOKENS.md §7.3).
fn paint_wave(painter: &egui::Painter, origin: egui::Pos2, elapsed_ms: u64) {
    for dot in 0..3u64 {
        let phase = ((elapsed_ms + dot * 200) % WAVE_LOOP_MS) as f32 / WAVE_LOOP_MS as f32;
        let (dy, opacity) = wave_keyframe(phase);
        let center = pos2(origin.x + 3.0 + dot as f32 * 8.0, origin.y + 4.0 + dy);
        painter.circle_filled(center, 1.75, with_alpha(color::FG_DEFAULT, opacity));
    }
}

/// One dot's offset/opacity over the wave loop (rise 0–30%, fall
/// 30–60%, rest 60–100%).
fn wave_keyframe(phase: f32) -> (f32, f32) {
    if phase < 0.3 {
        let t = phase / 0.3;
        (-3.0 * t, 0.5 + 0.5 * t)
    } else if phase < 0.6 {
        let t = (phase - 0.3) / 0.3;
        (-3.0 * (1.0 - t), 1.0 - 0.5 * t)
    } else {
        (0.0, 0.5)
    }
}

/// Prose font: `--type-md` in `--font-sans` (DESIGN_TOKENS.md §2).
fn prose_font() -> egui::FontId {
    egui::FontId::new(type_scale::MD, font::FAMILY_SANS)
}

/// Code font: `--type-sm` in `--font-mono` (JetBrains Mono).
fn code_font() -> egui::FontId {
    egui::FontId::new(type_scale::SM, font::FAMILY_MONO)
}

/// A color with its opacity scaled (the fade-in).
fn with_alpha(color: egui::Color32, alpha: f32) -> egui::Color32 {
    let alpha = alpha.clamp(0.0, 1.0);
    egui::Color32::from_rgba_unmultiplied(
        color.r(),
        color.g(),
        color.b(),
        (alpha * 255.0).round() as u8,
    )
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// The headless viewport the tests paint into. `Context::run_ui`
    /// otherwise defaults to a 10000×10000 screen, which pushes the
    /// composer below the pointer's reach.
    fn test_screen() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 600.0))
    }

    /// A focused window's input carrying the given events (focus is what
    /// makes egui deliver key events to the widgets at all).
    fn raw_input_with(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(test_screen()),
            events,
            ..Default::default()
        }
    }

    /// One headless frame over a shared context; returns the outcome.
    fn frame(
        ctx: &egui::Context,
        input: egui::RawInput,
        panel: &mut ChatPanel,
    ) -> ChatPanelOutcome {
        let mut input = input;
        if input.screen_rect.is_none() {
            input.screen_rect = Some(test_screen());
        }
        let mut outcome = ChatPanelOutcome::default();
        let mut output = ctx.run_ui(input, |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            outcome = panel.show(ui);
        });
        output.textures_delta.clear();
        outcome
    }

    /// Click at `pos` (move, press, release in one frame's events).
    fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
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

    /// Send a chat message programmatically (mirrors the Enter path).
    fn send(panel: &mut ChatPanel) -> Option<ChatId> {
        let mut events = Vec::new();
        panel.submit(&mut events);
        match events.pop() {
            Some(ChatPanelEvent::Send { id, .. }) => Some(id),
            _ => None,
        }
    }

    fn model(id: &str) -> Model {
        Model {
            id: id.to_owned(),
            display_name: id.to_owned(),
            context_window: 128_000,
            price_per_1k_input: 0.0,
            price_per_1k_output: 0.0,
            supports_streaming: true,
            supports_tools: false,
            family: "glm".to_owned(),
        }
    }

    // -- composer: Enter / Shift+Enter / disabled states ----------------------

    #[test]
    fn enter_sends_pushes_the_user_bubble_and_clears_the_composer() {
        let mut panel = ChatPanel::new();
        panel.focus_composer();
        panel.set_composer_text("write a haiku about rust");

        // Frame 1 grants the input focus; frame 2 delivers Enter — the
        // single-line edit surrenders focus, which is the submit signal.
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(
            &ctx,
            raw_input_with(vec![key_event(Key::Enter, Default::default())]),
            &mut panel,
        );

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::Send { id, text } => {
                assert_eq!(text, "write a haiku about rust");
                assert_eq!(panel.in_flight(), Some(*id));
                assert!(panel.is_streaming());
            }
            other => panic!("expected Send, got {other:?}"),
        }
        // The user bubble is echoed, the composer is cleared.
        assert_eq!(panel.turn_count(), 1);
        assert_eq!(
            panel.conversation(),
            vec![ChatMessage::user("write a haiku about rust")]
        );
        assert_eq!(panel.composer_text(), "");
    }

    #[test]
    fn shift_enter_inserts_a_newline_without_sending() {
        let mut panel = ChatPanel::new();
        panel.focus_composer();
        panel.set_composer_text("line one");

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(
            &ctx,
            raw_input_with(vec![key_event(Key::Enter, egui::Modifiers::SHIFT)]),
            &mut panel,
        );

        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);
        assert_eq!(panel.composer_text(), "line one\n");
        assert_eq!(panel.turn_count(), 0);
        assert!(!panel.is_streaming());
    }

    #[test]
    fn enter_without_composer_focus_does_not_send() {
        // Enter landing elsewhere in the app must not send a chat.
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let ctx = egui::Context::default();
        let outcome = frame(
            &ctx,
            raw_input_with(vec![key_event(Key::Enter, Default::default())]),
            &mut panel,
        );
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);
        assert_eq!(panel.turn_count(), 0);
    }

    #[test]
    fn empty_composer_does_not_send() {
        // Enter and the submit seam both no-op on empty (or whitespace).
        let mut panel = ChatPanel::new();
        panel.focus_composer();
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(
            &ctx,
            raw_input_with(vec![key_event(Key::Enter, Default::default())]),
            &mut panel,
        );
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);
        assert!(!panel.is_streaming());

        panel.set_composer_text("   ");
        assert!(send(&mut panel).is_none());
        assert_eq!(panel.turn_count(), 0);
    }

    #[test]
    fn send_button_click_emits_send() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hello");
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let center = outcome.send_button_rect.center();
        let outcome = frame(&ctx, raw_input_with(click_at(center)), &mut panel);

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        assert!(matches!(&outcome.events[0], ChatPanelEvent::Send { text, .. } if text == "hello"));
        assert_eq!(panel.turn_count(), 1);
    }

    #[test]
    fn send_button_click_with_empty_composer_does_nothing() {
        let mut panel = ChatPanel::new();
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let center = outcome.send_button_rect.center();
        let outcome = frame(&ctx, raw_input_with(click_at(center)), &mut panel);
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);
        assert_eq!(panel.turn_count(), 0);
    }

    #[test]
    fn send_is_blocked_while_a_stream_is_in_flight() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("first");
        let id = send(&mut panel).expect("first send");
        panel.push_assistant_delta(id, "…");

        // Enter while streaming: no second request.
        panel.set_composer_text("second");
        panel.focus_composer();
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(
            &ctx,
            raw_input_with(vec![key_event(Key::Enter, Default::default())]),
            &mut panel,
        );
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);

        // The send button is equally gated.
        let center = outcome.send_button_rect.center();
        let outcome = frame(&ctx, raw_input_with(click_at(center)), &mut panel);
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);
        assert_eq!(panel.turn_count(), 2); // user bubble + streaming bubble
    }

    // -- streaming lifecycle ----------------------------------------------------

    #[test]
    fn deltas_accumulate_into_the_streaming_bubble_then_complete() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");

        panel.push_assistant_delta(id, "The Buffer");
        panel.push_assistant_delta(id, "::open method");
        panel.push_assistant_delta(id, " takes a path.");

        assert!(panel.is_streaming());
        assert_eq!(panel.turn_count(), 2);
        assert_eq!(
            panel.conversation(),
            vec![ChatMessage::user("hi")],
            "the streaming bubble is not request context"
        );

        panel.complete_stream(id);
        assert!(!panel.is_streaming());
        assert_eq!(panel.turn_count(), 2);
        assert_eq!(
            panel.conversation(),
            vec![
                ChatMessage::user("hi"),
                ChatMessage::assistant("The Buffer::open method takes a path."),
            ]
        );
    }

    #[test]
    fn stale_deltas_and_completions_are_dropped() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");

        let stale = ChatId::next();
        panel.push_assistant_delta(stale, "stale");
        panel.complete_stream(stale);
        assert!(
            panel.is_streaming(),
            "a stale completion must not end the stream"
        );
        assert_eq!(
            panel.turn_count(),
            1,
            "a stale delta must not open a bubble"
        );

        panel.push_assistant_delta(id, "fresh");
        panel.fail_stream(stale, "error");
        assert!(
            panel.is_streaming(),
            "a stale failure must not fail the stream"
        );
        panel.complete_stream(id);
        assert!(!panel.is_streaming());
        assert_eq!(panel.turn_count(), 2);
    }

    #[test]
    fn completing_without_deltas_drops_the_empty_bubble() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.complete_stream(id);
        assert_eq!(panel.turn_count(), 1); // just the user bubble
        assert!(!panel.is_streaming());
    }

    #[test]
    fn failing_the_stream_records_the_error_and_unlocks_the_composer() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "partial ");
        panel.fail_stream(id, "gateway unreachable");

        assert!(!panel.is_streaming());
        // The partial reply stays; the error rides on the bubble.
        assert_eq!(panel.turn_count(), 2);
        assert_eq!(
            panel.entries.last().and_then(|entry| match entry {
                TimelineEntry::Bubble(message) => message.error.as_deref(),
                _ => None,
            }),
            Some("gateway unreachable")
        );
        // The failed turn is excluded from request context.
        assert_eq!(panel.conversation(), vec![ChatMessage::user("hi")]);

        // The composer unlocks: a new send goes through.
        panel.set_composer_text("again");
        assert!(send(&mut panel).is_some());
    }

    #[test]
    fn fail_stream_without_deltas_creates_an_error_note() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.fail_stream(id, "key invalid");
        assert_eq!(panel.turn_count(), 2);
        assert!(panel.entries.last().is_some_and(|entry| matches!(entry,
            TimelineEntry::Bubble(message) if message.error.as_deref() == Some("key invalid"))));
    }

    // -- rendering ---------------------------------------------------------------

    #[test]
    fn bubbles_and_code_blocks_paint_headlessly() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("how do I open a buffer?");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(
            id,
            "Sure — here's the signature:\n\n```rust\npub fn insert(&mut self, pos: Position)\n```\n\nand prose after.",
        );
        panel.complete_stream(id);
        panel.push_user("thanks");

        let outcome = {
            let mut outcome = ChatPanelOutcome::default();
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                outcome = panel.show(ui);
            });
            outcome
        };
        assert_eq!(outcome.bubbles_painted, 3); // user, assistant, user
        assert_eq!(outcome.code_blocks_painted, 1);
        assert_eq!(outcome.code_block_rects.len(), 1);
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn streaming_bubble_paints_and_counts_as_one() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "streaming text");

        let outcome = {
            let mut outcome = ChatPanelOutcome::default();
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                outcome = panel.show(ui);
            });
            outcome
        };
        assert_eq!(outcome.bubbles_painted, 2);
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn empty_panel_paints_nothing() {
        let mut panel = ChatPanel::new();
        let outcome = {
            let mut outcome = ChatPanelOutcome::default();
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                outcome = panel.show(ui);
            });
            outcome
        };
        assert_eq!(outcome.bubbles_painted, 0);
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn code_block_copy_puts_the_code_on_the_clipboard() {
        let mut panel = ChatPanel::new();
        panel.push_user("show me code");
        panel.entries.push(TimelineEntry::Bubble(turn_into_message(
            ChatTurn::assistant("here:\n\n```rust\nfn main() {}\n```\n"),
        )));

        // Frame 1: paint, then hover the code block — the Copy button
        // appears and is registered for hit testing.
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert_eq!(outcome.code_block_rects.len(), 1);
        let block_center = outcome.code_block_rects[0].center();

        let outcome = frame(
            &ctx,
            raw_input_with(vec![egui::Event::PointerMoved(block_center)]),
            &mut panel,
        );
        assert!(
            !outcome.copy_button_rects.is_empty(),
            "hover must reveal the Copy button"
        );
        let copy_center = outcome.copy_button_rects[0].center();

        // Frame 2: click it.
        let outcome = frame(&ctx, raw_input_with(click_at(copy_center)), &mut panel);
        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::CopyCode(code) => assert_eq!(code, "fn main() {}\n"),
            other => panic!("expected CopyCode, got {other:?}"),
        }
    }

    #[test]
    fn incomplete_code_block_has_no_copy_button() {
        let mut panel = ChatPanel::new();
        panel.push_user("show me code");
        // An open fence (mid-stream): no closing fence, no Copy button.
        panel.entries.push(TimelineEntry::Bubble(turn_into_message(
            ChatTurn::assistant("```\nfn partial(\n"),
        )));

        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let block_center = outcome.code_block_rects[0].center();
        let outcome = frame(
            &ctx,
            raw_input_with(vec![egui::Event::PointerMoved(block_center)]),
            &mut panel,
        );
        assert!(outcome.copy_button_rects.is_empty());
    }

    // -- clear / overflow menu ----------------------------------------------------

    #[test]
    fn clear_empties_the_transcript() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "hello");
        panel.complete_stream(id);
        assert_eq!(panel.turn_count(), 2);

        panel.clear();
        assert_eq!(panel.turn_count(), 0);
        assert!(!panel.is_streaming());
        assert!(panel.snapshot_turns().is_empty());
    }

    #[test]
    fn overflow_menu_clear_emits_clear_and_empties_the_transcript() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.complete_stream(id);
        panel.menu_open = true;

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        // Second warm-up frame: the popup sizes on its first open frame.
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let clear_rect = outcome
            .menu_item_rects
            .iter()
            .find(|(label, _)| *label == "Clear conversation")
            .map(|(_, rect)| *rect)
            .expect("clear menu item");
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(clear_rect.center())),
            &mut panel,
        );

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        assert!(matches!(outcome.events[0], ChatPanelEvent::Clear));
        assert_eq!(panel.turn_count(), 0);
    }

    #[test]
    fn overflow_menu_copy_transcript_emits_the_transcript() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hello");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "world");
        panel.complete_stream(id);
        panel.menu_open = true;

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let copy_rect = outcome
            .menu_item_rects
            .iter()
            .find(|(label, _)| *label == "Copy transcript")
            .map(|(_, rect)| *rect)
            .expect("copy transcript menu item");
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(copy_rect.center())),
            &mut panel,
        );

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::CopyTranscript(text) => {
                assert!(text.contains("You: hello"));
                assert!(text.contains("Selora: world"));
            }
            other => panic!("expected CopyTranscript, got {other:?}"),
        }
    }

    #[test]
    fn overflow_menu_switch_model_opens_the_picker() {
        let mut panel = ChatPanel::new();
        panel.set_models(vec![model("glm-5.3")]);
        panel.menu_open = true;

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let switch_rect = outcome
            .menu_item_rects
            .iter()
            .find(|(label, _)| *label == "Switch model")
            .map(|(_, rect)| *rect)
            .expect("switch model menu item");
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(switch_rect.center())),
            &mut panel,
        );
        assert!(outcome.events.is_empty());
        assert!(panel.is_picker_open());
    }

    // -- model picker ---------------------------------------------------------------

    #[test]
    fn model_picker_selection_passes_through() {
        let mut panel = ChatPanel::new();
        panel.set_model("claude-opus-5-5");
        panel.set_models(vec![
            model("claude-opus-5-5"),
            model("claude-sonnet-5-5"),
            model("glm-5.3"),
        ]);
        panel.open_model_picker();

        let ctx = egui::Context::default();
        // Two warm-up frames: the popup sizes on its first open frame.
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert_eq!(outcome.model_row_rects.len(), 3);

        let target = outcome
            .model_row_rects
            .iter()
            .find(|(id, _)| id == "glm-5.3")
            .map(|(_, rect)| *rect)
            .expect("glm row");
        let outcome = frame(&ctx, raw_input_with(click_at(target.center())), &mut panel);

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::SelectModel(id) => assert_eq!(id, "glm-5.3"),
            other => panic!("expected SelectModel, got {other:?}"),
        }
        assert_eq!(panel.model(), Some(&"glm-5.3".to_owned()));
        assert!(!panel.is_picker_open(), "selection closes the picker");
    }

    #[test]
    fn model_picker_footer_refresh_passes_through() {
        let mut panel = ChatPanel::new();
        panel.set_models(vec![model("glm-5.3")]);
        panel.open_model_picker();

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        // The footer sits below the rows: click the picker column in the
        // 24px band under the last row.
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let last_row = outcome.model_row_rects.last().expect("row").1;
        let footer_click = pos2(
            last_row.center().x,
            last_row.bottom() + MODEL_FOOTER_HEIGHT / 2.0,
        );
        let outcome = frame(&ctx, raw_input_with(click_at(footer_click)), &mut panel);

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        assert!(matches!(outcome.events[0], ChatPanelEvent::RefreshModels));
    }

    #[test]
    fn empty_catalog_paints_the_placeholder() {
        let mut panel = ChatPanel::new();
        panel.open_model_picker();
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(outcome.model_row_rects.is_empty());
        assert!(panel.is_picker_open());
    }

    // -- persistence ------------------------------------------------------------------

    #[test]
    fn snapshot_turns_truncates_to_the_last_twenty() {
        let mut panel = ChatPanel::new();
        for i in 0..30 {
            panel.push_user(&format!("user {i}"));
            panel.push_user(&format!("assistant {i}"));
        }
        let turns = panel.snapshot_turns();
        assert_eq!(turns.len(), MAX_PERSISTED_TURNS);
        assert_eq!(turns[0], ChatTurn::user("user 20"));
        assert_eq!(turns[19], ChatTurn::user("assistant 29"));
    }

    #[test]
    fn restore_turns_rebuilds_the_transcript() {
        let mut panel = ChatPanel::new();
        panel.restore_turns(vec![
            ChatTurn::user("hello"),
            ChatTurn::assistant("```rust\nfn main() {}\n```\n"),
        ]);
        assert_eq!(panel.turn_count(), 2);
        assert!(!panel.is_streaming());
        assert_eq!(
            panel.conversation(),
            vec![
                ChatMessage::user("hello"),
                ChatMessage::assistant("```rust\nfn main() {}\n```\n"),
            ]
        );

        // The restored transcript renders headlessly (code block
        // included).
        let outcome = {
            let mut outcome = ChatPanelOutcome::default();
            egui::__run_test_ui(|ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                outcome = panel.show(ui);
            });
            outcome
        };
        assert_eq!(outcome.bubbles_painted, 2);
        assert_eq!(outcome.code_blocks_painted, 1);
    }

    #[test]
    fn snapshot_and_restore_round_trip() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hello");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "world");
        panel.complete_stream(id);
        let snapshot = panel.snapshot_turns();

        let mut restored = ChatPanel::new();
        restored.restore_turns(snapshot);
        assert_eq!(restored.turn_count(), 2);
        assert_eq!(restored.conversation(), panel.conversation());
    }

    #[test]
    fn the_streaming_bubble_is_not_persisted() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hello");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "partial");
        let turns = panel.snapshot_turns();
        assert_eq!(turns.len(), 1, "only the user bubble is stable mid-stream");
        assert_eq!(turns[0], ChatTurn::user("hello"));
    }

    // -- parse_segments -----------------------------------------------------------------

    #[test]
    fn parse_segments_splits_prose_and_code() {
        let text = "before\n```rust\nfn main() {}\n```\nafter";
        assert_eq!(
            parse_segments(text),
            vec![
                Segment::Prose("before\n"),
                Segment::Code {
                    lang: Some("rust".to_owned()),
                    body: "fn main() {}\n".to_owned(),
                    complete: true
                },
                Segment::Prose("after"),
            ]
        );
    }

    #[test]
    fn parse_segments_tolerates_a_space_after_the_fence() {
        // The phase brief writes fences as ``` rust; the language tag is
        // trimmed.
        let text = "``` rust\nlet x = 1;\n```";
        assert_eq!(
            parse_segments(text),
            vec![Segment::Code {
                lang: Some("rust".to_owned()),
                body: "let x = 1;\n".to_owned(),
                complete: true
            }]
        );
    }

    #[test]
    fn parse_segments_unclosed_fence_is_incomplete() {
        let text = "answer:\n```\nfn partial(";
        assert_eq!(
            parse_segments(text),
            vec![
                Segment::Prose("answer:\n"),
                Segment::Code {
                    lang: None,
                    body: "fn partial(\n".to_owned(),
                    complete: false
                },
            ]
        );
    }

    #[test]
    fn parse_segments_multiple_blocks_and_plain_text() {
        assert_eq!(
            parse_segments("just prose"),
            vec![Segment::Prose("just prose")]
        );
        let text = "```a\n1\n```\nmiddle\n```b\n2\n```";
        assert_eq!(
            parse_segments(text),
            vec![
                Segment::Code {
                    lang: Some("a".to_owned()),
                    body: "1\n".to_owned(),
                    complete: true
                },
                Segment::Prose("middle\n"),
                Segment::Code {
                    lang: Some("b".to_owned()),
                    body: "2\n".to_owned(),
                    complete: true
                },
            ]
        );
    }

    #[test]
    fn parse_segments_skips_whitespace_only_prose() {
        let text = "```rust\nx\n```";
        assert_eq!(
            parse_segments(text),
            vec![Segment::Code {
                lang: Some("rust".to_owned()),
                body: "x\n".to_owned(),
                complete: true
            }]
        );
    }

    // -- misc ---------------------------------------------------------------------------

    #[test]
    fn chat_ids_increase() {
        let a = ChatId::next();
        let b = ChatId::next();
        assert_ne!(a, b);
        assert!(b.as_u64() > a.as_u64());
    }

    #[test]
    fn wave_keyframe_stays_in_bounds() {
        for phase in [0.0, 0.15, 0.3, 0.45, 0.6, 0.9, 1.0] {
            let (dy, opacity) = wave_keyframe(phase);
            assert!((-3.0..=0.0).contains(&dy));
            assert!((0.5..=1.0).contains(&opacity));
        }
    }

    // -- Agent Mode (Phase 5d) ------------------------------------------------

    /// Submit with agent mode on (mirrors the Enter path's agent branch).
    fn send_agent(panel: &mut ChatPanel) -> Option<ChatId> {
        let mut events = Vec::new();
        panel.submit(&mut events);
        match events.pop() {
            Some(ChatPanelEvent::AgentSend { id, .. }) => Some(id),
            _ => None,
        }
    }

    fn tool_call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: format!("call_{name}"),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn tool_result(call: &ToolCall, ok: bool, content: &str) -> ToolResult {
        ToolResult {
            id: call.id.clone(),
            ok,
            content: content.to_owned(),
        }
    }

    fn diff_approval() -> ApprovalCard {
        ApprovalCard::Diff {
            path: "src/main.rs".to_owned(),
            diff: "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n".to_owned(),
        }
    }

    fn summary() -> AgentSummary {
        AgentSummary {
            iterations: 2,
            tools_used: 1,
            approved: 1,
            denied: 0,
            tokens: 1240,
            cost_usd: 0.004,
            completed: true,
        }
    }

    /// The timeline row kinds, in order (the interleave assertion).
    fn entry_kinds(panel: &ChatPanel) -> Vec<&'static str> {
        panel
            .entries
            .iter()
            .map(|entry| match entry {
                TimelineEntry::Bubble(_) => "bubble",
                TimelineEntry::Card(_) => "card",
                TimelineEntry::Summary(_) => "summary",
            })
            .collect()
    }

    #[test]
    fn agent_pill_toggles_agent_mode() {
        let mut panel = ChatPanel::new();
        assert!(!panel.is_agent_mode());
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let pill = outcome.agent_pill_rect;
        assert!(pill.width() > 0.0, "the pill paints in the header");
        frame(&ctx, raw_input_with(click_at(pill.center())), &mut panel);
        assert!(panel.is_agent_mode(), "the pill click toggles on");
        frame(&ctx, raw_input_with(click_at(pill.center())), &mut panel);
        assert!(!panel.is_agent_mode(), "and back off");
    }

    #[test]
    fn agent_submit_emits_agent_send_and_paints_the_status_line() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("read src/main.rs");

        let mut events = Vec::new();
        panel.submit(&mut events);
        assert_eq!(events.len(), 1, "events: {events:?}");
        match &events[0] {
            ChatPanelEvent::AgentSend { id, text } => {
                assert_eq!(text, "read src/main.rs");
                assert_eq!(panel.in_flight(), Some(*id));
            }
            other => panic!("expected AgentSend, got {other:?}"),
        }
        assert!(panel.is_streaming());

        // While the turn runs the status line + Stop paint under the
        // last entry (no cards yet: the `thinking` phase).
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(outcome.status_line_rect.is_some());
        assert!(outcome.stop_button_rect.is_some());
        assert_eq!(outcome.cards_painted, 0);
    }

    #[test]
    fn plain_submit_still_emits_send_with_agent_mode_off() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let mut events = Vec::new();
        panel.submit(&mut events);
        assert!(matches!(events[0], ChatPanelEvent::Send { .. }));
        // No status line for a plain chat.
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(outcome.status_line_rect.is_none());
    }

    #[test]
    fn stale_agent_events_are_dropped() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("hi");
        let id = send_agent(&mut panel).expect("agent send");

        let stale = ChatId::next();
        panel.tool_call_started(stale, tool_call("read_file", r#"{"path":"a.rs"}"#));
        panel.approval_needed(stale, ApprovalId::from_u128(1), diff_approval());
        panel.tool_result(stale, tool_result(&tool_call("read_file", ""), true, "x"));
        panel.agent_turn_done(stale, summary());
        assert!(
            panel.is_streaming(),
            "a stale turn-done must not end the turn"
        );
        assert_eq!(panel.turn_count(), 1, "only the user bubble");

        // The real turn's events land.
        let call = tool_call("read_file", r#"{"path":"a.rs"}"#);
        panel.tool_call_started(id, call.clone());
        assert_eq!(panel.turn_count(), 2);
        panel.tool_result(id, tool_result(&call, true, "x"));
        panel.agent_turn_done(id, summary());
        assert!(!panel.is_streaming());
        assert_eq!(panel.turn_count(), 3, "user bubble + card + summary");
    }

    #[test]
    fn cards_interleave_in_arrival_order_and_finalize_streaming_bubbles() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("fix it");
        let id = send_agent(&mut panel).expect("agent send");

        panel.push_assistant_delta(id, "reading the file first.");
        let call = tool_call("read_file", r#"{"path":"src/main.rs"}"#);
        panel.tool_call_started(id, call.clone());

        // The card interrupted the streaming bubble: it finalized, so its
        // text is context for the next request already (the round-1
        // context-loss fix).
        assert!(panel
            .conversation()
            .iter()
            .any(|m| m.role == Role::Assistant && m.content == "reading the file first."));

        panel.tool_result(id, tool_result(&call, true, "contents"));
        panel.push_assistant_delta(id, "now the answer.");
        assert_eq!(
            entry_kinds(&panel),
            ["bubble", "bubble", "card", "bubble"],
            "arrival order interleaves cards between bubbles"
        );

        panel.agent_turn_done(id, summary());
        assert_eq!(
            entry_kinds(&panel),
            ["bubble", "bubble", "card", "bubble", "summary"]
        );
        // No bubble is left streaming; BOTH rounds' texts are context.
        assert!(panel
            .entries
            .iter()
            .all(|entry| !matches!(entry, TimelineEntry::Bubble(m) if m.streaming)));
        let texts: Vec<String> = panel
            .conversation()
            .iter()
            .map(|m| m.content.clone())
            .collect();
        assert!(texts.iter().any(|t| t == "reading the file first."));
        assert!(texts.iter().any(|t| t == "now the answer."));
        assert!(!panel.is_streaming());
    }

    #[test]
    fn cards_and_summaries_never_enter_context_snapshots_or_transcripts() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("fix it");
        let id = send_agent(&mut panel).expect("agent send");
        panel.push_assistant_delta(id, "the answer.");
        let call = tool_call("read_file", r#"{"path":"src/main.rs"}"#);
        panel.tool_call_started(id, call.clone());
        panel.tool_result(id, tool_result(&call, true, "contents"));
        panel.agent_turn_done(id, summary());

        // Bubbles only: user + assistant; the card and the summary are
        // neither context, nor persisted, nor transcript text.
        assert_eq!(
            panel.conversation(),
            vec![
                ChatMessage::user("fix it"),
                ChatMessage::assistant("the answer."),
            ]
        );
        assert_eq!(panel.snapshot_turns().len(), 2);
        let transcript = panel.transcript();
        assert!(transcript.contains("You: fix it"));
        assert!(transcript.contains("Selora: the answer."));
        assert!(
            !transcript.contains("call_read_file"),
            "cards never enter the transcript: {transcript}"
        );
    }

    #[test]
    fn a_deny_click_on_the_diff_card_emits_the_agent_decision() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("edit it");
        let id = send_agent(&mut panel).expect("agent send");
        panel.tool_call_started(
            id,
            tool_call(
                "edit_file",
                r#"{"path":"src/main.rs","old_str":"a","new_str":"b"}"#,
            ),
        );
        let approval_id = ApprovalId::from_u128(42);
        panel.approval_needed(id, approval_id, diff_approval());

        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert_eq!(outcome.cards_painted, 1);
        let deny_rect = outcome
            .decision_button_rects
            .iter()
            .find(|(label, _)| *label == "Deny")
            .map(|(_, rect)| *rect)
            .expect("Deny button paints");
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(deny_rect.center())),
            &mut panel,
        );
        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::AgentDecision {
                id: event_id,
                approval_id: event_approval,
                decision,
            } => {
                assert_eq!(*event_id, id);
                assert_eq!(*event_approval, approval_id);
                assert_eq!(*decision, AgentDecision::Deny);
            }
            other => panic!("expected AgentDecision, got {other:?}"),
        }

        // The latch: no buttons next frame, no double-fire.
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(deny_rect.center())),
            &mut panel,
        );
        assert!(outcome.decision_button_rects.is_empty());
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);
    }

    #[test]
    fn the_status_line_stop_names_the_pending_card_or_nil() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("run it");
        let id = send_agent(&mut panel).expect("agent send");

        let ctx = egui::Context::default();
        // No card parked yet: Stop names Uuid::nil().
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let stop = outcome.stop_button_rect.expect("Stop paints").center();
        let outcome = frame(&ctx, raw_input_with(click_at(stop)), &mut panel);
        match &outcome.events[0] {
            ChatPanelEvent::AgentDecision {
                approval_id,
                decision,
                ..
            } => {
                assert_eq!(*approval_id, ApprovalId::nil());
                assert_eq!(*decision, AgentDecision::Stop);
            }
            other => panic!("expected AgentDecision, got {other:?}"),
        }

        // With a card parked, Stop names its approval id.
        panel.tool_call_started(id, tool_call("bash", r#"{"command":"cargo test"}"#));
        let approval_id = ApprovalId::from_u128(7);
        panel.approval_needed(
            id,
            approval_id,
            ApprovalCard::Command {
                command: "cargo test".to_owned(),
            },
        );
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let stop = outcome.stop_button_rect.expect("Stop paints").center();
        let outcome = frame(&ctx, raw_input_with(click_at(stop)), &mut panel);
        match &outcome.events[0] {
            ChatPanelEvent::AgentDecision {
                approval_id: got,
                decision,
                ..
            } => {
                assert_eq!(*got, approval_id);
                assert_eq!(*decision, AgentDecision::Stop);
            }
            other => panic!("expected AgentDecision, got {other:?}"),
        }
    }

    #[test]
    fn clear_during_an_agent_turn_emits_stop_before_clear() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("work");
        send_agent(&mut panel).expect("agent send");
        panel.menu_open = true;

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        // Second warm-up frame: the popup sizes on its first open frame.
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let clear_rect = outcome
            .menu_item_rects
            .iter()
            .find(|(label, _)| *label == "Clear conversation")
            .map(|(_, rect)| *rect)
            .expect("clear menu item");
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(clear_rect.center())),
            &mut panel,
        );

        assert_eq!(outcome.events.len(), 2, "events: {:?}", outcome.events);
        assert!(
            matches!(
                &outcome.events[0],
                ChatPanelEvent::AgentDecision {
                    decision: AgentDecision::Stop,
                    approval_id,
                    ..
                } if approval_id.is_nil()
            ),
            "Stop (nil id) lands BEFORE Clear: {:?}",
            outcome.events
        );
        assert!(matches!(outcome.events[1], ChatPanelEvent::Clear));
        assert_eq!(panel.turn_count(), 0);
        assert!(!panel.is_streaming());
    }

    #[test]
    fn approve_all_appears_from_the_turns_second_pending_card() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("edits");
        let id = send_agent(&mut panel).expect("agent send");

        // First pending card: no "Approve all".
        let call1 = tool_call(
            "edit_file",
            r#"{"path":"a.rs","old_str":"a","new_str":"b"}"#,
        );
        panel.tool_call_started(id, call1.clone());
        panel.approval_needed(id, ApprovalId::from_u128(1), diff_approval());
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(!outcome
            .decision_button_rects
            .iter()
            .any(|(label, _)| *label == "Approve all"));

        // It resolves (denied → result), then the turn's second pending
        // card shows "Approve all" (§4.2:268).
        panel.tool_result(id, tool_result(&call1, false, "user denied this action"));
        let call2 = tool_call(
            "edit_file",
            r#"{"path":"b.rs","old_str":"c","new_str":"d"}"#,
        );
        panel.tool_call_started(id, call2);
        panel.approval_needed(id, ApprovalId::from_u128(2), diff_approval());
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert_eq!(
            outcome
                .decision_button_rects
                .iter()
                .filter(|(label, _)| *label == "Approve all")
                .count(),
            1,
            "exactly one Approve all: {:?}",
            outcome.decision_button_rects
        );
    }

    #[test]
    fn the_summary_footer_paints_and_the_status_line_dies_with_the_turn() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("fix it");
        let id = send_agent(&mut panel).expect("agent send");
        let call = tool_call("read_file", r#"{"path":"src/main.rs"}"#);
        panel.tool_call_started(id, call.clone());
        panel.tool_result(id, tool_result(&call, true, "contents"));

        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert_eq!(outcome.cards_painted, 1);
        assert!(outcome.status_line_rect.is_some(), "mid-turn status line");
        assert!(outcome.summary_footer_rect.is_none());

        panel.agent_turn_done(id, summary());
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(outcome.summary_footer_rect.is_some());
        assert!(
            outcome.status_line_rect.is_none(),
            "the status line is gone once the turn ends"
        );
    }

    #[test]
    fn a_card_parked_at_turn_end_latches_its_buttons_off() {
        // Stop while parked: the loop breaks without a ToolResult for
        // the parked card — its buttons must die with the turn.
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("edit it");
        let id = send_agent(&mut panel).expect("agent send");
        panel.tool_call_started(
            id,
            tool_call(
                "edit_file",
                r#"{"path":"a.rs","old_str":"a","new_str":"b"}"#,
            ),
        );
        panel.approval_needed(id, ApprovalId::from_u128(9), diff_approval());
        panel.agent_turn_done(id, summary());

        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(
            outcome.decision_button_rects.is_empty(),
            "a dead turn's card offers no buttons"
        );
        // And a click where Approve was is a no-op (no in-flight id).
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn a_running_bash_card_freezes_when_its_turn_ends() {
        // Approved and mid-run when the turn ends (Stop/cap): the loop
        // breaks without a ToolResult for the killed command — the card's
        // Stop must die with the turn, not spin forever.
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("run it");
        let id = send_agent(&mut panel).expect("agent send");
        panel.tool_call_started(id, tool_call("bash", r#"{"command":"cargo test"}"#));
        panel.approval_needed(
            id,
            ApprovalId::from_u128(7),
            ApprovalCard::Command {
                command: "cargo test".to_owned(),
            },
        );

        // Approve via the card's button.
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let approve = outcome
            .decision_button_rects
            .iter()
            .find(|(label, _)| *label == "Approve")
            .map(|(_, rect)| *rect)
            .expect("Approve paints");
        let outcome = frame(&ctx, raw_input_with(click_at(approve.center())), &mut panel);
        assert!(matches!(
            outcome.events[0],
            ChatPanelEvent::AgentDecision {
                decision: AgentDecision::Approve,
                ..
            }
        ));

        // Running: the card's Stop replaces Approve.
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(
            outcome
                .decision_button_rects
                .iter()
                .any(|(label, _)| *label == "Stop"),
            "running: Stop paints"
        );

        // The turn ends mid-run: the card freezes — no Stop, no buttons.
        panel.agent_turn_done(id, summary());
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        assert!(
            outcome.decision_button_rects.is_empty(),
            "a dead turn's running card offers no buttons"
        );
    }
}
