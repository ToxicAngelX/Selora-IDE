//! Chat sidebar content (UI_SPEC.md §4) — Phase 4c.
//!
//! The shell owns the right `egui::SidePanel` itself (360px default,
//! 240px min, 600px max, 1px hairline left border, `Ctrl+L` visibility
//! per UI_SPEC.md §2 and §15 — persisted in workspace state by the
//! shell); this widget paints the panel's content:
//!
//! - the 40px header: the 20px presence orb (the status line's mapping,
//!   so the two never disagree), the session switcher, and the `⋯`
//!   overflow menu (clear conversation, copy transcript, switch model),
//! - the conversation scroll: user bubbles (`--ink-mid`, right-aligned),
//!   assistant bubbles (`--ink-low`, left-aligned) with fenced code
//!   blocks (`--ink-canvas` + 1px `--ink-hi` border, JetBrains Mono, a
//!   hover "Copy" button top-right), and the streaming bubble with a
//!   comet fade-in on the first delta and the `wave` thinking indicator,
//! - the hero composer (REVAMP_SPEC §2): the `--ink-low` prompt card
//!   (the frameless multiline input — "Ask anything…", Enter to send,
//!   Shift+Enter for a newline — and the `⏎` send / `■` stop button)
//!   with the five focus glow rings, the border beam riding the card's
//!   perimeter while a request is in flight (§8), the streaming glow +
//!   ready shimmer on its bottom edge, and a footer row carrying the
//!   model chip, the `agent` pill, and the key hint when it fits whole
//!   (§2.1).
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
//! The model picker is Phase 4b's canonical widget
//! ([`crate::picker::model_picker`]) — Phase R0 wired it into the header
//! (the parallel-build inline dropdown was deleted), gaining its filter
//! input and keyboard navigation. Persistence of the last 20 turns and of
//! panel visibility lives in `selora-app` (`workspace.json`); this module
//! exposes the data via [`ChatPanel::snapshot_turns`] and
//! [`ChatPanel::restore_turns`].
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

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui, Vec2};

use selora_ai::{
    AgentDecision, AgentSummary, ApprovalCard, ApprovalId, ChatMessage, Model, ModelId, Role,
    ToolCall, ToolResult,
};

use crate::picker::model_picker::{self, ModelPicker, ModelPickerEvent};
use crate::presence::beam::BorderBeam;
use crate::presence::liquid::LiquidPill;
use crate::presence::orb::{OrbState, ThinkingOrb, ORB_AVATAR, ORB_INLINE};
use crate::presence::stream_glow::StreamGlow;
use crate::presence::{bloom_progress, clock_s, request_anim_frame, with_alpha, AgentAvatar};
use crate::sidebar::icons;
use crate::theme::{color, font, motion, radius, space, type_scale};

/// Agent Mode cards (UI_SPEC.md §4.2) — Phase 5d's file.
pub mod agent_cards;
/// Markdown-lite parsing for assistant messages (Phase R2).
pub mod markdown;

/// Header bar height (UI_SPEC.md §4: "Header bar (40px tall)").
pub const HEADER_HEIGHT: f32 = 40.0;
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
/// The composer input's placeholder (UI_SPEC.md §4).
const COMPOSER_HINT: &str = "Ask anything…";
/// The keyboard hint under the composer input (UI_SPEC.md §4 wireframe).
const COMPOSER_KEY_HINT: &str = "Enter to send, Shift+Enter for newline";
/// The send button's glyph (UI_SPEC.md §4: "⏎ glyph").
const SEND_GLYPH: &str = "⏎";
/// The composer's row cap (Phase R1): the bar grows to this many rows,
/// then the edit scrolls internally.
const COMPOSER_MAX_ROWS: usize = 6;
/// How many submitted prompts the ↑/↓ recall keeps (Phase R1).
const HISTORY_LEN: usize = 10;
/// The Stop button's square edge (Phase R1 — painted, not a glyph, so
/// no font carries the requirement).
const STOP_SQUARE: f32 = 8.0;
/// The code block's header row (Phase R2): the language label left, the
/// Copy affordance right.
const CODE_HEADER_HEIGHT: f32 = 22.0;
/// The list items' inter-item gap (Phase R2).
const LIST_ITEM_GAP: f32 = 4.0;
/// The empty transcript's suggested prompts (Phase R2) — a click prefills
/// the composer.
const SUGGESTIONS: [&str; 3] = [
    "Explain this workspace's structure",
    "Find the TODOs and unfinished work",
    "Review the active file for bugs",
];
/// Send button size (width × height; a `--radius-1` chip).
const SEND_BUTTON_SIZE: Vec2 = Vec2::new(28.0, 20.0);
/// The overflow menu's glyph (UI_SPEC.md §4: "⋯ overflow menu").
const OVERFLOW_GLYPH: &str = "⋯";
/// The thinking indicator appears only after this long without a
/// delta (UI_SPEC.md §4: "longer than 800ms with no chunk").
const THINKING_AFTER: Duration = Duration::from_millis(800);
/// The streaming bubble's fade-in on the first delta (the `comet`
/// glyph's fade half; DESIGN_TOKENS.md §7.2 `--dur-base`).
const FADE_DURATION: Duration = Duration::from_millis(240);
/// The composer's stable widget id.
const COMPOSER_ID: &str = "selora_chat_composer";
/// The focus glow rings' stepped alphas (REVAMP_SPEC §2.2 — painted
/// alpha on `--fg-faint`, the `LiquidPill` layered-alpha precedent: no
/// blur, no new hue; §11 bend #1).
const GLOW_RING_ALPHAS: [f32; 5] = [0.18, 0.12, 0.08, 0.05, 0.03];
/// Ring i's outset from the card edge: `GLOW_RING_STEP · (i+1)` (§2.2).
/// A centered 1px stroke reaches outset + 0.5 = 10.5px at most — inside
/// the 12px card margins, so the rings never meet the panel's clip.
const GLOW_RING_STEP: f32 = 2.0;
/// The rings' entrance/exit fade, seconds (`--dur-quick`, §2.2's `fade`
/// glyph — snapped under reduce-motion per §11 bend #5).
const RING_FADE_S: f32 = motion::DUR_QUICK_MS as f32 / 1000.0;
/// The welcome chips' stagger step and per-chip bloom live in
/// `presence::bloom_progress` (`--dur-instant` / `--dur-base`, §1/§4).
/// The chips' bloom start scale (§4: 0.96 → 1.0 on `--ease-precise`).
const CHIP_BLOOM_FROM: f32 = 0.96;
/// The agent pill's dot diameter — mirrors `liquid.rs`'s file-private
/// `DOT`, so the footer's fit-or-hide math (§2.1) knows the pill's
/// constant width before it paints. `footer_layout_and_hint_fit_or_hide`
/// pins the two formulas together.
const AGENT_PILL_DOT: f32 = 6.0;

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
// Sessions (Phase R3): the persisted conversation units
// -----------------------------------------------------------------------------

/// How many sessions persist (oldest evicted).
pub const MAX_SESSIONS: usize = 20;
/// How many entries one persisted session keeps (newest kept).
pub const MAX_SESSION_ENTRIES: usize = 100;
/// The auto-title's length cap.
const TITLE_MAX_CHARS: usize = 32;
/// The sessions dropdown's width.
const SESSIONS_MENU_WIDTH: f32 = 280.0;
/// One row of the sessions dropdown.
const SESSION_ROW_HEIGHT: f32 = 30.0;
/// Rows visible before the sessions list scrolls.
const SESSION_MAX_VISIBLE_ROWS: usize = 8;

/// One persisted conversation (Phase R3): the serde mirror of a
/// timeline. Paint state (layout caches, animation clocks, avatars) is
/// rebuilt on restore — only the durable content persists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: uuid::Uuid,
    /// The auto-title (the first user message, truncated) — `""` while
    /// the session has no user turn yet.
    pub title: String,
    /// Unix epoch seconds.
    pub created_at: u64,
    pub entries: Vec<SessionEntry>,
}

/// One persisted timeline entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SessionEntry {
    Bubble {
        role: Role,
        content: String,
        error: Option<String>,
    },
    Card(SessionCard),
    Summary(AgentSummary),
}

/// The persisted mirror of [`agent_cards::ToolCard`] — the call, its
/// approval state, its result, and the latch/expanded flags. Restoring
/// rebuilds the card's animation state fresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionCard {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub approval: Option<(ApprovalId, ApprovalCard, u32)>,
    pub result: Option<ToolResult>,
    pub decided: Option<AgentDecision>,
    pub expanded: bool,
}

/// Why a session action refused: a turn is running (Phase R3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionBusy;

/// The auto-title of a session: its first user message, truncated.
fn truncate_title(text: &str) -> String {
    let mut title: String = text.trim().chars().take(TITLE_MAX_CHARS).collect();
    if text.trim().chars().count() > TITLE_MAX_CHARS {
        title.push('…');
    }
    title
}

/// `now` in Unix epoch seconds.
fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// `now` / `4m` / `2h` / `3d` — the session rows' relative age.
fn relative_time(epoch: u64) -> String {
    let now = epoch_now();
    let delta = now.saturating_sub(epoch);
    match delta {
        0..=59 => "now".to_owned(),
        60..=3599 => format!("{}m", delta / 60),
        3600..=86399 => format!("{}h", delta / 3600),
        _ => format!("{}d", delta / 86_400),
    }
}

/// The persisted form of one timeline entry.
fn session_entry_of(entry: &TimelineEntry) -> SessionEntry {
    match entry {
        TimelineEntry::Bubble(message) => SessionEntry::Bubble {
            role: message.role,
            content: message.content.clone(),
            error: message.error.clone(),
        },
        TimelineEntry::Card(card) => SessionEntry::Card(SessionCard {
            id: card.call.id.clone(),
            name: card.call.name.clone(),
            arguments: card.call.arguments.clone(),
            approval: card.approval.clone(),
            result: card.result.clone(),
            decided: card.decided,
            expanded: card.expanded,
        }),
        TimelineEntry::Summary(summary) => SessionEntry::Summary(summary.clone()),
    }
}

/// The live form of one persisted entry.
fn timeline_entry_of(entry: SessionEntry) -> TimelineEntry {
    match entry {
        SessionEntry::Bubble {
            role,
            content,
            error,
        } => TimelineEntry::Bubble(Message {
            role,
            content,
            streaming: false,
            stream_started: None,
            last_delta: None,
            error,
            layout: None,
            finished_at: None,
        }),
        SessionEntry::Card(card) => {
            let mut restored = agent_cards::ToolCard::new(ToolCall {
                id: card.id,
                name: card.name,
                arguments: card.arguments,
            });
            restored.approval = card.approval;
            restored.result = card.result;
            restored.decided = card.decided;
            restored.expanded = card.expanded;
            TimelineEntry::Card(restored)
        }
        SessionEntry::Summary(summary) => TimelineEntry::Summary(summary),
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
    /// The composer's Stop button was clicked on a running plain (non-
    /// agent) stream (Phase R1). The panel has already finalized the
    /// streaming bubble; the shell cancels the request itself
    /// (`Effect::SeloraChatAbort`) — the stream's late deltas are
    /// stale-dropped.
    StopStream { id: ChatId },
    /// A session action (switch / new) was refused while a turn runs
    /// (Phase R3) — the shell toasts the reason.
    SessionBlocked,
    /// A card body's "open ↗" link was clicked (Phase R4): open the file
    /// at the 1-based line. The path is workspace-relative; the shell
    /// resolves it against the root.
    OpenFileAt { path: PathBuf, line: usize },
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
    /// Rect of the composer's send button (inside the card, §2.1).
    pub send_button_rect: Rect,
    /// The composer card's rect (REVAMP_SPEC §2 — the glow rings expand
    /// from it, the footer lives inside it).
    pub composer_card_rect: Rect,
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
    /// The agent-mode pill's rect (the composer footer's left cluster,
    /// REVAMP_SPEC §2.1).
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
            composer_card_rect: Rect::ZERO,
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
    composer_card_rect: Rect,
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
    /// A failed stream's Retry was clicked (applied by the paint loop:
    /// `ChatPanel::retry_failed`).
    retry_clicked: bool,
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
            composer_card_rect: Rect::ZERO,
            model_row_rects: Vec::new(),
            menu_item_rects: Vec::new(),
            cards_painted: 0,
            decision_button_rects: Vec::new(),
            status_line_rect: None,
            stop_button_rect: None,
            agent_pill_rect: Rect::ZERO,
            summary_footer_rect: None,
            copies: Vec::new(),
            retry_clicked: false,
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
    /// The rendered parts (Phase R2): rebuilt when the content or the
    /// wrap width changes — never per frame (the streaming bubble
    /// re-lays out per delta; settled bubbles once).
    layout: Option<CachedLayout>,
    /// When the stream ended (the footer's elapsed metadata freezes at
    /// it). `None` while streaming or for restored turns.
    finished_at: Option<Instant>,
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
            layout: None,
            finished_at: None,
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

/// One laid-out piece of an assistant message (Phase R2): the markdown
/// blocks' rendered form, produced by [`build_parts`] and cached on
/// [`Message`] (rebuilt only when the content or the wrap width moves).
#[derive(Clone, Debug)]
enum Part {
    /// A prose-ish block (paragraph, heading, list item, quote): the
    /// styled layout-job galley plus the char ranges of its links (the
    /// click targets).
    Prose {
        galley: Arc<egui::Galley>,
        links: Vec<LinkSpan>,
        /// A quote block paints a left rule and indents.
        quote: bool,
    },
    /// A `---` hairline.
    Rule,
    /// A fenced code block, syntax-highlighted: its source (for Copy),
    /// the highlighted galley, the block height (header + body +
    /// padding), whether the fence closed (Copy only appears on closed
    /// blocks), and the language tag for the header label.
    Code {
        body: String,
        galley: Arc<egui::Galley>,
        block_height: f32,
        complete: bool,
        lang: Option<String>,
    },
}

/// One clickable link inside a prose galley: its char range in the
/// galley's job and the target (a click opens the browser).
#[derive(Clone, Debug)]
struct LinkSpan {
    /// Char range within the layout job's text.
    range: std::ops::Range<usize>,
    url: String,
}

/// The cached rendered form of an assistant message (Phase R2): the
/// parts and their total height, valid while the content length and the
/// wrap width hold. The streaming fade's first 240ms rebuilds per frame
/// (the fade bakes into the section colors).
#[derive(Clone, Debug)]
struct CachedLayout {
    content_len: usize,
    wrap: f32,
    parts: Vec<Part>,
    height: f32,
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
    /// settings and re-set when the picker selects). The picker mirrors
    /// this for its active row.
    model: Option<ModelId>,
    /// The canonical model picker (Phase R0): owns the model list, the
    /// active row, the dropdown's open state and filter. This panel
    /// forwards `set_models`/`set_model` into it and maps its events to
    /// [`ChatPanelEvent::SelectModel`] / [`RefreshModels`].
    picker: ModelPicker,
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
    /// Phase 6h: the composer's streaming-activity glow (the voice-glow
    /// port — deltas, not a microphone).
    stream_glow: StreamGlow,
    /// Phase 6h: the header pill's gooey melt (animation state only; the
    /// source of truth stays `agent_mode`).
    agent_pill: LiquidPill,
    /// Phase 6h: the status line's agent identity glyph.
    status_avatar: AgentAvatar,
    /// Phase 6h: the gateway's reachability (the avatars sleep while
    /// disconnected). Threaded by the shell's `AppMessage::SeloraStatus`
    /// reduction.
    gateway_connected: bool,
    /// Submitted prompts for the composer's ↑/↓ recall (Phase R1),
    /// oldest first, capped at [`HISTORY_LEN`].
    history: Vec<String>,
    /// Where the recall is parked: `None` = the live buffer, `Some(i)` =
    /// `history[i]`.
    history_index: Option<usize>,
    /// The live buffer stashed when the recall first walks back (restored
    /// when it walks forward off the end).
    history_draft: Option<String>,
    /// Stored sessions (Phase R3), oldest first — the live timeline is
    /// separate; switching stashes it here and loads the target.
    sessions: Vec<ChatSession>,
    /// The live session's identity (its id survives stashing; a new
    /// session gets a fresh one).
    live_id: uuid::Uuid,
    /// The live session's auto-title ("" until its first user message).
    live_title: String,
    /// The live session's creation time (epoch seconds).
    live_created: u64,
    /// Whether the sessions dropdown is open.
    sessions_open: bool,
    /// The welcome entrance's frame-clock anchor (REVAMP_SPEC §4): armed
    /// while the transcript is empty, cleared the moment it isn't — a
    /// Clear / new_session / restored empty session re-runs the bloom.
    welcome_anchor: Option<f64>,
    /// Whether the composer's focus glow rings are on (§2.2);
    /// `ring_anim` carries the entrance/exit clock.
    rings_on: bool,
    /// The rings' fade clock: (frame-clock start, appearing) — the
    /// `LiquidPill::anim` pattern. `None` once settled, and always under
    /// reduce-motion (the rings snap, §11 bend #5).
    ring_anim: Option<(f64, bool)>,
    /// The border beam (REVAMP_SPEC §8): rides the composer card's
    /// perimeter while a request is in flight, painted only then
    /// (§8.6) — inactive it freezes at its held phase, unseen.
    beam: BorderBeam,
}

impl Default for ChatPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatPanel {
    /// An empty panel with no transcript and no model.
    pub fn new() -> Self {
        // REVAMP_SPEC §2.1: the picker's closed trigger lives on the
        // composer card — its header tuning would invert there.
        let mut picker = ModelPicker::new(None);
        picker.set_on_card(true);
        // §2.2: the glow band's end caps match the card's `--radius-2`.
        let mut stream_glow = StreamGlow::new();
        stream_glow.set_cap_radius(radius::R2);
        Self {
            entries: Vec::new(),
            composer: String::new(),
            want_focus: false,
            in_flight: None,
            model: None,
            picker,
            menu_open: false,
            agent_mode: false,
            reduce_motion: false,
            agent_in_flight: false,
            turn_approvals: 0,
            turn_cards_start: 0,
            stream_glow,
            agent_pill: LiquidPill::default(),
            status_avatar: AgentAvatar::new(0.0),
            gateway_connected: true,
            history: Vec::new(),
            history_index: None,
            history_draft: None,
            sessions: Vec::new(),
            live_id: uuid::Uuid::new_v4(),
            live_title: String::new(),
            live_created: epoch_now(),
            sessions_open: false,
            welcome_anchor: None,
            rings_on: false,
            ring_anim: None,
            beam: BorderBeam::new(),
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
        let model = model.into();
        self.picker.set_active_model(Some(&model));
        self.model = Some(model);
    }

    /// The model catalog (fed from `AppMessage::ModelsLoaded`; the
    /// picker renders it).
    pub fn models(&self) -> &[Model] {
        self.picker.models()
    }

    /// Install the model catalog.
    pub fn set_models(&mut self, models: Vec<Model>) {
        self.picker.set_models(&models);
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

    /// Thread `ui.reduce_motion` (DESIGN_TOKENS.md §7.4) into the cards
    /// and the presence widgets: pulse/spin/orbit become static dots and
    /// the border beam freezes (REVAMP_SPEC §8.4).
    /// Called by the shell on settings load and on dialog adopt.
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
        self.stream_glow.set_reduce_motion(on);
        self.agent_pill.set_reduce_motion(on);
        self.status_avatar.set_reduce_motion(on);
        self.beam.set_reduce_motion(on);
    }

    /// Whether reduced motion is on (the shell's threading assertions).
    pub fn is_reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// Thread the gateway's reachability into the presence widgets
    /// (Phase 6h): while disconnected, the status-line and card avatars
    /// sleep. The shell's `AppMessage::SeloraStatus` reduction calls this.
    pub fn set_gateway_connected(&mut self, connected: bool) {
        self.gateway_connected = connected;
    }

    /// Whether the gateway is reachable (drives the avatars' sleeping
    /// state; the shell's wiring assertions).
    pub fn is_gateway_connected(&self) -> bool {
        self.gateway_connected
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
        self.picker.open();
    }

    /// Whether the model picker dropdown is open.
    pub fn is_picker_open(&self) -> bool {
        self.picker.is_open()
    }

    /// The picker dropdown's refresh-button rect, while it is open (the
    /// headless-test seam — the same role `model_row_rects` in
    /// [`ChatPanelOutcome`] plays for the rows).
    pub fn picker_refresh_rect(&self) -> Option<Rect> {
        self.picker.last_geometry().refresh
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

    // -- sessions (Phase R3) -----------------------------------------------------

    /// The stored sessions, oldest first (the live timeline is not in
    /// here — `snapshot_sessions` stitches it in).
    pub fn sessions(&self) -> &[ChatSession] {
        &self.sessions
    }

    /// The live session's title (`""` until its first user message).
    pub fn session_title(&self) -> &str {
        &self.live_title
    }

    /// Stash the live timeline into the stored sessions (skipped when
    /// empty), then start a fresh live session. [`SessionBusy`] — the
    /// shell toasts — while a turn runs.
    pub fn new_session(&mut self) -> Result<(), SessionBusy> {
        if self.in_flight.is_some() {
            return Err(SessionBusy);
        }
        self.stash_live();
        self.entries.clear();
        self.in_flight = None;
        self.agent_in_flight = false;
        self.turn_approvals = 0;
        self.turn_cards_start = 0;
        self.live_id = uuid::Uuid::new_v4();
        self.live_title = String::new();
        self.live_created = epoch_now();
        Ok(())
    }

    /// Switch to stored session `index` (storage order, oldest first):
    /// the live timeline is stashed, the target's becomes live.
    /// [`SessionBusy`] while a turn runs.
    pub fn switch_session(&mut self, index: usize) -> Result<(), SessionBusy> {
        if self.in_flight.is_some() {
            return Err(SessionBusy);
        }
        if index >= self.sessions.len() {
            return Err(SessionBusy);
        }
        self.stash_live();
        let session = self.sessions.remove(index);
        self.load_session(session);
        Ok(())
    }

    /// Every session for `sessions.json`: the stored ones plus the live
    /// timeline last (an empty, untitled live session is skipped — it
    /// would restore as noise).
    pub fn snapshot_sessions(&self) -> Vec<ChatSession> {
        let mut all = self.sessions.clone();
        if !self.entries.is_empty() {
            let mut entries: Vec<SessionEntry> =
                self.entries.iter().map(session_entry_of).collect();
            if entries.len() > MAX_SESSION_ENTRIES {
                let excess = entries.len() - MAX_SESSION_ENTRIES;
                entries.drain(0..excess);
            }
            all.push(ChatSession {
                id: self.live_id,
                title: self.live_title.clone(),
                created_at: self.live_created,
                entries,
            });
        }
        if all.len() > MAX_SESSIONS {
            let excess = all.len() - MAX_SESSIONS;
            all.drain(0..excess);
        }
        all
    }

    /// Restore persisted sessions (app boot): the last becomes live, the
    /// rest stored.
    pub fn restore_sessions(&mut self, mut sessions: Vec<ChatSession>) {
        match sessions.pop() {
            Some(live) => {
                self.sessions = sessions;
                self.load_session(live);
            }
            None => {
                self.sessions = Vec::new();
            }
        }
    }

    /// Stash the live timeline as a stored session (kept even when only
    /// titled — an empty live timeline is dropped).
    fn stash_live(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let mut entries: Vec<SessionEntry> = self.entries.iter().map(session_entry_of).collect();
        if entries.len() > MAX_SESSION_ENTRIES {
            let excess = entries.len() - MAX_SESSION_ENTRIES;
            entries.drain(0..excess);
        }
        self.sessions.push(ChatSession {
            id: self.live_id,
            title: self.live_title.clone(),
            created_at: self.live_created,
            entries,
        });
        if self.sessions.len() > MAX_SESSIONS {
            self.sessions.remove(0);
        }
    }

    /// Make `session` the live timeline (a restored session never
    /// streams; the turn state resets).
    fn load_session(&mut self, session: ChatSession) {
        self.entries = session.entries.into_iter().map(timeline_entry_of).collect();
        self.live_id = session.id;
        self.live_title = session.title;
        self.live_created = session.created_at;
        self.in_flight = None;
        self.agent_in_flight = false;
        self.turn_approvals = 0;
        self.turn_cards_start = 0;
    }

    // -- streaming lifecycle (the shell drives these) --------------------------

    /// Append a user bubble (programmatic injection — the Enter path
    /// already echoes its own; do not call both for one message). The
    /// session's auto-title latches on the first one (Phase R3).
    pub fn push_user(&mut self, text: &str) {
        if self.live_title.is_empty() {
            self.live_title = truncate_title(text);
        }
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
        // Phase 6h: the composer's glow rises with the delta rate.
        self.stream_glow.note_delta();
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
                layout: None,
                finished_at: None,
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
        // Phase 6h: the composer's one-shot --ok spark (turn complete).
        self.stream_glow.note_turn_done();
        self.in_flight = None;
        if let Some(TimelineEntry::Bubble(message)) = self.entries.last_mut() {
            message.streaming = false;
            message.finished_at = Some(Instant::now());
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
                message.finished_at = Some(Instant::now());
            }
            _ => self.entries.push(TimelineEntry::Bubble(Message {
                role: Role::Assistant,
                content: String::new(),
                streaming: false,
                stream_started: None,
                last_delta: None,
                error: Some(error.to_owned()),
                layout: None,
                finished_at: Some(Instant::now()),
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
        // Phase 6h: the composer's one-shot --ok spark (turn complete).
        self.stream_glow.note_turn_done();
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
        // Phase R1: the composer bar grows with its buffer (wrapped rows
        // counted, capped at COMPOSER_MAX_ROWS) — the transcript yields.
        let composer_height = self.composer_height(ui);
        egui::Panel::bottom(egui::Id::new("selora_chat_composer_bar"))
            .frame(egui::Frame::default())
            .exact_size(composer_height)
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
            composer_card_rect: frame.composer_card_rect,
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

    /// The 40px header (REVAMP_SPEC §3): the 20px presence orb at the
    /// left edge, the session switcher, the `⋯` overflow trigger, and the
    /// two dropdowns (anchored overlays). The model picker and the
    /// `agent` pill live on the composer card (§2.1); opening one of the
    /// header's dropdowns still closes the picker.
    fn paint_header(&mut self, ui: &mut Ui, frame: &mut FrameArtifacts) {
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
        let painter = ui.painter();
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            egui::Stroke::new(1.0, color::INK_HI),
        );

        // -- the presence orb (§3), left edge ------------------------------
        // The same mapping as the status line over the same turn slice,
        // so header and timeline never disagree; a disconnected gateway
        // pauses the mapped state's phase-0 picture (the avatars-sleeping
        // precedent).
        let orb = header_orb(
            self.agent_in_flight,
            self.in_flight.is_some(),
            &agent_cards::status_phase_for(self.turn_cards()),
            agent_cards::status_running_tool(self.turn_cards()),
            self.streaming_active(),
            self.gateway_connected,
            self.reduce_motion,
        );
        orb.paint(
            ui,
            Rect::from_min_size(
                pos2(rect.left() + space::S3, rect.center().y - ORB_INLINE / 2.0),
                Vec2::splat(ORB_INLINE),
            ),
        );

        // -- session switcher (left of center, right of the orb) ----------
        // Phase R3: the session's title IS the header label (the static
        // "CHAT" went with it) — click for the sessions dropdown.
        let title = if self.live_title.is_empty() {
            "new chat".to_owned()
        } else {
            self.live_title.clone()
        };
        let title_font = egui::FontId::new(type_scale::SM, font::FAMILY_SANS);
        let mut label = title.clone();
        let title_galley = loop {
            let galley = painter.layout_no_wrap(label.clone(), title_font.clone(), color::FG_MUTED);
            if galley.size().x <= 140.0 || label.chars().count() <= 4 {
                break galley;
            }
            // Truncate to fit the left zone.
            let keep = label.chars().count() - 1;
            let mut shortened: String = label.chars().take(keep.saturating_sub(1)).collect();
            shortened.push('…');
            label = shortened;
        };
        let session_chevron = 12.0;
        let session_width = title_galley.size().x + session_chevron + 2.0 * space::S1;
        let session_rect = Rect::from_min_size(
            pos2(
                rect.left() + space::S3 + ORB_INLINE + space::S2,
                rect.top() + 8.0,
            ),
            vec2(session_width, 24.0),
        );
        let session_response = ui.interact(
            session_rect,
            egui::Id::new("selora_chat_session_button"),
            Sense::click(),
        );
        if session_response.hovered() {
            painter.rect_filled(session_rect, radius::R1, color::INK_FAINT);
        }
        painter.galley(
            pos2(
                session_rect.left() + space::S1,
                session_rect.center().y - title_galley.size().y / 2.0,
            ),
            title_galley,
            if session_response.hovered() {
                color::FG_DEFAULT
            } else {
                color::FG_MUTED
            },
        );
        icons::paint_chevron(
            painter,
            Rect::from_min_size(
                pos2(
                    session_rect.right() - session_chevron,
                    session_rect.center().y - 8.0,
                ),
                vec2(session_chevron, 16.0),
            ),
            color::FG_FAINT,
            self.sessions_open,
        );
        if session_response.clicked() {
            self.sessions_open = !self.sessions_open;
            self.menu_open = false;
            self.picker.close();
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
            self.picker.close();
            self.sessions_open = false;
        }

        self.paint_overflow_menu(ui, &overflow_response, frame);
        self.paint_sessions_menu(ui, &session_rect, frame);
    }

    /// The sessions dropdown (Phase R3): "New session" on top, the live
    /// session (marked), then the stored ones newest-first. A click
    /// stashes + loads (or starts fresh); anything mid-turn refuses with
    /// [`ChatPanelEvent::SessionBlocked`] (the shell toasts).
    fn paint_sessions_menu(&mut self, ui: &mut Ui, anchor: &Rect, frame: &mut FrameArtifacts) {
        let mut open = self.sessions_open;
        let mut open_new = false;
        let mut switch_to: Option<usize> = None;
        let mut blocked = false;

        if open {
            // Esc dismisses (the picker's keyboard rule).
            if ui.ctx().input(|i| i.key_pressed(Key::Escape)) {
                open = false;
            }
        }
        if open {
            let screen = ui.ctx().input(egui::InputState::viewport_rect);
            let pos = pos2(anchor.left(), anchor.bottom() + space::S1);
            let _ = screen; // the menu opens downward from the header; short lists fit
            let area = egui::Area::new(egui::Id::new("selora_chat_sessions_menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .interactable(true)
                .movable(false)
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(SESSIONS_MENU_WIDTH);
                    ui.set_max_width(SESSIONS_MENU_WIDTH);
                    egui::Frame::default()
                        .fill(color::INK_MID)
                        .stroke(egui::Stroke::new(1.0, color::INK_HI))
                        .corner_radius(radius::R2)
                        .inner_margin(egui::Margin::same(space::S2 as i8))
                        .show(ui, |ui| {
                            // New session.
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 28.0),
                                Sense::click(),
                            );
                            if response.hovered() {
                                ui.painter().rect_filled(rect, radius::R0, color::INK_FAINT);
                            }
                            ui.painter().text(
                                pos2(rect.left() + space::S3, rect.center().y),
                                Align2::LEFT_CENTER,
                                "+ new session",
                                egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                                if response.hovered() {
                                    color::FG_DEFAULT
                                } else {
                                    color::FG_MUTED
                                },
                            );
                            if response.clicked() {
                                open_new = true;
                            }

                            // The live session (marked), then stored,
                            // newest first.
                            let stored = self.sessions.clone();
                            let rows: Vec<(bool, String, String, usize)> = std::iter::once((
                                true,
                                if self.live_title.is_empty() {
                                    "new chat".to_owned()
                                } else {
                                    self.live_title.clone()
                                },
                                format!("{} turns", self.entries.len()),
                                usize::MAX, // the live row: never a switch target
                            ))
                            .chain(stored.iter().rev().enumerate().map(|(display, session)| {
                                (
                                    false,
                                    if session.title.is_empty() {
                                        "untitled".to_owned()
                                    } else {
                                        session.title.clone()
                                    },
                                    format!(
                                        "{} turns · {}",
                                        session.entries.len(),
                                        relative_time(session.created_at)
                                    ),
                                    stored.len() - 1 - display, // storage index
                                )
                            }))
                            .collect();
                            egui::ScrollArea::vertical()
                                .id_salt("selora_chat_sessions_rows")
                                .max_height(SESSION_ROW_HEIGHT * SESSION_MAX_VISIBLE_ROWS as f32)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for (is_live, title, meta, storage_index) in rows {
                                        let (rect, response) = ui.allocate_exact_size(
                                            vec2(ui.available_width(), SESSION_ROW_HEIGHT),
                                            Sense::click(),
                                        );
                                        if is_live {
                                            ui.painter().rect_filled(
                                                rect,
                                                radius::R0,
                                                color::INK_LOW,
                                            );
                                        } else if response.hovered() {
                                            ui.painter().rect_filled(
                                                rect,
                                                radius::R0,
                                                color::INK_FAINT,
                                            );
                                        }
                                        ui.painter().text(
                                            pos2(rect.left() + space::S3, rect.top() + 13.0),
                                            Align2::LEFT_CENTER,
                                            title,
                                            egui::FontId::new(type_scale::SM, font::FAMILY_SANS),
                                            if is_live || response.hovered() {
                                                color::FG_DEFAULT
                                            } else {
                                                color::FG_MUTED
                                            },
                                        );
                                        ui.painter().text(
                                            pos2(rect.right() - space::S3, rect.top() + 13.0),
                                            Align2::RIGHT_CENTER,
                                            meta,
                                            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                                            color::FG_FAINT,
                                        );
                                        if is_live {
                                            // The live row's `--ok` marker
                                            // (liveness semantics only).
                                            ui.painter().circle_filled(
                                                pos2(
                                                    rect.right() - space::S3 - 44.0,
                                                    rect.top() + 13.0,
                                                ),
                                                3.0,
                                                color::OK,
                                            );
                                        } else if response.clicked() {
                                            switch_to = Some(storage_index);
                                        }
                                    }
                                });
                        });
                });
            let menu_rect = area.response.rect;
            // Click-outside dismissal (the picker's rule).
            let clicked_outside = ui.ctx().input(|i| {
                i.pointer.any_click()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|pos| !menu_rect.contains(pos) && !anchor.contains(pos))
            });
            if clicked_outside {
                open = false;
            }
        }

        if open_new {
            if self.new_session().is_err() {
                blocked = true;
            }
            open = false;
        }
        if let Some(index) = switch_to {
            if self.switch_session(index).is_err() {
                blocked = true;
            } else {
                open = false;
            }
        }
        if blocked {
            frame.events.push(ChatPanelEvent::SessionBlocked);
        }
        self.sessions_open = open;
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
                self.picker.open();
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
        // The welcome entrance's anchor (§4): armed on the first empty
        // frame, cleared while the transcript is non-empty so a re-emptied
        // panel (Clear, new_session, a restored empty session) re-runs
        // the bloom. Read on the frame clock (`RawInput.time` drives it
        // headlessly — the `presence/mod.rs` pattern).
        let now = clock_s(ui.ctx());
        let welcome_anchor = if self.entries.is_empty() {
            Some(*self.welcome_anchor.get_or_insert(now))
        } else {
            self.welcome_anchor = None;
            None
        };
        egui::ScrollArea::vertical()
            .id_salt("selora_chat_transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |scroll| {
                scroll.add_space(AREA_PADDING);
                let content_width = scroll.available_width();
                // The empty transcript's welcome (§4): the 64px breathing
                // hero orb, the heading, and the suggestion chips (prefill
                // the composer on click), blooming in with an 80ms
                // stagger from the anchor.
                if self.entries.is_empty() {
                    let mut prefill: Option<&'static str> = None;
                    scroll.vertical_centered(|ui| {
                        ui.add_space(AREA_PADDING);
                        let (hero_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(ORB_AVATAR), Sense::hover());
                        // The hero breathes at rest (the §7.3 `pulse`);
                        // the `--ok` satellite is the gateway's liveness
                        // (presence contract #2). Under reduce-motion it
                        // is the one static muted dot, satellite-free.
                        ThinkingOrb::new(OrbState::Breathing, ORB_AVATAR)
                            .liveness(self.gateway_connected)
                            .reduce_motion(self.reduce_motion)
                            .paint(ui, hero_rect);
                        ui.add_space(space::S3);
                        ui.label(
                            egui::RichText::new("Ask anything")
                                .color(color::FG_DEFAULT)
                                .font(egui::FontId::new(
                                    type_scale::XXL,
                                    font::family_sans_semibold(),
                                )),
                        );
                        ui.add_space(space::S1);
                        ui.label(
                            egui::RichText::new(
                                "Ask about this workspace — or pick a start below.",
                            )
                            .italics()
                            .color(color::FG_MUTED)
                            .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS)),
                        );
                        ui.add_space(space::S3);
                        for (index, suggestion) in SUGGESTIONS.iter().enumerate() {
                            // The bloom (§4's `bloom` glyph, kept under
                            // reduce-motion per §7.4): alpha 0→1 and scale
                            // 0.96→1.0 on --ease-precise, one 80ms
                            // stagger step per chip. The text color is
                            // baked into the galley so the bloom's alpha
                            // rides it (job galleys ignore the paint-time
                            // fallback).
                            let eased = welcome_anchor
                                .map(|anchor| bloom_progress(now, anchor, index))
                                .unwrap_or(1.0);
                            if eased < 1.0 {
                                request_anim_frame(ui.ctx());
                            }
                            let chip_font = egui::FontId::new(type_scale::SM, font::FAMILY_SANS);
                            let galley = ui.painter().layout_no_wrap(
                                (*suggestion).to_owned(),
                                chip_font.clone(),
                                with_alpha(color::FG_MUTED, eased),
                            );
                            let chip = galley.size() + vec2(2.0 * space::S3, 0.0);
                            let (chip_rect, response) = ui.allocate_exact_size(
                                vec2(chip.x, (28.0_f32).max(chip.y + 2.0 * space::S1)),
                                Sense::click(),
                            );
                            let galley = if response.hovered() {
                                ui.painter().layout_no_wrap(
                                    (*suggestion).to_owned(),
                                    chip_font,
                                    with_alpha(color::FG_DEFAULT, eased),
                                )
                            } else {
                                galley
                            };
                            if eased > 0.0 {
                                let scale = CHIP_BLOOM_FROM + (1.0 - CHIP_BLOOM_FROM) * eased;
                                let painted = Rect::from_center_size(
                                    chip_rect.center(),
                                    chip_rect.size() * scale,
                                );
                                let (fill, border) = if response.hovered() {
                                    (color::INK_MID, color::INK_RULE)
                                } else {
                                    (color::INK_LOW, color::INK_HI)
                                };
                                ui.painter().rect(
                                    painted,
                                    radius::R1,
                                    with_alpha(fill, eased),
                                    egui::Stroke::new(1.0, with_alpha(border, eased)),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().galley(
                                    pos2(
                                        chip_rect.left() + space::S3,
                                        chip_rect.center().y - galley.size().y / 2.0,
                                    ),
                                    galley,
                                    if response.hovered() {
                                        color::FG_DEFAULT
                                    } else {
                                        color::FG_MUTED
                                    },
                                );
                            }
                            if response.clicked() {
                                prefill = Some(*suggestion);
                            }
                            ui.add_space(space::S1);
                        }
                    });
                    if let Some(suggestion) = prefill {
                        self.set_composer_text(suggestion);
                        self.focus_composer();
                    }
                    scroll.add_space(AREA_PADDING);
                }
                for index in 0..self.entries.len() {
                    match &mut self.entries[index] {
                        TimelineEntry::Bubble(message) => {
                            paint_message(
                                scroll,
                                message,
                                index,
                                content_width,
                                frame,
                                reduce_motion,
                            );
                        }
                        TimelineEntry::Card(card) => {
                            // A card is live only if its OWN turn is: the
                            // in-flight turn's cards (index ≥ the turn's
                            // start) while it runs. A dead turn's
                            // resultless card freezes (no spin, no Stop).
                            let turn_live = agent_in_flight && index >= self.turn_cards_start;
                            let outcome = agent_cards::tool_card(
                                scroll,
                                card,
                                reduce_motion,
                                turn_live,
                                self.gateway_connected,
                            );
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
                            if let Some((path, line)) = outcome.open_at {
                                frame.events.push(ChatPanelEvent::OpenFileAt { path, line });
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
                    let phase = agent_cards::status_phase_for(self.turn_cards());
                    // Phase 6h: the status line's indicator is the
                    // ThinkingOrb (the loop state mapped per the module
                    // table) next to the AgentAvatar.
                    let tool = agent_cards::status_running_tool(self.turn_cards());
                    let orb_state = orb_state_for(&phase, tool, self.streaming_active());
                    let status = agent_cards::status_line(
                        scroll,
                        &phase,
                        orb_state,
                        &mut self.status_avatar,
                        self.gateway_connected,
                        reduce_motion,
                    );
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
                // A failed stream's Retry (applied after the loop — the
                // panel methods need the borrows back).
                if frame.retry_clicked {
                    self.retry_failed(&mut frame.events);
                }
                scroll.add_space(AREA_PADDING);
            });
    }

    /// The hero composer (REVAMP_SPEC §2): the prompt card — `--ink-low`
    /// fill, `--radius-2`, a 1px `--ink-hi` hairline that lifts to
    /// `--ink-rule` on hover/focus, and the five focus glow rings — plus
    /// the footer row (the model chip and the `agent` pill left, the key
    /// hint right when it fits whole, §2.1). While the edit has focus it
    /// owns the keyboard: a plain Enter is consumed here as the submit
    /// signal (the edit's return key is Shift+Enter, so the newline
    /// lands at the caret), and on a one-line buffer the arrows walk the
    /// history (multi-line buffers keep them as caret moves). The border
    /// beam rides the card's perimeter while a request is in flight
    /// (§2.3/§8.6); the StreamGlow rides the card's bottom edge, arming
    /// the ready shimmer while connected AND focused (§2.2).
    fn paint_composer(&mut self, ui: &mut Ui, frame: &mut FrameArtifacts) {
        // The bar's rect (captured before any allocation shrinks it). The
        // card sits inside it with 12px margins on all four sides — sized
        // to the glow rings: ring 5's paint extent is 10.5px (§2.2), so
        // the margins guarantee no panel-clip on any side.
        let bar_rect = ui.max_rect();
        let card_rect = bar_rect.shrink2(vec2(space::S3, space::S3));
        let inner_rect = card_rect.shrink2(vec2(space::S3, space::S3));
        frame.composer_card_rect = card_rect;
        let want_focus = self.want_focus;
        let mut submit_now = false;
        let mut stop_clicked = false;
        // A plain (non-agent) stream: the send button becomes Stop.
        let streaming_plain = self.in_flight.is_some() && !self.agent_in_flight;

        // The composer owns the keyboard while focused (last frame's
        // focus — the edit grants it during paint). Events are consumed
        // here so the multiline edit never sees them: a plain Enter
        // submits instead of newlineing (the edit's return key is
        // Shift+Enter), and the arrows recall history on a one-line
        // buffer.
        let composer_focused = ui
            .ctx()
            .memory(|mem| mem.focused() == Some(egui::Id::new(COMPOSER_ID)));
        let mut recalled = false;
        if composer_focused {
            let mut recall_back = false;
            let mut recall_forward = false;
            let one_line = !self.composer.contains('\n');
            ui.ctx().input_mut(|input| {
                input.events.retain(|event| match event {
                    egui::Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        modifiers,
                        ..
                    } if !modifiers.shift && !modifiers.ctrl && !modifiers.alt => {
                        submit_now = true;
                        false
                    }
                    egui::Event::Key {
                        key: Key::ArrowUp,
                        pressed: true,
                        ..
                    } if one_line => {
                        recall_back = true;
                        false
                    }
                    egui::Event::Key {
                        key: Key::ArrowDown,
                        pressed: true,
                        ..
                    } if one_line => {
                        recall_forward = true;
                        false
                    }
                    _ => true,
                });
            });
            if recall_back || recall_forward {
                self.recall_history(if recall_back { -1 } else { 1 });
                recalled = true;
            }
        }

        // -- card chrome (§2.2) -------------------------------------------
        let now = clock_s(ui.ctx());
        let card_hovered = ui.rect_contains_pointer(card_rect);
        // The rings' entrance/exit: the `fade` glyph over --dur-quick on
        // a frame-clock clock (the `LiquidPill::anim` pattern). Under
        // reduce-motion they snap with focus (§11 bend #5 — the owner's
        // glow rule overrides §7.4's fade-keep for glows).
        if composer_focused != self.rings_on {
            self.rings_on = composer_focused;
            self.ring_anim = if self.reduce_motion {
                None
            } else {
                Some((now, composer_focused))
            };
        }
        let ring_alpha = if self.reduce_motion {
            if self.rings_on {
                1.0
            } else {
                0.0
            }
        } else if let Some((started, appearing)) = self.ring_anim {
            let t = ((now - started) as f32 / RING_FADE_S).clamp(0.0, 1.0);
            if t >= 1.0 {
                self.ring_anim = None;
            }
            let eased = motion::ease_bezier(motion::EASE_FADE, t);
            if appearing {
                eased
            } else {
                1.0 - eased
            }
        } else if self.rings_on {
            1.0
        } else {
            0.0
        };
        if self.ring_anim.is_some() {
            request_anim_frame(ui.ctx());
        }

        let painter = ui.painter();
        // The glow rings (§2.2): ring i expands the card by 2·(i+1), the
        // corner radius growing with it — stepped `--fg-faint` alpha, no
        // blur, no new hue.
        if ring_alpha > 0.0 {
            for (i, alpha) in GLOW_RING_ALPHAS.iter().enumerate() {
                let outset = GLOW_RING_STEP * (i as f32 + 1.0);
                painter.rect(
                    card_rect.expand(outset),
                    radius::R2 + outset,
                    egui::Color32::TRANSPARENT,
                    egui::Stroke::new(1.0, with_alpha(color::FG_FAINT, alpha * ring_alpha)),
                    egui::StrokeKind::Middle,
                );
            }
        }
        painter.rect(
            card_rect,
            radius::R2,
            color::INK_LOW,
            egui::Stroke::new(
                1.0,
                if composer_focused || card_hovered {
                    color::INK_RULE
                } else {
                    color::INK_HI
                },
            ),
            egui::StrokeKind::Inside,
        );

        // The border beam (§2.3/§8.6): rides the card's perimeter while a
        // request is in flight (a plain stream or an agent turn), painted
        // over the card stroke, under the StreamGlow band. Inactive it
        // freezes at its held phase and is simply not shown; a disconnect
        // mid-flight does not freeze it (in-flight requests complete).
        self.beam.set_active(self.in_flight.is_some());
        if self.in_flight.is_some() {
            self.beam.show(ui, card_rect);
        }

        // -- row 1: the frameless input + the send/stop button -------------
        let (rows, pitch) = self.composer_rows(ui.painter(), bar_rect.width());
        let input_row_height = (rows as f32 * pitch).max(SEND_BUTTON_SIZE.y);
        let input_rect = Rect::from_min_size(
            inner_rect.left_top(),
            vec2(inner_rect.width(), input_row_height),
        );
        let mut input_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(input_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        // The edit is frameless (§2.1 — the splash precedent): egui's
        // default edit frame would double-paint the card's fill and
        // compete with the card's focus chrome.
        let response = input_ui.add(
            egui::TextEdit::multiline(&mut self.composer)
                .id(egui::Id::new(COMPOSER_ID))
                .hint_text(COMPOSER_HINT)
                .font(egui::FontId::new(type_scale::MD, font::FAMILY_SANS))
                .frame(egui::Frame::default())
                .desired_width((inner_rect.width() - SEND_BUTTON_SIZE.x - space::S2).max(40.0))
                .desired_rows(rows)
                // Shift+Enter is the newline key (Phase R1): plain
                // Enter was consumed above as the submit signal, so
                // the caret-accurate newline needs the modifier.
                .return_key(egui::KeyboardShortcut::new(
                    egui::Modifiers::SHIFT,
                    Key::Enter,
                )),
        );
        if want_focus {
            response.request_focus();
        }
        input_ui.add_space(space::S2);

        // [⏎] send / [■] stop (§2.2 arms) — send: --ink-mid, hover
        // --ink-hi, disabled (FG_DISABLED, no fill) when empty or
        // mid-stream. While a plain stream runs the button is Stop
        // (Phase R1): a painted square, `--err` on hover (the agent
        // status line's Stop precedent).
        let send_enabled = !self.composer.trim().is_empty() && self.in_flight.is_none();
        let button_enabled = send_enabled || streaming_plain;
        let (send_rect, send_response) =
            input_ui.allocate_exact_size(SEND_BUTTON_SIZE, Sense::click());
        if button_enabled {
            let fill = if send_response.hovered() {
                color::INK_HI
            } else {
                color::INK_MID
            };
            input_ui.painter().rect_filled(send_rect, radius::R1, fill);
        }
        if streaming_plain {
            input_ui.painter().rect_filled(
                egui::Rect::from_center_size(send_rect.center(), vec2(STOP_SQUARE, STOP_SQUARE)),
                radius::R0,
                if send_response.hovered() {
                    color::ERR
                } else {
                    color::FG_DEFAULT
                },
            );
        } else {
            input_ui.painter().text(
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
        }
        frame.send_button_rect = send_rect;
        if send_response.clicked() {
            if send_enabled {
                submit_now = true;
            } else if streaming_plain {
                stop_clicked = true;
            }
        }
        self.want_focus = false;
        // A recall's external text change can surrender the edit's focus
        // (egui resets the widget state when the buffer moves under it) —
        // re-claim it next frame so typing and further recall keys keep
        // working. (Set after the consumption above, so it survives.)
        if recalled {
            self.want_focus = true;
        }

        if submit_now {
            self.submit(&mut frame.events);
            // Keep typing after a send: the cleared buffer can surrender
            // the edit's focus (the single-line composer re-armed focus
            // the same way).
            self.want_focus = true;
        }
        if stop_clicked {
            // The panel finalizes the bubble itself; the event is the
            // shell's cue to cancel the request (the late deltas are
            // stale-dropped against the cleared in-flight id).
            if let Some(id) = self.in_flight {
                self.abort_stream(id);
                frame.events.push(ChatPanelEvent::StopStream { id });
            }
        }

        // -- row 2 (footer): the model chip + the `agent` pill left; the
        // key hint right-aligned, whole or hidden (§2.1) -------------------
        let footer_rect = Rect::from_min_size(
            pos2(inner_rect.left(), input_rect.bottom() + space::S2),
            vec2(inner_rect.width(), model_picker::CLOSED_BUTTON_HEIGHT),
        );
        let chip_width = self.picker.closed_button_width(ui.painter());
        let chip_rect = Rect::from_min_size(
            footer_rect.left_top(),
            vec2(chip_width, footer_rect.height()),
        );
        // The picker's closed trigger + dropdown (moved from the header,
        // §2.1 — the dropdown self-anchors upward when the bottom
        // overflows). Selecting a model flips the picker's row
        // immediately; the shell persists the id.
        let picker_was_open = self.picker.is_open();
        for event in self.picker.show_at(ui, chip_rect) {
            match event {
                ModelPickerEvent::Select(id) => {
                    self.model = Some(id.clone());
                    frame.events.push(ChatPanelEvent::SelectModel(id));
                }
                ModelPickerEvent::Refresh => {
                    frame.events.push(ChatPanelEvent::RefreshModels);
                }
                ModelPickerEvent::Close => {}
            }
        }
        // Opening the picker closes the header's dropdowns (the mutual
        // exclusion, preserved from the header placement).
        if self.picker.is_open() && !picker_was_open {
            self.menu_open = false;
            self.sessions_open = false;
        }
        frame.model_row_rects = self.picker.last_geometry().rows;

        // The `agent` pill (UI_SPEC.md §4.2 — moved from the header; the
        // `set_on` wiring is unchanged): constant width, right-aligned at
        // its slot.
        self.agent_pill.set_on(self.agent_mode, now);
        let pill = self.agent_pill.show(
            ui,
            pos2(
                chip_rect.right() + space::S2 + agent_pill_width(ui.painter()),
                footer_rect.top(),
            ),
            "agent",
        );
        if pill.clicked {
            self.agent_mode = !self.agent_mode;
        }
        frame.agent_pill_rect = pill.rect;

        // The key hint: painted only whole (§2.1's fit-or-hide — never
        // elided). The input's placeholder remains the primary affordance.
        let hint_galley = ui.painter().layout_no_wrap(
            COMPOSER_KEY_HINT.to_owned(),
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );
        if pill.rect.right() + space::S2 + hint_galley.size().x <= footer_rect.right() {
            ui.painter().galley(
                pos2(
                    footer_rect.right() - hint_galley.size().x,
                    footer_rect.center().y - hint_galley.size().y / 2.0,
                ),
                hint_galley,
                color::FG_FAINT,
            );
        }

        // §2.2: the StreamGlow rides the card's bottom edge — it rises
        // with the delta rate, a running tool gathers it into the beam,
        // and the ready shimmer arms while connected AND focused (a
        // disconnected gateway paints none — the avatars-sleep
        // precedent).
        self.stream_glow
            .set_ready(self.gateway_connected && composer_focused);
        let processing = self.agent_in_flight && self.turn_tool_running();
        self.stream_glow.show(ui, card_rect, processing);
    }

    /// The in-flight turn's cards (index ≥ the turn's start) — the
    /// status line's and the header orb's shared slice: a previous turn's
    /// latched card must never leak into the phase, so both derive it
    /// from here each frame (§3's never-disagree rule).
    fn turn_cards(&self) -> impl Iterator<Item = &agent_cards::ToolCard> {
        self.entries[self.turn_cards_start.min(self.entries.len())..]
            .iter()
            .filter_map(|entry| match entry {
                TimelineEntry::Card(card) => Some(card),
                _ => None,
            })
    }

    /// Whether this turn has a tool executing right now (a resultless,
    /// non-parked card since the turn's start) — the StreamGlow's
    /// `processing`.
    fn turn_tool_running(&self) -> bool {
        self.entries[self.turn_cards_start.min(self.entries.len())..]
            .iter()
            .any(|entry| matches!(entry, TimelineEntry::Card(card) if card.result.is_none() && !card.is_pending()))
    }

    /// Whether a streaming bubble received a delta within the
    /// no-delta window (the status line's orb shows `connecting` while
    /// deltas stream, `working` while the model thinks between them).
    fn streaming_active(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(entry, TimelineEntry::Bubble(message)
                if message.streaming
                    && message.last_delta.is_some_and(|delta| delta.elapsed() < THINKING_AFTER))
        })
    }

    /// Submit the composer: assign the id, echo the user bubble, clear
    /// the input, emit [`ChatPanelEvent::Send`] — or, with Agent Mode on,
    /// [`ChatPanelEvent::AgentSend`] (ARCHITECTURE.md §17.3). A no-op
    /// while empty or mid-stream. The submitted text enters the recall
    /// history (Phase R1).
    fn submit(&mut self, events: &mut Vec<ChatPanelEvent>) {
        if self.in_flight.is_some() {
            return;
        }
        let text = self.composer.trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.push_history(text.clone());
        self.history_index = None;
        self.history_draft = None;
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

    /// Record a submitted prompt for the ↑/↓ recall (deduplicated against
    /// the previous entry so a resent prompt doesn't stack; capped at
    /// [`HISTORY_LEN`], oldest evicted).
    fn push_history(&mut self, text: String) {
        if self.history.last().is_some_and(|last| last == &text) {
            return;
        }
        self.history.push(text);
        if self.history.len() > HISTORY_LEN {
            self.history.remove(0);
        }
    }

    /// Walk the recall history: negative = back in time, positive =
    /// forward. The live buffer is stashed on the first step back and
    /// restored stepping off the end. A no-op with no history.
    fn recall_history(&mut self, direction: i32) {
        if self.history.is_empty() {
            return;
        }
        match (self.history_index, direction < 0) {
            (None, true) => {
                self.history_draft = Some(std::mem::take(&mut self.composer));
                let last = self.history.len() - 1;
                self.history_index = Some(last);
                self.composer.clone_from(&self.history[last]);
            }
            (Some(index), true) => {
                let prev = index.saturating_sub(1);
                self.history_index = Some(prev);
                self.composer.clone_from(&self.history[prev]);
            }
            (Some(index), false) => {
                if index + 1 < self.history.len() {
                    self.history_index = Some(index + 1);
                    self.composer.clone_from(&self.history[index + 1]);
                } else {
                    // Forward off the end: back to the live buffer.
                    self.history_index = None;
                    self.composer = self.history_draft.take().unwrap_or_default();
                }
            }
            (None, false) => {}
        }
    }

    /// The composer's visible row count and its measured row pitch for
    /// the current buffer at the panel's width: wrapped lines counted
    /// (the same font and the same wrap expression the edit paints with,
    /// §2.1), the count capped at [`COMPOSER_MAX_ROWS`]. The pitch is the
    /// laid-out galley's per-row height — measured each frame, never
    /// assumed (§2.1).
    fn composer_rows(&self, painter: &egui::Painter, panel_width: f32) -> (usize, f32) {
        let card_inner = panel_width - 2.0 * space::S3 - 2.0 * space::S3;
        let wrap = (card_inner - SEND_BUTTON_SIZE.x - space::S2).max(40.0);
        let galley = painter.layout(
            self.composer.clone(),
            egui::FontId::new(type_scale::MD, font::FAMILY_SANS),
            color::FG_DEFAULT,
            wrap,
        );
        let laid_out = galley.rows.len().max(1);
        let pitch = galley.size().y / laid_out as f32;
        (laid_out.clamp(1, COMPOSER_MAX_ROWS), pitch)
    }

    /// The composer bar's height this frame (§2.1): the card (12px
    /// padding both sides, the input row, 8px, the 24px footer) plus the
    /// 12px card margins — `input_row + 80` where
    /// `input_row = max(rows · pitch, 20)` (the edit row's min-height
    /// clamp is real layout behavior, so it is in the formula).
    fn composer_height(&self, ui: &Ui) -> f32 {
        let (rows, pitch) = self.composer_rows(ui.painter(), ui.available_width());
        let input_row = (rows as f32 * pitch).max(SEND_BUTTON_SIZE.y);
        input_row
            + 2.0 * space::S3
            + 2.0 * space::S3
            + space::S2
            + model_picker::CLOSED_BUTTON_HEIGHT
    }

    /// Retry a failed stream (Phase R2, the failed bubble's footer
    /// button): the failed bubble is dropped and its user message
    /// re-sent — a fresh [`ChatPanelEvent::Send`] whose request is built
    /// from the conversation (the user turn is still there). A no-op
    /// while anything is in flight or when the last entry isn't the
    /// failed bubble (the button only paints there).
    fn retry_failed(&mut self, events: &mut Vec<ChatPanelEvent>) {
        if self.in_flight.is_some() {
            return;
        }
        let failed_is_last = matches!(
            self.entries.last(),
            Some(TimelineEntry::Bubble(message)) if message.error.is_some()
        );
        if !failed_is_last {
            return;
        }
        let last_user = self.entries.iter().rev().find_map(|entry| match entry {
            TimelineEntry::Bubble(message) if message.role == Role::User => {
                Some(message.content.clone())
            }
            _ => None,
        });
        if let Some(text) = last_user {
            self.entries.pop(); // the failed bubble goes; the user turn stays
            let id = ChatId::next();
            self.in_flight = Some(id);
            events.push(ChatPanelEvent::Send { id, text });
        }
    }

    /// The user stopped a plain stream (the composer's Stop button,
    /// Phase R1): the bubble keeps whatever streamed, stops streaming,
    /// and the composer unlocks. Late deltas for the id are
    /// stale-dropped; the partial text stays in the conversation for the
    /// next request (the ZCode-class behavior).
    pub fn abort_stream(&mut self, id: ChatId) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        if let Some(TimelineEntry::Bubble(message)) = self.entries.last_mut() {
            message.streaming = false;
        }
        // A bubble that never received a delta is dropped (the
        // complete_stream precedent).
        let is_empty_assistant = matches!(
            self.entries.last(),
            Some(TimelineEntry::Bubble(message))
                if message.role == Role::Assistant && message.content.is_empty()
        );
        if is_empty_assistant {
            self.entries.pop();
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
        layout: None,
        finished_at: None,
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

/// The status line's orb state for the loop phase (Phase 6h — the
/// thinking-orbs port's mapping; ARCHITECTURE.md §17.3 states):
/// `Thinking` → `working` (or `connecting` while deltas stream);
/// read-type tools → `searching`; bash/write tools → `solving`;
/// parked on an approval → `listening`.
fn orb_state_for(
    phase: &agent_cards::StatusPhase,
    running_tool: Option<&str>,
    streaming_active: bool,
) -> OrbState {
    use agent_cards::StatusPhase;
    match phase {
        StatusPhase::AwaitingApproval => OrbState::Listening,
        StatusPhase::Thinking => {
            if streaming_active {
                OrbState::Connecting
            } else {
                OrbState::Working
            }
        }
        StatusPhase::Running(_) => match running_tool {
            Some("bash" | "write_file" | "edit_file" | "delete_file") => OrbState::Solving,
            _ => OrbState::Searching,
        },
    }
}

/// The header orb (REVAMP_SPEC §3): the status line's mapping over the
/// same turn slice, so header and timeline can never disagree — an agent
/// turn maps per [`orb_state_for`]; a plain (non-agent) stream is
/// `Connecting` while deltas flow and `Working` while the model thinks;
/// idle is `Breathing`. A disconnected gateway pauses the *mapped*
/// state's phase-0 picture (a paused `Working` still paints its three
/// dots, statically — not a frozen breathing dot).
fn header_orb(
    agent_in_flight: bool,
    in_flight: bool,
    phase: &agent_cards::StatusPhase,
    running_tool: Option<&str>,
    streaming_active: bool,
    gateway_connected: bool,
    reduce_motion: bool,
) -> ThinkingOrb {
    let state = if agent_in_flight {
        orb_state_for(phase, running_tool, streaming_active)
    } else if in_flight {
        if streaming_active {
            OrbState::Connecting
        } else {
            OrbState::Working
        }
    } else {
        OrbState::Breathing
    };
    ThinkingOrb::new(state, ORB_INLINE)
        .paused(!gateway_connected)
        .reduce_motion(reduce_motion)
}

/// The agent pill's constant width — the `LiquidPill` formula
/// (S2 + dot + S1 + label + S2), with the dot diameter mirrored as
/// [`AGENT_PILL_DOT`] (file-private there). The composer footer's
/// fit-or-hide math (§2.1) needs the pill's width before it paints.
fn agent_pill_width(painter: &egui::Painter) -> f32 {
    let galley = painter.layout_no_wrap(
        "agent".to_owned(),
        egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
        color::FG_MUTED,
    );
    space::S2 + AGENT_PILL_DOT + space::S1 + galley.size().x + space::S2
}

/// `""` for 1, `"s"` otherwise (the summary footer's wording).
pub(crate) fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

// -----------------------------------------------------------------------------
// Bubble painting (free functions: the paint loop borrows the transcript
// immutably while click targets are collected for later application)
// -----------------------------------------------------------------------------

// -----------------------------------------------------------------------------
// Layout builders (Phase R2): markdown blocks -> egui layout jobs
// -----------------------------------------------------------------------------

/// Build the layout job for one prose block's runs: each run carries its
/// style's font/color (`**strong**` SemiBold, `*emphasis*` Medium, `` `code` ``
/// mono on `--ink-canvas`, links underlined); links record their char
/// ranges for click hit-testing. `fade` bakes into the colors (the
/// streaming entrance — job galleys ignore the paint-time fallback).
fn prose_job(
    runs: &[markdown::Run],
    wrap: f32,
    fade: f32,
    heading: Option<u8>,
    quote: bool,
) -> (egui::text::LayoutJob, Vec<LinkSpan>) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut links = Vec::new();
    let base = if quote {
        color::FG_MUTED
    } else {
        color::FG_DEFAULT
    };
    let faded = with_alpha(base, fade);
    let heading_size = |default: f32| match heading {
        Some(3) => type_scale::XL,
        Some(4) => type_scale::LG,
        _ => default,
    };
    for run in runs {
        let format = match run.style {
            markdown::RunStyle::Plain => egui::TextFormat::simple(
                egui::FontId::new(heading_size(type_scale::MD), font::FAMILY_SANS),
                faded,
            ),
            markdown::RunStyle::Strong => egui::TextFormat::simple(
                egui::FontId::new(heading_size(type_scale::MD), font::family_sans_semibold()),
                faded,
            ),
            markdown::RunStyle::Emphasis => egui::TextFormat::simple(
                egui::FontId::new(heading_size(type_scale::MD), font::family_sans_medium()),
                faded,
            ),
            markdown::RunStyle::Code => egui::TextFormat {
                font_id: code_font(),
                color: with_alpha(color::FG_DEFAULT, fade),
                background: with_alpha(color::INK_CANVAS, fade),
                ..Default::default()
            },
            markdown::RunStyle::Link => egui::TextFormat {
                font_id: prose_font(),
                color: faded,
                underline: egui::Stroke::new(1.0, faded),
                ..Default::default()
            },
        };
        let char_start = job.text.chars().count();
        job.append(&run.text, 0.0, format);
        let char_end = job.text.chars().count();
        if let Some(url) = &run.url {
            links.push(LinkSpan {
                range: char_start..char_end,
                url: url.clone(),
            });
        }
    }
    (job, links)
}

/// Build the layout job for a fenced code block: tree-sitter sections in
/// the §1.5 mono-tinted palette (`--syn-*`), the gaps in `--syn-default`
/// (§2.5's comment italics are the editor's; chat code paints color only
/// — the SM mono has no italic face).
fn code_job(
    body: &str,
    language: Option<selora_editor::Language>,
    wrap: f32,
    fade: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    let mono = code_font();
    let append_plain = |job: &mut egui::text::LayoutJob, text: &str| {
        if !text.is_empty() {
            job.append(
                text,
                0.0,
                egui::TextFormat::simple(mono.clone(), with_alpha(color::SYN_DEFAULT, fade)),
            );
        }
    };
    let spans = language
        .and_then(selora_editor::SyntaxLayer::new)
        .map(|mut layer| {
            layer.set_source(body);
            layer.spans().to_vec()
        })
        .unwrap_or_default();
    let mut pos = 0usize;
    for span in spans {
        if span.start as usize > pos {
            append_plain(&mut job, &body[pos..span.start as usize]);
        }
        let format = egui::TextFormat::simple(mono.clone(), with_alpha(span.kind.color(), fade));
        job.append(&body[span.start as usize..span.end as usize], 0.0, format);
        pos = span.end as usize;
    }
    if pos < body.len() {
        append_plain(&mut job, &body[pos..]);
    }
    job
}

/// Parse and lay out a message's blocks at the wrap widths (the cache's
/// builder): the parts plus their total height, gaps included.
fn build_parts(
    content: &str,
    wrap: f32,
    code_wrap: f32,
    fade: f32,
    painter: &egui::Painter,
) -> (Vec<Part>, f32) {
    let mut parts = Vec::new();
    let mut height = 0.0f32;
    for block in markdown::parse(content) {
        match block {
            markdown::Block::Paragraph { runs } => {
                let (job, links) = prose_job(&runs, wrap, fade, None, false);
                let galley = painter.layout_job(job);
                if height > 0.0 {
                    height += BUBBLE_GAP;
                }
                height += galley.size().y;
                parts.push(Part::Prose {
                    galley,
                    links,
                    quote: false,
                });
            }
            markdown::Block::Heading { level, runs } => {
                let (job, links) = prose_job(&runs, wrap, fade, Some(level), false);
                let galley = painter.layout_job(job);
                if height > 0.0 {
                    height += BUBBLE_GAP;
                }
                height += galley.size().y;
                parts.push(Part::Prose {
                    galley,
                    links,
                    quote: false,
                });
            }
            markdown::Block::Quote { runs } => {
                // The quote indents past its left rule.
                let quote_wrap = (wrap - space::S2).max(20.0);
                let (job, links) = prose_job(&runs, quote_wrap, fade, None, true);
                let galley = painter.layout_job(job);
                if height > 0.0 {
                    height += BUBBLE_GAP;
                }
                height += galley.size().y;
                parts.push(Part::Prose {
                    galley,
                    links,
                    quote: true,
                });
            }
            markdown::Block::List { items, ordered } => {
                for (index, item) in items.iter().enumerate() {
                    if height > 0.0 || index > 0 {
                        height += if index > 0 { LIST_ITEM_GAP } else { BUBBLE_GAP };
                    }
                    let marker = if ordered {
                        format!("{}. ", index + 1)
                    } else {
                        "- ".to_owned()
                    };
                    let mut runs = vec![markdown::Run::plain(marker)];
                    runs.extend(item.iter().cloned());
                    let (job, links) = prose_job(&runs, wrap, fade, None, false);
                    let galley = painter.layout_job(job);
                    height += galley.size().y;
                    parts.push(Part::Prose {
                        galley,
                        links,
                        quote: false,
                    });
                }
            }
            markdown::Block::Rule => {
                if height > 0.0 {
                    height += BUBBLE_GAP;
                }
                height += 9.0; // 4px above, the 1px line, 4px below
                parts.push(Part::Rule);
            }
            markdown::Block::Code {
                lang,
                body,
                complete,
            } => {
                let language = lang.as_deref().and_then(markdown::fence_language);
                let job = code_job(&body, language, code_wrap, fade);
                let galley = painter.layout_job(job);
                let block_height = CODE_HEADER_HEIGHT + galley.size().y + 2.0 * BUBBLE_PADDING;
                if height > 0.0 {
                    height += BUBBLE_GAP;
                }
                height += block_height;
                parts.push(Part::Code {
                    body,
                    galley,
                    block_height,
                    complete,
                    lang,
                });
            }
        }
    }
    (parts, height)
}

/// `1.2s` / `12s` / `1m 03s` — the footer's elapsed form.
fn format_elapsed(duration: Duration) -> String {
    let secs = duration.as_secs_f32();
    if secs < 10.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{secs:.0}s")
    } else {
        let whole = duration.as_secs();
        format!("{}m {:02}s", whole / 60, whole % 60)
    }
}

/// Paint one bubble. User bubbles are fitted and right-aligned on
/// `--ink-mid`; assistant (and system) bubbles span the full width minus
/// the insets on `--ink-low`, rendering the markdown blocks (headings,
/// lists, quotes, rules, highlighted code, inline runs — Phase R2) with
/// the streaming fade and the ThinkingOrb indicator when mid-stream.
fn paint_message(
    ui: &mut Ui,
    message: &mut Message,
    message_index: usize,
    content_width: f32,
    frame: &mut FrameArtifacts,
    reduce_motion: bool,
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

    // Assistant: parse + lay out the markdown blocks (Phase R2), cached
    // on the message — rebuilt when the content or the wrap width moves
    // (a streaming bubble re-lays out per delta, a settled one once);
    // the fade's first 240ms rebuilds per frame (it bakes into the
    // section colors).
    let wrap = (max_bubble_width - 2.0 * BUBBLE_PADDING).max(20.0);
    let code_wrap = (wrap - 2.0 * BUBBLE_PADDING).max(20.0);
    let stale = message
        .layout
        .as_ref()
        .is_none_or(|cached| cached.content_len != message.content.len() || cached.wrap != wrap);
    if stale || fade < 1.0 {
        let (parts, parts_height) =
            build_parts(&message.content, wrap, code_wrap, fade, ui.painter());
        message.layout = Some(CachedLayout {
            content_len: message.content.len(),
            wrap,
            parts,
            height: parts_height,
        });
    }
    let cached = message.layout.as_ref().expect("built above");
    let mut content_height = cached.height;

    // The thinking indicator: after 800ms without a delta, or while
    // waiting for the first one (UI_SPEC.md §4's trigger). Phase 6h: the
    // ThinkingOrb (20px, `working`) replaces the ⠿⠿⠿ wave. An empty
    // bubble reserves the orb's band so its height is stable as it
    // streams.
    let thinking = message
        .last_delta
        .is_some_and(|delta| delta.elapsed() > THINKING_AFTER);
    let show_orb = message.streaming && (thinking || message.content.is_empty());
    if show_orb {
        content_height += ORB_INLINE;
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
    for part in &cached.parts {
        match part {
            Part::Prose {
                galley,
                links,
                quote,
            } => {
                let indent = if *quote { space::S2 } else { 0.0 };
                if *quote {
                    // The quote's left rule.
                    ui.painter().vline(
                        bubble_rect.left() + BUBBLE_PADDING,
                        egui::Rangef::new(cursor, cursor + galley.size().y),
                        egui::Stroke::new(1.0, with_alpha(color::INK_RULE, fade)),
                    );
                }
                let galley_pos = pos2(bubble_rect.left() + BUBBLE_PADDING + indent, cursor);
                ui.painter()
                    .galley(galley_pos, galley.clone(), color::FG_DEFAULT);
                // Link clicks: hit-test the pointer against the galley's
                // char ranges (job galleys keep their baked colors; the
                // fallback is inert).
                if !links.is_empty() {
                    let galley_rect = Rect::from_min_size(galley_pos, galley.size());
                    if ui.rect_contains_pointer(galley_rect) {
                        if let Some(local) = ui
                            .input(|i| i.pointer.interact_pos())
                            .map(|pos| pos - galley_pos)
                        {
                            let hit = galley.cursor_from_pos(local);
                            for link in links {
                                if link.range.contains(&hit.index.0)
                                    && ui.input(|i| i.pointer.primary_clicked())
                                {
                                    ui.ctx().open_url(egui::OpenUrl {
                                        url: link.url.clone(),
                                        new_tab: true,
                                    });
                                }
                            }
                        }
                    }
                }
                cursor += galley.size().y;
            }
            Part::Rule => {
                ui.painter().hline(
                    egui::Rangef::new(
                        bubble_rect.left() + BUBBLE_PADDING,
                        bubble_rect.right() - BUBBLE_PADDING,
                    ),
                    cursor + 4.0,
                    egui::Stroke::new(1.0, with_alpha(color::INK_RULE, fade)),
                );
                cursor += 9.0;
            }
            Part::Code {
                body,
                galley,
                block_height,
                complete,
                lang,
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
                // The header row: the language label left, Copy right
                // (on hover of the block, closed blocks only — Phase
                // R2's header; the label falls back to "text").
                let header_center = block_rect.top() + CODE_HEADER_HEIGHT / 2.0;
                ui.painter().text(
                    pos2(block_rect.left() + BUBBLE_PADDING, header_center),
                    Align2::LEFT_CENTER,
                    lang.as_deref().unwrap_or("text"),
                    egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                    with_alpha(color::FG_FAINT, fade),
                );
                let block_hovered = ui.rect_contains_pointer(block_rect);
                if block_hovered && *complete {
                    let copy_rect = Rect::from_min_size(
                        pos2(block_rect.right() - 4.0 - 40.0, block_rect.top() + 3.0),
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
                ui.painter().galley(
                    pos2(
                        block_rect.left() + BUBBLE_PADDING,
                        block_rect.top() + CODE_HEADER_HEIGHT,
                    ),
                    galley.clone(),
                    color::FG_DEFAULT,
                );
                frame.code_block_rects.push(block_rect);
                frame.code_blocks_painted += 1;
                cursor += *block_height;
            }
        }
        cursor += BUBBLE_GAP;
    }

    if show_orb {
        // The orb drives its clock from the frame time and requests its
        // own repaints while it animates (the ghost_text.rs pattern) —
        // including through a quiet gap between deltas, which the wave
        // never survived.
        let orb_rect = Rect::from_min_size(
            pos2(bubble_rect.left() + BUBBLE_PADDING, cursor),
            vec2(ORB_INLINE, ORB_INLINE),
        );
        ThinkingOrb::new(OrbState::Working, ORB_INLINE)
            .reduce_motion(reduce_motion)
            .paint(ui, orb_rect);
    }

    // The footer row (Phase R2): a failed stream renders its note and a
    // Retry (the only red outside diagnostics: --err, per the phase's
    // color discipline); a successful one renders the elapsed/token
    // metadata and the message-level Copy.
    if message.error.is_some() {
        let note = format!(
            "stream failed: {}",
            message.error.as_deref().unwrap_or_default()
        );
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
        // Retry: re-send the failed turn's user message (the panel's
        // `retry_failed`, applied by the paint loop).
        let retry_rect = Rect::from_min_size(
            pos2(note_rect.right() - BUBBLE_INSET - 40.0, note_rect.top()),
            vec2(40.0, 16.0),
        );
        let retry_response = ui.interact(
            retry_rect,
            egui::Id::new(("selora_chat_retry", message_index)),
            Sense::click(),
        );
        let retry_color = if retry_response.hovered() {
            color::FG_DEFAULT
        } else {
            color::FG_MUTED
        };
        ui.painter().text(
            retry_rect.center(),
            Align2::CENTER_CENTER,
            "retry",
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            retry_color,
        );
        if retry_response.clicked() {
            frame.retry_clicked = true;
        }
    } else if message.stream_started.is_some() {
        // The metadata clock ticks while streaming: keep frames coming.
        if message.streaming {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        let elapsed = match (message.stream_started, message.finished_at) {
            (Some(started), Some(finished)) => finished.duration_since(started),
            (Some(started), None) => started.elapsed(),
            _ => Duration::ZERO,
        };
        let tokens = message.content.chars().count() / 4;
        let meta = format!("{} · ~{} tok", format_elapsed(elapsed), tokens);
        let (meta_rect, _) = ui.allocate_exact_size(vec2(content_width, 16.0), Sense::hover());
        ui.painter().text(
            pos2(meta_rect.left() + BUBBLE_INSET, meta_rect.center().y),
            Align2::LEFT_CENTER,
            meta,
            egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
            color::FG_FAINT,
        );
        // The message-level Copy (the code blocks keep their own).
        if !message.content.trim().is_empty() {
            let copy_rect = Rect::from_min_size(
                pos2(meta_rect.right() - BUBBLE_INSET - 44.0, meta_rect.top()),
                vec2(44.0, 16.0),
            );
            let copy_response = ui.interact(
                copy_rect,
                egui::Id::new(("selora_chat_message_copy", message_index)),
                Sense::click(),
            );
            let copy_color = if copy_response.hovered() {
                color::FG_DEFAULT
            } else {
                color::FG_FAINT
            };
            ui.painter().text(
                copy_rect.center(),
                Align2::CENTER_CENTER,
                "copy",
                egui::FontId::new(type_scale::XS, font::FAMILY_SANS),
                copy_color,
            );
            if copy_response.clicked() {
                frame.copies.push(message.content.clone());
            }
        }
    }

    frame.bubbles_painted += 1;
}

/// Prose font: `--type-md` in `--font-sans` (DESIGN_TOKENS.md §2).
fn prose_font() -> egui::FontId {
    egui::FontId::new(type_scale::MD, font::FAMILY_SANS)
}

/// Code font: `--type-sm` in `--font-mono` (JetBrains Mono).
fn code_font() -> egui::FontId {
    egui::FontId::new(type_scale::SM, font::FAMILY_MONO)
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

    /// A focused window's input carrying the given events at the given
    /// frame-clock time (focus is what makes egui deliver key events to
    /// the widgets at all; the explicit time drives the presence
    /// animations headlessly — the `presence/mod.rs` pattern).
    fn raw_input_with(events: Vec<egui::Event>, time: f64) -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(test_screen()),
            events,
            time: Some(time),
            ..Default::default()
        }
    }

    /// One headless frame over a shared context; returns the `FullOutput`
    /// (the shape/repaint assertions need it) alongside the outcome.
    /// The bundled fonts are registered per frame (Phase R2's headings
    /// use the named SemiBold family; `set_fonts` is idempotent and the
    /// handful of test frames re-shaping galleys costs nothing).
    fn frame_out(
        ctx: &egui::Context,
        input: egui::RawInput,
        panel: &mut ChatPanel,
    ) -> (egui::FullOutput, ChatPanelOutcome) {
        crate::theme::set_fonts(ctx);
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
        (output, outcome)
    }

    /// One headless frame over a shared context; returns the outcome.
    fn frame(
        ctx: &egui::Context,
        input: egui::RawInput,
        panel: &mut ChatPanel,
    ) -> ChatPanelOutcome {
        frame_out(ctx, input, panel).1
    }

    /// Sweep a click grid (the four-frame move/move/press/release pattern
    /// per point) over the panel, returning the release-frame's outcome
    /// for the first point where `want` holds (Phase R2's probe for
    /// hover-revealed affordances whose rects the outcome does not
    /// report).
    fn click_sweep(
        ctx: &egui::Context,
        panel: &mut ChatPanel,
        xs: std::ops::Range<usize>,
        ys: std::ops::Range<usize>,
        step: usize,
        want: impl Fn(&ChatPanelOutcome, &ChatPanel) -> bool,
    ) -> Option<ChatPanelOutcome> {
        for y in ys.step_by(step) {
            for x in xs.clone().step_by(step) {
                let pos = egui::pos2(x as f32, y as f32);
                let press = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ];
                let release = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ];
                let mut last = None;
                for events in [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerMoved(pos)],
                    press,
                    release,
                ] {
                    last = Some(frame(ctx, raw_input_with(events, 0.0), panel));
                }
                if let Some(outcome) = last {
                    if want(&outcome, panel) {
                        return Some(outcome);
                    }
                }
            }
        }
        None
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
            raw_input_with(vec![key_event(Key::Enter, Default::default())], 0.0),
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
            raw_input_with(vec![key_event(Key::Enter, egui::Modifiers::SHIFT)], 0.0),
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
            raw_input_with(vec![key_event(Key::Enter, Default::default())], 0.0),
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
            raw_input_with(vec![key_event(Key::Enter, Default::default())], 0.0),
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
        let outcome = frame(&ctx, raw_input_with(click_at(center), 0.0), &mut panel);

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
        let outcome = frame(&ctx, raw_input_with(click_at(center), 0.0), &mut panel);
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
            raw_input_with(vec![key_event(Key::Enter, Default::default())], 0.0),
            &mut panel,
        );
        assert!(outcome.events.is_empty(), "events: {:?}", outcome.events);

        // The send button is equally gated — and while a plain stream
        // runs the button IS the Stop affordance (Phase R1): the click
        // aborts the stream instead of sending.
        let center = outcome.send_button_rect.center();
        let outcome = frame(&ctx, raw_input_with(click_at(center), 0.0), &mut panel);
        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        assert!(
            matches!(
                &outcome.events[0],
                ChatPanelEvent::StopStream { id: stop_id } if *stop_id == id
            ),
            "the click stops the in-flight stream: {:?}",
            outcome.events
        );
        assert!(!panel.is_streaming(), "the panel unlocked");
        assert_eq!(panel.turn_count(), 2); // user bubble + the finalized partial bubble
    }

    #[test]
    fn enter_submits_a_multiline_buffer() {
        // The Enter interception works on a multi-line buffer too: plain
        // Enter submits (never newlines), the panel sends the whole text.
        let mut panel = ChatPanel::new();
        panel.focus_composer();
        panel.set_composer_text("line one\nline two");
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(
            &ctx,
            raw_input_with(vec![key_event(Key::Enter, Default::default())], 0.0),
            &mut panel,
        );
        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::Send { text, .. } => {
                assert_eq!(text, "line one\nline two");
            }
            other => panic!("expected Send, got {other:?}"),
        }
        // The multi-line buffer left the composer (cleared with the send).
        assert_eq!(panel.composer_text(), "");
    }

    #[test]
    fn arrow_recall_walks_the_history_and_restores_the_draft() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("first");
        let id = send(&mut panel).expect("first");
        panel.complete_stream(id);
        panel.set_composer_text("second");
        let id = send(&mut panel).expect("second");
        panel.complete_stream(id);

        // The live buffer is empty; recall parks it as the draft.
        panel.focus_composer();
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let press = |key: Key| raw_input_with(vec![key_event(key, Default::default())], 0.0);
        let outcome = frame(&ctx, press(Key::ArrowUp), &mut panel);
        assert!(outcome.events.is_empty());
        assert_eq!(panel.composer_text(), "second");

        frame(&ctx, egui::RawInput::default(), &mut panel);
        let _ = frame(&ctx, press(Key::ArrowUp), &mut panel);
        assert_eq!(panel.composer_text(), "first");

        // Saturates at the oldest entry.
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let _ = frame(&ctx, press(Key::ArrowUp), &mut panel);
        assert_eq!(panel.composer_text(), "first");

        // Forward walks back, then off the end restores the live draft.
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let _ = frame(&ctx, press(Key::ArrowDown), &mut panel);
        assert_eq!(panel.composer_text(), "second");
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let _ = frame(&ctx, press(Key::ArrowDown), &mut panel);
        assert_eq!(panel.composer_text(), "");
    }

    #[test]
    fn abort_stream_finalizes_the_partial_bubble_and_unlocks() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("question");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "partial answer");

        panel.abort_stream(id);
        assert!(!panel.is_streaming(), "the composer unlocked");
        assert_eq!(panel.turn_count(), 2, "user bubble + the partial bubble");
        // The partial text stays: it re-enters the next request's context.
        assert!(
            panel
                .conversation()
                .iter()
                .any(|message| message.content == "partial answer"),
            "conversation: {:?}",
            panel.conversation()
        );

        // Late deltas for the aborted id are stale-dropped.
        panel.push_assistant_delta(id, "late");
        assert_eq!(panel.turn_count(), 2);

        // A second abort of the same id is a no-op.
        panel.abort_stream(id);
        assert_eq!(panel.turn_count(), 2);
    }

    // -- Phase R2: markdown rendering, actions, the empty state ---------------

    #[test]
    fn markdown_message_paints_blocks_and_code() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("go");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(
            id,
            "### Title\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\nplain tail",
        );
        panel.complete_stream(id);
        let ctx = egui::Context::default();
        let _ = frame(&ctx, egui::RawInput::default(), &mut panel);
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        // The user bubble + the markdown bubble.
        assert_eq!(outcome.bubbles_painted, 2);
        assert_eq!(outcome.code_blocks_painted, 1);
        assert_eq!(panel.turn_count(), 2);
    }

    #[test]
    fn the_layout_cache_holds_across_frames() {
        // A settled bubble re-lays out only when the content or the wrap
        // width moves: the cache object survives frames (observable via
        // the stale-check — same content, same width, no rebuild).
        let mut panel = ChatPanel::new();
        panel.set_composer_text("go");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "answer body");
        panel.complete_stream(id);
        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        frame(&ctx, egui::RawInput::default(), &mut panel);
        let cached = panel
            .entries
            .iter()
            .rev()
            .find_map(|entry| match entry {
                TimelineEntry::Bubble(message) => message.layout.as_ref(),
                _ => None,
            })
            .expect("the bubble built its layout");
        assert_eq!(cached.content_len, "answer body".len());
        assert!(!cached.parts.is_empty());
    }

    #[test]
    fn link_click_opens_the_url() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("go");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "see [docs](https://selora.dev) now");
        panel.complete_stream(id);
        let ctx = egui::Context::default();
        crate::theme::set_fonts(&ctx);
        for _ in 0..2 {
            let mut output = ctx.run_ui(raw_input_with(vec![], 0.0), |ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                panel.show(ui);
            });
            output.textures_delta.clear();
        }
        // Sweep the bubble's area; the release frame that opened the url
        // carries it in its FullOutput.
        let mut opened = None;
        'probe: for y in (48..=160).step_by(6) {
            for x in (12..=340).step_by(6) {
                let pos = egui::pos2(x as f32, y as f32);
                let mut last_output = None;
                for events in [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerMoved(pos)],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        },
                    ],
                ] {
                    let mut output = ctx.run_ui(raw_input_with(events, 0.0), |ui| {
                        ui.set_min_size(vec2(360.0, 600.0));
                        panel.show(ui);
                    });
                    output.textures_delta.clear();
                    last_output = Some(output);
                }
                if let Some(output) = last_output {
                    let url =
                        output
                            .platform_output
                            .commands
                            .iter()
                            .find_map(|command| match command {
                                egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
                                _ => None,
                            });
                    if let Some(url) = url {
                        opened = Some(url);
                        break 'probe;
                    }
                }
            }
        }
        assert_eq!(opened.as_deref(), Some("https://selora.dev"));
    }

    #[test]
    fn empty_state_suggestion_prefills_the_composer() {
        let mut panel = ChatPanel::new();
        let ctx = egui::Context::default();
        for _ in 0..2 {
            frame(&ctx, egui::RawInput::default(), &mut panel);
        }
        assert_eq!(panel.composer_text(), "");

        // The chips sit centered in the welcome; a click prefills.
        let _swept = click_sweep(&ctx, &mut panel, 40..320, 120..500, 8, |_, panel| {
            !panel.composer_text().is_empty()
        });
        assert!(
            sweep_hit(&panel),
            "a chip was clicked: {}",
            panel.composer_text()
        );
        assert!(SUGGESTIONS.contains(&panel.composer_text()));
    }

    /// Whether the composer holds one of the suggestions (the empty-state
    /// test's sweep detector — a tiny indirection so the closure borrows
    /// stay clean).
    fn sweep_hit(panel: &ChatPanel) -> bool {
        SUGGESTIONS.contains(&panel.composer_text())
    }

    #[test]
    fn retry_resends_the_failed_turns_user_message() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hello");
        let id = send(&mut panel).expect("send");
        panel.fail_stream(id, "boom");
        let ctx = egui::Context::default();
        for _ in 0..2 {
            frame(&ctx, egui::RawInput::default(), &mut panel);
        }
        assert_eq!(panel.turn_count(), 2); // user bubble + failed bubble

        // The retry button sits right-aligned on the error row under the
        // bubble.
        let outcome = click_sweep(&ctx, &mut panel, 200..330, 60..260, 8, |outcome, _| {
            outcome
                .events
                .iter()
                .any(|event| matches!(event, ChatPanelEvent::Send { .. }))
        })
        .expect("the retry button was clicked");
        assert!(matches!(
            &outcome.events[0],
            ChatPanelEvent::Send { text, .. } if text == "hello"
        ));
        assert!(panel.is_streaming());
        assert_eq!(panel.turn_count(), 1); // the failed bubble went; the user turn stays
    }

    // -- sessions (Phase R3) ------------------------------------------------------

    #[test]
    fn new_session_stashes_titled_timelines() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("first question");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "an answer");
        panel.complete_stream(id);
        assert_eq!(panel.session_title(), "first question");

        panel.new_session().expect("new session");
        assert_eq!(panel.turn_count(), 0, "the live timeline cleared");
        assert_eq!(panel.session_title(), "", "a fresh session is untitled");
        assert_eq!(panel.sessions().len(), 1);
        assert_eq!(panel.sessions()[0].title, "first question");
        assert_eq!(panel.sessions()[0].entries.len(), 2);

        // An empty live session never stashes (no noise).
        panel.new_session().expect("new session again");
        assert_eq!(
            panel.sessions().len(),
            1,
            "the empty live session was skipped"
        );

        // A second conversation stashes beside the first.
        panel.set_composer_text("second question");
        let id = send(&mut panel).expect("send");
        panel.complete_stream(id);
        panel.new_session().expect("stash");
        assert_eq!(panel.sessions().len(), 2);
        assert_eq!(panel.sessions()[1].title, "second question");
    }

    #[test]
    fn switch_session_restores_the_timeline_with_cards() {
        let mut panel = ChatPanel::new();
        // A turn with a parked diff card.
        panel.set_agent_mode(true);
        panel.set_composer_text("edit the file");
        let mut events = Vec::new();
        panel.submit(&mut events);
        let id = match events.pop() {
            Some(ChatPanelEvent::AgentSend { id, .. }) => id,
            other => panic!("expected AgentSend, got {other:?}"),
        };
        panel.tool_call_started(
            id,
            ToolCall {
                id: "call_1".into(),
                name: "edit_file".into(),
                arguments: r#"{"path":"main.rs"}"#.into(),
            },
        );
        let approval_id = ApprovalId::nil();
        panel.approval_needed(
            id,
            approval_id,
            ApprovalCard::Diff {
                path: "main.rs".into(),
                diff: "@@ -1 +1 @@".into(),
            },
        );
        assert_eq!(panel.turn_count(), 2); // user bubble + card
                                           // End the turn the Stop way — the parked card stays pending
                                           // (agent_turn_done would latch it).
        panel.abort_stream(id);
        assert!(
            panel.pending_approval_id().is_some(),
            "the card is still parked"
        );

        // Stash, start fresh, then switch back (index 0, the only
        // stored session).
        panel.new_session().expect("new session");
        assert_eq!(panel.turn_count(), 0);
        assert_eq!(panel.sessions().len(), 1);
        panel.switch_session(0).expect("switch back");

        assert_eq!(panel.turn_count(), 2, "the timeline restored");
        assert_eq!(panel.session_title(), "edit the file");
        assert_eq!(
            panel.pending_approval_id(),
            Some(approval_id),
            "the parked card restored with its approval"
        );
        // The card's tool identity survived the serde mirror.
        assert!(panel.entries.iter().any(
            |entry| matches!(entry, TimelineEntry::Card(card) if card.call.name == "edit_file")
        ));
    }

    #[test]
    fn session_actions_refuse_while_a_turn_runs() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("busy");
        let id = send(&mut panel).expect("send");
        assert!(panel.is_streaming());
        assert!(panel.new_session().is_err(), "new session refused mid-turn");
        assert!(panel.switch_session(0).is_err(), "switch refused mid-turn");
        assert_eq!(panel.turn_count(), 1, "nothing was stashed or dropped");

        panel.complete_stream(id);
        assert!(panel.new_session().is_ok(), "unlocked after the turn");
    }

    #[test]
    fn sessions_snapshot_and_restore_round_trip() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("persisted conversation");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "with an answer");
        panel.complete_stream(id);
        panel.new_session().expect("stash the live one");

        let snapshot = panel.snapshot_sessions();
        assert_eq!(snapshot.len(), 1);

        // A fresh panel restores the same history (the last becomes
        // live).
        let mut restored = ChatPanel::new();
        restored.restore_sessions(snapshot);
        assert_eq!(restored.session_title(), "persisted conversation");
        assert_eq!(restored.turn_count(), 2);
        assert_eq!(restored.sessions().len(), 0, "only the live one came back");
        assert_eq!(restored.snapshot_sessions().len(), 1);
    }

    #[test]
    fn message_copy_copies_the_whole_content() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("go");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "the whole answer");
        panel.complete_stream(id);
        let ctx = egui::Context::default();
        for _ in 0..2 {
            frame(&ctx, egui::RawInput::default(), &mut panel);
        }

        let outcome = click_sweep(&ctx, &mut panel, 200..330, 60..260, 8, |outcome, _| {
            outcome
                .events
                .iter()
                .any(|event| matches!(event, ChatPanelEvent::CopyCode(_)))
        })
        .expect("the copy button was clicked");
        assert!(matches!(
            &outcome.events[0],
            ChatPanelEvent::CopyCode(text) if text == "the whole answer"
        ));
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
        // Phase R2: an empty transcript paints the welcome state — but
        // no bubbles, cards, or code blocks.
        let mut panel = ChatPanel::new();
        let ctx = egui::Context::default();
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
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
            raw_input_with(vec![egui::Event::PointerMoved(block_center)], 0.0),
            &mut panel,
        );
        assert!(
            !outcome.copy_button_rects.is_empty(),
            "hover must reveal the Copy button"
        );
        let copy_center = outcome.copy_button_rects[0].center();

        // Frame 2: click it.
        let outcome = frame(&ctx, raw_input_with(click_at(copy_center), 0.0), &mut panel);
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
            raw_input_with(vec![egui::Event::PointerMoved(block_center)], 0.0),
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
            raw_input_with(click_at(clear_rect.center()), 0.0),
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
            raw_input_with(click_at(copy_rect.center()), 0.0),
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
            raw_input_with(click_at(switch_rect.center()), 0.0),
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
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(target.center()), 0.0),
            &mut panel,
        );

        assert_eq!(outcome.events.len(), 1, "events: {:?}", outcome.events);
        match &outcome.events[0] {
            ChatPanelEvent::SelectModel(id) => assert_eq!(id, "glm-5.3"),
            other => panic!("expected SelectModel, got {other:?}"),
        }
        assert_eq!(panel.model(), Some(&"glm-5.3".to_owned()));
        assert!(!panel.is_picker_open(), "selection closes the picker");
    }

    #[test]
    fn model_picker_refresh_passes_through() {
        let mut panel = ChatPanel::new();
        panel.set_models(vec![model("glm-5.3")]);
        panel.open_model_picker();

        let ctx = egui::Context::default();
        frame(&ctx, egui::RawInput::default(), &mut panel);
        // The dropdown's header row carries the refresh button (its rect
        // reports through the picker-geometry seam).
        let outcome = frame(&ctx, egui::RawInput::default(), &mut panel);
        let _ = outcome;
        let refresh = panel
            .picker_refresh_rect()
            .expect("the refresh button painted");
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(refresh.center()), 0.0),
            &mut panel,
        );

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

    // (The old `parse_segments` parser tests moved to `markdown`'s test
    // suite with the Phase R2 parser — fences, prose splitting, and the
    // incomplete-block case are covered there.)

    // -- misc ---------------------------------------------------------------------------

    #[test]
    fn chat_ids_increase() {
        let a = ChatId::next();
        let b = ChatId::next();
        assert_ne!(a, b);
        assert!(b.as_u64() > a.as_u64());
    }

    // -- Phase 6h presence ------------------------------------------------------------

    /// One panel frame with its `FullOutput` kept (the presence tests
    /// inspect painted shapes; the shared `frame` helper drops them).
    fn paint_panel(ctx: &egui::Context, panel: &mut ChatPanel) -> egui::FullOutput {
        frame_out(ctx, raw_input_with(vec![], 0.0), panel).0
    }

    fn circles(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Circle(_)))
            .count()
    }

    #[test]
    fn the_streaming_bubble_paints_the_thinking_orb() {
        // The stream opened but no text landed yet: the ThinkingOrb
        // (three `working` dots) stands where the ⠿⠿⠿ wave stood.
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "");
        let output = paint_panel(&egui::Context::default(), &mut panel);
        assert!(
            circles(&output) >= 3,
            "the working orb's three dots: {}",
            circles(&output)
        );
    }

    #[test]
    fn reduce_motion_freezes_the_streaming_orb_to_a_dot() {
        let mut panel = ChatPanel::new();
        panel.set_reduce_motion(true);
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        panel.push_assistant_delta(id, "");
        let output = paint_panel(&egui::Context::default(), &mut panel);
        // Below the header (whose own orb freezes to one dot too, §3):
        // exactly the bubble's §7.4 static dot.
        let below_header = output
            .shapes
            .iter()
            .filter(|s| match &s.shape {
                egui::Shape::Circle(c) => c.center.y > HEADER_HEIGHT,
                _ => false,
            })
            .count();
        assert_eq!(below_header, 1, "the §7.4 static dot");
    }

    #[test]
    fn the_status_line_paints_the_avatar_and_the_orb() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_composer_text("work");
        send_agent(&mut panel).expect("agent send");
        let output = paint_panel(&egui::Context::default(), &mut panel);
        // The thinking orb (3 dots) plus the working avatar's two eyes.
        assert!(
            circles(&output) >= 5,
            "orb dots + avatar eyes: {}",
            circles(&output)
        );
    }

    #[test]
    fn a_disconnected_gateway_sleeps_the_status_line_avatar() {
        let mut panel = ChatPanel::new();
        panel.set_agent_mode(true);
        panel.set_gateway_connected(false);
        panel.set_composer_text("work");
        send_agent(&mut panel).expect("agent send");

        let ctx = egui::Context::default();
        // Frame 1 starts the Default → Sleeping cross-fade; frame 2 (a
        // second later) is past it.
        let mut line_rect = Rect::ZERO;
        let mut warm = ctx.run_ui(raw_input_with(vec![], 0.0), |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            line_rect = panel.show(ui).status_line_rect.expect("status line");
        });
        warm.textures_delta.clear();
        let mut output = ctx.run_ui(raw_input_with(vec![], 1.0), |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            line_rect = panel.show(ui).status_line_rect.expect("status line");
        });
        // The sleeping avatar's closed eyes: two horizontal hairlines at
        // the status line's center height, inside the line's left slot.
        let lid_lines = output
            .shapes
            .iter()
            .filter(|s| match &s.shape {
                egui::Shape::LineSegment { points, .. } => {
                    let [a, b] = points;
                    (a.y - b.y).abs() < 0.01
                        && (a.y - line_rect.center().y).abs() < 6.0
                        && a.x >= line_rect.left()
                        && b.x <= line_rect.left() + 40.0
                }
                _ => false,
            })
            .count();
        output.textures_delta.clear();
        assert_eq!(lid_lines, 2, "two closed-eye hairlines in the status line");
    }

    #[test]
    fn the_orb_state_maps_the_loop_phase() {
        use agent_cards::StatusPhase;
        // Thinking without fresh deltas works; with them, connects.
        assert_eq!(
            orb_state_for(&StatusPhase::Thinking, None, false),
            OrbState::Working
        );
        assert_eq!(
            orb_state_for(&StatusPhase::Thinking, None, true),
            OrbState::Connecting
        );
        // Read-type searches; bash/write solves; approvals listen.
        let running = StatusPhase::Running("reading src/main.rs".to_owned());
        assert_eq!(
            orb_state_for(&running, Some("read_file"), false),
            OrbState::Searching
        );
        assert_eq!(
            orb_state_for(&running, Some("bash"), false),
            OrbState::Solving
        );
        assert_eq!(
            orb_state_for(&running, Some("write_file"), false),
            OrbState::Solving
        );
        assert_eq!(
            orb_state_for(&StatusPhase::AwaitingApproval, None, false),
            OrbState::Listening
        );
    }

    #[test]
    fn the_composer_glow_rises_on_deltas_and_sparks_on_completion() {
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hi");
        let id = send(&mut panel).expect("send");
        for _ in 0..8 {
            panel.push_assistant_delta(id, "word ");
        }
        // The attack lands at the next frame's tick (deltas carry no
        // frame clock).
        let output = paint_panel(&egui::Context::default(), &mut panel);
        assert!(panel.stream_glow.level() > 0.0, "deltas raise the glow");
        // The glow band paints along the composer's bottom edge.
        let band = output.shapes.iter().any(|s| match &s.shape {
            egui::Shape::Rect(r) => r.fill != egui::Color32::TRANSPARENT && r.rect.height() <= 6.0,
            _ => false,
        });
        assert!(band, "the glow band paints");
        panel.complete_stream(id);
        // The spark plays on the next frame (and is spent within
        // --dur-long — the widget's own tests cover the envelope).
        assert!(panel.stream_glow.spark_pending());
    }

    // -- REVAMP_SPEC §2–§4: the hero composer, the header orb, the welcome ------

    /// The rect shapes of one frame (the chrome assertions' view).
    fn rect_shapes(output: &egui::FullOutput) -> Vec<&egui::epaint::RectShape> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// The glow rings of one frame: stroked rects concentric with and
    /// larger than the card, sorted inside-out.
    fn ring_shapes(output: &egui::FullOutput, card: Rect) -> Vec<&egui::epaint::RectShape> {
        let mut rings: Vec<&egui::epaint::RectShape> = rect_shapes(output)
            .into_iter()
            .filter(|r| {
                r.stroke.width > 0.0
                    && r.rect.width() > card.width()
                    && r.rect.height() > card.height()
                    && (r.rect.center().x - card.center().x).abs() < 0.01
                    && (r.rect.center().y - card.center().y).abs() < 0.01
            })
            .collect();
        rings.sort_by(|a, b| a.rect.width().total_cmp(&b.rect.width()));
        rings
    }

    /// The frame's requested repaint delay (`Duration::MAX` = none).
    fn repaint_delay(output: &egui::FullOutput) -> Duration {
        output
            .viewport_output
            .values()
            .next()
            .map(|o| o.repaint_delay)
            .unwrap_or(Duration::MAX)
    }

    /// The welcome chips' fill shapes (chip-sized: 28px tall at rest,
    /// shrinking to 0.96× mid-bloom; wider than the footer widgets).
    fn chip_fills(output: &egui::FullOutput) -> Vec<egui::epaint::RectShape> {
        rect_shapes(output)
            .into_iter()
            .filter(|r| {
                r.fill != egui::Color32::TRANSPARENT
                    && r.rect.width() > 100.0
                    && (24.0..=29.0).contains(&r.rect.height())
            })
            .cloned()
            .collect()
    }

    /// Circles in `--ok`'s hue. egui's `Color32` is premultiplied, so the
    /// rgb channels scale with alpha — compare the hue ratio (the
    /// `liquid.rs` test precedent), not the raw channels.
    fn ok_hue_circles(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|s| {
                matches!(&s.shape, egui::Shape::Circle(c)
                    if c.fill.a() > 0 && c.fill.g() > c.fill.r() && c.fill.g() > c.fill.b())
            })
            .count()
    }

    #[test]
    fn composer_card_paints_its_chrome_at_rest() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        panel.set_composer_text("hello");
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        let card = outcome.composer_card_rect;
        assert!(card.width() > 0.0, "the card rect is reported");

        // The chrome: one `--ink-low` fill + a 1px `--ink-hi` inside
        // stroke at the card rect, `--radius-2` corners (§2.1).
        let chrome: Vec<_> = rect_shapes(&output)
            .into_iter()
            .filter(|r| r.rect == card)
            .collect();
        assert_eq!(chrome.len(), 1, "exactly one shape at the card rect");
        assert_eq!(chrome[0].fill, color::INK_LOW);
        assert_eq!(chrome[0].stroke, egui::Stroke::new(1.0, color::INK_HI));
        assert_eq!(
            chrome[0].corner_radius,
            egui::CornerRadius::same(radius::R2 as u8)
        );

        // The bar height is the §2.1 formula computed from the measured
        // pitch (never a hardcoded total): input_row + 80.
        let (rows, pitch) = {
            let mut measured = (0usize, 0.0f32);
            let mut out = ctx.run_ui(raw_input_with(vec![], 0.2), |ui| {
                ui.set_min_size(vec2(360.0, 600.0));
                measured = panel.composer_rows(ui.painter(), ui.available_width());
            });
            out.textures_delta.clear();
            measured
        };
        assert!(
            (16.0..=22.0).contains(&pitch),
            "the measured --type-md pitch: {pitch}"
        );
        let input_row = (rows as f32 * pitch).max(SEND_BUTTON_SIZE.y);
        let expected_bar = input_row
            + 2.0 * space::S3
            + 2.0 * space::S3
            + space::S2
            + model_picker::CLOSED_BUTTON_HEIGHT;
        let mut out = ctx.run_ui(raw_input_with(vec![], 0.3), |ui| {
            ui.set_min_size(vec2(360.0, 600.0));
            let height = panel.composer_height(ui);
            assert!(
                (height - expected_bar).abs() < 1e-4,
                "bar {height} == the formula's {expected_bar}"
            );
        });
        out.textures_delta.clear();
        // The painted card is the bar minus the 12px margins.
        assert!(
            (card.height() + 2.0 * space::S3 - expected_bar).abs() < 1e-4,
            "card {} + margins == bar {expected_bar}",
            card.height()
        );
    }

    #[test]
    fn focused_composer_paints_five_glow_rings() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        panel.focus_composer();
        frame(&ctx, raw_input_with(vec![], 0.0), &mut panel); // grants focus
        frame(&ctx, raw_input_with(vec![], 1.0), &mut panel); // focused: the fade starts
                                                              // Past --dur-quick the rings are fully in.
        let (output, outcome) = frame_out(
            &ctx,
            raw_input_with(vec![], 1.0 + RING_FADE_S as f64 + 0.05),
            &mut panel,
        );
        let card = outcome.composer_card_rect;

        let rings = ring_shapes(&output, card);
        assert_eq!(rings.len(), GLOW_RING_ALPHAS.len(), "five glow rings");
        for (i, ring) in rings.iter().enumerate() {
            let outset = GLOW_RING_STEP * (i as f32 + 1.0);
            assert_eq!(
                ring.rect,
                card.expand(outset),
                "ring {i} expands by 2·(i+1)"
            );
            assert_eq!(ring.stroke.width, 1.0);
            assert_eq!(
                ring.stroke.color,
                with_alpha(color::FG_FAINT, GLOW_RING_ALPHAS[i]),
                "ring {i}'s stepped alpha"
            );
            assert_eq!(
                ring.corner_radius,
                egui::CornerRadius::same((radius::R2 + outset) as u8),
                "ring {i}'s corner radius grows with the expansion"
            );
        }
        // The card hairline lifts to --ink-rule while focused.
        let chrome: Vec<_> = rect_shapes(&output)
            .into_iter()
            .filter(|r| r.rect == card)
            .collect();
        assert_eq!(
            chrome[0].stroke,
            egui::Stroke::new(1.0, color::INK_RULE),
            "the focused hairline"
        );
        // Ring 5's outer edge (outset 10 + a centered stroke's 0.5) stays
        // inside the 12px card margins — the rings never meet the panel's
        // clip bounds.
        let bar = card.expand(space::S3);
        assert!(
            bar.contains_rect(rings[GLOW_RING_ALPHAS.len() - 1].rect.expand(0.5)),
            "ring 5 paints at most 10.5px past the card"
        );
    }

    #[test]
    fn hover_without_focus_lifts_only_the_hairline() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        let (_, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        let card = outcome.composer_card_rect;
        // The pointer rests on the card's padding (over no child widget);
        // the edit never took focus.
        let hover = pos2(card.left() + 2.0, card.top() + 2.0);
        let (output, outcome) = frame_out(
            &ctx,
            raw_input_with(vec![egui::Event::PointerMoved(hover)], 0.1),
            &mut panel,
        );
        let card = outcome.composer_card_rect;
        let chrome: Vec<_> = rect_shapes(&output)
            .into_iter()
            .filter(|r| r.rect == card)
            .collect();
        assert_eq!(chrome.len(), 1);
        assert_eq!(
            chrome[0].stroke,
            egui::Stroke::new(1.0, color::INK_RULE),
            "hover lifts the hairline"
        );
        assert!(
            ring_shapes(&output, card).is_empty(),
            "no rings without focus"
        );
    }

    #[test]
    fn disconnected_focus_paints_rings_but_no_shimmer() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        panel.set_gateway_connected(false);
        panel.focus_composer();
        frame(&ctx, raw_input_with(vec![], 0.0), &mut panel); // grants focus
        frame(&ctx, raw_input_with(vec![], 1.0), &mut panel); // focused
        let (output, outcome) = frame_out(
            &ctx,
            raw_input_with(vec![], 1.0 + RING_FADE_S as f64 + 0.05),
            &mut panel,
        );
        let card = outcome.composer_card_rect;
        assert_eq!(
            ring_shapes(&output, card).len(),
            GLOW_RING_ALPHAS.len(),
            "the rings paint regardless of the gateway (§2.2)"
        );
        // No ready-shimmer band along the card's bottom edge (the
        // avatars-sleep precedent): no wide hairline-height fill there.
        let band = rect_shapes(&output).into_iter().any(|r| {
            r.fill != egui::Color32::TRANSPARENT
                && r.rect.height() <= 1.5
                && r.rect.width() > card.width() / 2.0
                && (r.rect.bottom() - card.bottom()).abs() < 1.0
        });
        assert!(!band, "disconnected: no shimmer band");
    }

    #[test]
    fn send_button_arms_ink_mid() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();

        // Empty buffer: no fill, the disabled glyph.
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        let send = outcome.send_button_rect;
        assert!(
            rect_shapes(&output)
                .iter()
                .all(|r| r.rect != send || r.fill == egui::Color32::TRANSPARENT),
            "empty buffer: the send button has no fill"
        );
        let glyph = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if send.contains(t.pos) => Some(t),
                _ => None,
            })
            .expect("the ⏎ glyph paints");
        assert_eq!(glyph.fallback_color, color::FG_DISABLED);

        // Text in the buffer: armed at --ink-mid...
        panel.set_composer_text("hi");
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.1), &mut panel);
        let send = outcome.send_button_rect;
        let fill = rect_shapes(&output)
            .into_iter()
            .find(|r| r.rect == send && r.fill != egui::Color32::TRANSPARENT)
            .map(|r| r.fill);
        assert_eq!(fill, Some(color::INK_MID), "the armed send button");
        let glyph = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if send.contains(t.pos) => Some(t),
                _ => None,
            })
            .expect("the ⏎ glyph paints");
        assert_eq!(glyph.fallback_color, color::FG_DEFAULT);

        // ...and --ink-hi on hover.
        let (output, outcome) = frame_out(
            &ctx,
            raw_input_with(vec![egui::Event::PointerMoved(send.center())], 0.2),
            &mut panel,
        );
        let send = outcome.send_button_rect;
        let fill = rect_shapes(&output)
            .into_iter()
            .find(|r| r.rect == send && r.fill != egui::Color32::TRANSPARENT)
            .map(|r| r.fill);
        assert_eq!(fill, Some(color::INK_HI), "the hovered send button");
    }

    #[test]
    fn frameless_edit_paints_no_own_frame() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        panel.focus_composer();
        frame(&ctx, raw_input_with(vec![], 0.0), &mut panel); // grants focus
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 1.0), &mut panel);
        let card = outcome.composer_card_rect;

        // Exactly one rounded stroke at the card rect (the card's own
        // chrome; the rings live outside it).
        let at_card = rect_shapes(&output)
            .into_iter()
            .filter(|r| r.rect == card && r.stroke.width > 0.0)
            .count();
        assert_eq!(at_card, 1, "only the card's hairline at the card rect");
        // No second `--ink-low` fill at the edit's scale inside the card
        // (egui's own edit frame would be extreme_bg = --ink-low at the
        // edit's rect; the pill's footer-sized fill stays excluded).
        let edit_fills = rect_shapes(&output)
            .into_iter()
            .filter(|r| {
                r.fill == color::INK_LOW
                    && card.contains_rect(r.rect)
                    && r.rect != card
                    && r.rect.width() > 200.0
                    && r.rect.height() < 40.0
            })
            .count();
        assert_eq!(edit_fills, 0, "the edit paints no own background");
        // No egui-native focus stroke inside the card at the edit's scale
        // (the chip/pill hairlines are footer-sized).
        let inner_strokes = rect_shapes(&output)
            .into_iter()
            .filter(|r| {
                r.stroke.width > 0.0
                    && r.rect != card
                    && card.contains_rect(r.rect)
                    && r.rect.width() > 200.0
            })
            .count();
        assert_eq!(inner_strokes, 0, "the frameless edit strokes nothing");
    }

    #[test]
    fn reduce_motion_composer_requests_no_frames() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        panel.set_reduce_motion(true);
        panel.focus_composer();
        frame(&ctx, raw_input_with(vec![], 0.0), &mut panel); // grants focus
                                                              // The first focused frame: the rings snap on at full alpha (no
                                                              // --dur-quick fade — §11 bend #5).
        let (focus_frame, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.1), &mut panel);
        let card = outcome.composer_card_rect;
        let rings = ring_shapes(&focus_frame, card);
        assert_eq!(rings.len(), GLOW_RING_ALPHAS.len(), "the rings snapped on");
        for (i, ring) in rings.iter().enumerate() {
            assert_eq!(
                ring.stroke.color,
                with_alpha(color::FG_FAINT, GLOW_RING_ALPHAS[i]),
                "ring {i} at full alpha on the focus frame"
            );
        }
        // A later focused frame paints the identical rings (settled).
        let ring_key = |rings: &[&egui::epaint::RectShape]| {
            rings.iter().map(|r| (r.rect, r.stroke)).collect::<Vec<_>>()
        };
        let (later, _) = frame_out(&ctx, raw_input_with(vec![], 0.3), &mut panel);
        assert_eq!(
            ring_key(&rings),
            ring_key(&ring_shapes(&later, card)),
            "snapped: identical shapes"
        );

        // Steady state, nothing focused: under reduce-motion nothing
        // requests a frame. (A focused edit's text-cursor blink is egui's
        // own scheduler — the UI_SPEC §3.2 exemption — so this is
        // measured on an unfocused panel, on a fresh context: the first
        // panel's focus memory must not leak in.)
        let calm_ctx = egui::Context::default();
        let mut calm = ChatPanel::new();
        calm.set_reduce_motion(true);
        for i in 0..3 {
            frame(
                &calm_ctx,
                raw_input_with(vec![], 1.0 + i as f64 * 0.1),
                &mut calm,
            );
        }
        let (steady, _) = frame_out(&calm_ctx, raw_input_with(vec![], 1.5), &mut calm);
        assert_eq!(
            repaint_delay(&steady),
            Duration::MAX,
            "reduce-motion requests no frames"
        );
    }

    #[test]
    fn live_idle_chat_requests_animation_frames() {
        // The §9 standing-cost lock: an idle open chat panel repaints at
        // the ANIM_FRAME cadence (the header orb + the welcome hero
        // breathe) — deliberate, and disconnected/reduce-motion are the
        // bounds (covered by their own tests).
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        for i in 0..3 {
            frame(&ctx, raw_input_with(vec![], i as f64 * 0.1), &mut panel);
        }
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 0.4), &mut panel);
        let anim_frame = Duration::from_millis(motion::DUR_LOOP_MS as u64 / 60);
        assert!(
            repaint_delay(&output) <= anim_frame,
            "idle live chat repaints at the presence cadence: {:?}",
            repaint_delay(&output)
        );
    }

    #[test]
    fn header_orb_matches_the_status_line_mapping() {
        use agent_cards::StatusPhase;
        // The §3 table (agent in flight): the status line's own mapping.
        assert_eq!(
            header_orb(
                true,
                true,
                &StatusPhase::AwaitingApproval,
                None,
                false,
                true,
                false
            )
            .state(),
            OrbState::Listening,
            "parked on an approval"
        );
        assert_eq!(
            header_orb(
                true,
                true,
                &StatusPhase::Running("running cargo test".to_owned()),
                Some("bash"),
                false,
                true,
                false
            )
            .state(),
            OrbState::Solving,
            "running bash"
        );
        assert_eq!(
            header_orb(
                true,
                true,
                &StatusPhase::Running("reading src/main.rs".to_owned()),
                Some("read_file"),
                false,
                true,
                false
            )
            .state(),
            OrbState::Searching,
            "running a read tool"
        );
        assert_eq!(
            header_orb(true, true, &StatusPhase::Thinking, None, true, true, false).state(),
            OrbState::Connecting,
            "thinking while deltas stream"
        );
        assert_eq!(
            header_orb(true, true, &StatusPhase::Thinking, None, false, true, false).state(),
            OrbState::Working,
            "thinking between deltas"
        );
        // Plain (non-agent) streams: connecting while deltas flow,
        // working while the model thinks.
        assert_eq!(
            header_orb(false, true, &StatusPhase::Thinking, None, true, true, false).state(),
            OrbState::Connecting
        );
        assert_eq!(
            header_orb(
                false,
                true,
                &StatusPhase::Thinking,
                None,
                false,
                true,
                false
            )
            .state(),
            OrbState::Working
        );
        // Idle: breathing.
        assert_eq!(
            header_orb(
                false,
                false,
                &StatusPhase::Thinking,
                None,
                false,
                true,
                false
            )
            .state(),
            OrbState::Breathing
        );
        // Disconnected: paused on the MAPPED state (a paused `Working`
        // still paints its three dots, statically).
        let paused = header_orb(
            false,
            true,
            &StatusPhase::Thinking,
            None,
            false,
            false,
            false,
        );
        assert_eq!(paused.state(), OrbState::Working);

        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        panel.set_gateway_connected(false);
        panel.set_composer_text("hi");
        let _id = send(&mut panel).expect("send"); // plain in-flight, no deltas: Working
        frame(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        frame(&ctx, raw_input_with(vec![], 0.5), &mut panel);
        let header_dots = |output: &egui::FullOutput| {
            let mut dots: Vec<(i32, i32, i32, egui::Color32)> = output
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Circle(c) if c.center.y < HEADER_HEIGHT => Some((
                        (c.center.x * 100.0) as i32,
                        (c.center.y * 100.0) as i32,
                        (c.radius * 100.0) as i32,
                        c.fill,
                    )),
                    _ => None,
                })
                .collect();
            dots.sort_by_key(|a| (a.0, a.1, a.2));
            dots
        };
        let (a, _) = frame_out(&ctx, raw_input_with(vec![], 1.0), &mut panel);
        let (b, _) = frame_out(&ctx, raw_input_with(vec![], 2.0), &mut panel);
        assert_eq!(header_dots(&a).len(), 3, "the paused Working orb's dots");
        assert_eq!(header_dots(&a), header_dots(&b), "paused: frozen in place");
        // Disconnected pauses the orbs (§9's bound); the frames that
        // remain are the in-flight beam's — §8.6 (owner's, verbatim): "a
        // disconnect mid-flight does not freeze the beam (in-flight
        // requests complete)". The beam rides at the ANIM_FRAME cadence
        // (§8.3), which bounds the delay. (egui subtracts the predicted
        // frame time from every delayed request, so the observed delay
        // lands just under ANIM_FRAME.)
        let anim_frame = Duration::from_millis(motion::DUR_LOOP_MS as u64 / 60);
        assert!(
            repaint_delay(&b) <= anim_frame,
            "disconnected pauses the orbs — only the in-flight beam requests frames (§8.6/§9): {:?}",
            repaint_delay(&b)
        );
    }

    #[test]
    fn welcome_hero_is_the_64px_breathing_orb() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        // The hero's breathing dot (radius 0.12 · 64 · (1..1.08) ≈ 7.7–8.3)
        // — the 20px header orb's dot is a third of that.
        let hero_dots = output
            .shapes
            .iter()
            .filter(
                |s| matches!(&s.shape, egui::Shape::Circle(c) if (6.0..=10.0).contains(&c.radius)),
            )
            .count();
        assert_eq!(hero_dots, 1, "the 64px hero's breathing dot");
        // Connected: the `--ok` liveness satellite paints (§4 contract #2).
        assert_eq!(ok_hue_circles(&output), 1, "the liveness satellite");
        // Liveness off: the satellite is gone, the hero keeps breathing.
        panel.set_gateway_connected(false);
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 0.1), &mut panel);
        assert_eq!(ok_hue_circles(&output), 0, "liveness(false): no satellite");
        let hero_dots = output
            .shapes
            .iter()
            .filter(
                |s| matches!(&s.shape, egui::Shape::Circle(c) if (6.0..=10.0).contains(&c.radius)),
            )
            .count();
        assert_eq!(hero_dots, 1, "the hero still breathes");
    }

    #[test]
    fn welcome_chips_bloom_in_three_staggered_steps() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        // The anchor arms on the first empty frame (t=1.0): every chip is
        // still at alpha 0.
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 1.0), &mut panel);
        assert!(chip_fills(&output).is_empty(), "the bloom starts at zero");
        // 40ms later: chip 0 is mid-bloom; chips 1 and 2 haven't started.
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 1.04), &mut panel);
        let chips = chip_fills(&output);
        assert_eq!(chips.len(), 1, "only chip 0 has started");
        let alpha = chips[0].fill.a();
        assert!(alpha > 0 && alpha < 255, "chip 0 mid-bloom (alpha {alpha})");
        // Well past the last chip's end: all three settled at full alpha.
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 2.0), &mut panel);
        let chips = chip_fills(&output);
        assert_eq!(chips.len(), 3, "all three chips bloomed");
        assert!(chips.iter().all(|c| c.fill.a() == 255));

        // A turn pushed and cleared re-arms the entrance from the top.
        panel.push_user("hi");
        frame(&ctx, raw_input_with(vec![], 2.5), &mut panel);
        panel.clear();
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 3.0), &mut panel);
        assert!(
            chip_fills(&output).is_empty(),
            "the re-emptied panel re-runs the entrance"
        );
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 3.04), &mut panel);
        assert_eq!(chip_fills(&output).len(), 1, "and it staggers again");
    }

    #[test]
    fn welcome_chips_paint_the_1px_border() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        frame(&ctx, raw_input_with(vec![], 0.0), &mut panel); // arm the anchor
                                                              // Past the bloom: the settled chips carry the §4 chrome — an
                                                              // `--ink-low` fill with a 1px `--ink-hi` border.
        let (output, _) = frame_out(&ctx, raw_input_with(vec![], 1.0), &mut panel);
        let chips: Vec<_> = rect_shapes(&output)
            .into_iter()
            .filter(|r| {
                (27.5..=28.5).contains(&r.rect.height())
                    && r.rect.width() > 100.0
                    && r.stroke.width > 0.0
            })
            .collect();
        assert_eq!(chips.len(), SUGGESTIONS.len(), "three chipped suggestions");
        for chip in chips {
            assert_eq!(chip.fill, color::INK_LOW);
            assert_eq!(chip.stroke, egui::Stroke::new(1.0, color::INK_HI));
            assert_eq!(
                chip.corner_radius,
                egui::CornerRadius::same(radius::R1 as u8)
            );
        }
    }

    /// One frame at a custom panel width (the footer's fit-or-hide test)
    /// — `frame_out`'s shape with a wider/narrower surface.
    fn frame_wide(
        ctx: &egui::Context,
        width: f32,
        panel: &mut ChatPanel,
    ) -> (egui::FullOutput, ChatPanelOutcome) {
        crate::theme::set_fonts(ctx);
        let input = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0))),
            time: Some(0.0),
            ..Default::default()
        };
        let mut outcome = ChatPanelOutcome::default();
        let mut output = ctx.run_ui(input, |ui| {
            ui.set_min_size(vec2(width, 600.0));
            outcome = panel.show(ui);
        });
        output.textures_delta.clear();
        (output, outcome)
    }

    /// The composer card's footer row (the bottom 24px inside the card
    /// padding, §2.1).
    fn composer_footer(card: Rect) -> Rect {
        Rect::from_min_size(
            pos2(
                card.left() + space::S3,
                card.bottom() - space::S3 - model_picker::CLOSED_BUTTON_HEIGHT,
            ),
            vec2(
                card.width() - 2.0 * space::S3,
                model_picker::CLOSED_BUTTON_HEIGHT,
            ),
        )
    }

    /// Whether the key hint painted in this frame.
    fn hint_painted(output: &egui::FullOutput) -> bool {
        output.shapes.iter().any(
            |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == COMPOSER_KEY_HINT),
        )
    }

    #[test]
    fn footer_layout_and_hint_fit_or_hide() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new(); // no model: the shortest chip label

        // 360px default: the cluster (chip + --space-2 + pill) paints
        // inside the footer; the hint stays hidden — the fit-or-hide
        // rule needs ~357px of card inner width for this cluster + hint
        // (measured on the bundled Space Grotesk) and 360px has 312.
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        let footer = composer_footer(outcome.composer_card_rect);
        let chip = panel.picker.last_geometry().toggle;
        assert!(chip.width() > 0.0, "the model chip paints");
        assert!(footer.contains_rect(chip), "the chip sits in the footer");
        let pill = outcome.agent_pill_rect;
        assert!(
            footer.contains_rect(pill),
            "the agent pill sits in the footer"
        );
        assert!(
            (pill.left() - (chip.right() + space::S2)).abs() < 0.01,
            "the pill follows the chip by --space-2 (pins the mirrored width formula)"
        );
        assert!(
            !hint_painted(&output),
            "360px: the hint hides whole (never elided)"
        );

        // 240px minimum: the hint still hides; the cluster is intact.
        let (output, outcome) = frame_wide(&ctx, 240.0, &mut panel);
        let footer = composer_footer(outcome.composer_card_rect);
        assert!(!hint_painted(&output), "240px: the hint hides");
        let chip = panel.picker.last_geometry().toggle;
        assert!(footer.contains_rect(chip), "240px: the chip holds");
        assert!(
            footer.contains_rect(outcome.agent_pill_rect),
            "240px: the pill holds"
        );

        // 440px: the footer genuinely fits — the hint paints,
        // right-aligned in the row.
        let (output, outcome) = frame_wide(&ctx, 440.0, &mut panel);
        let footer = composer_footer(outcome.composer_card_rect);
        let hint = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == COMPOSER_KEY_HINT => Some(t),
                _ => None,
            })
            .expect("440px: the hint paints");
        assert_eq!(hint.fallback_color, color::FG_FAINT);
        assert!(
            (hint.pos.x + hint.galley.size().x - footer.right()).abs() < 0.5,
            "the hint is right-aligned in the footer"
        );
        assert!(
            (hint.pos.y + hint.galley.size().y / 2.0 - footer.center().y).abs() < 0.5,
            "and vertically centered in it"
        );
    }

    // -- the border beam (REVAMP_SPEC §8; the §2.3 composer hook) --------

    /// The border beam's segments in one frame: 2px line segments of a
    /// gray (`--fg-default` hue — premultiplied alpha keeps r == g == b)
    /// whose endpoints hug the card's perimeter. The StreamGlow's own 2px
    /// segments ride the bottom edge's interior band (and its spark is
    /// `--ok`-green), so neither passes the perimeter + hue filter; the
    /// icons' 1.5px strokes fail the width.
    fn beam_segments(output: &egui::FullOutput, card: Rect) -> Vec<(egui::Pos2, egui::Pos2)> {
        // A point on the rounded-rect path is on the rect's outline —
        // never deeper than the corner arc (R2 · (1 − 1/√2) ≈ 1.17px).
        let on_perimeter =
            |p: egui::Pos2| card.distance_to_pos(p) <= 1.5 && !card.shrink(1.5).contains(p);
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 2.0
                        && stroke.color.r() == stroke.color.g()
                        && stroke.color.g() == stroke.color.b()
                        && stroke.color.a() > 0
                        && on_perimeter(points[0])
                        && on_perimeter(points[1]) =>
                {
                    Some((points[0], points[1]))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn in_flight_composer_paints_the_beam() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();

        // Idle: the card's perimeter carries no beam (§8.6).
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.0), &mut panel);
        let card = outcome.composer_card_rect;
        assert!(
            beam_segments(&output, card).is_empty(),
            "idle: no beam on the card's perimeter"
        );

        // In flight (a plain stream): 2px `--fg-default`-hue segments
        // ride the perimeter.
        panel.begin_stream(ChatId::next());
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.2), &mut panel);
        let card = outcome.composer_card_rect;
        let segments = beam_segments(&output, card);
        assert!(
            !segments.is_empty(),
            "in flight: the beam rides the card's perimeter"
        );
        // The lap advances: a frame 600ms later paints the arc elsewhere
        // (the eased phase's authority is beam.rs's own test).
        let (later, _) = frame_out(&ctx, raw_input_with(vec![], 0.8), &mut panel);
        let moved = beam_segments(&later, card);
        assert!(!moved.is_empty(), "the beam still rides mid-lap");
        assert_ne!(segments, moved, "the beam travels over 600ms");
    }

    #[test]
    fn beam_stops_when_the_stream_completes() {
        let ctx = egui::Context::default();
        let mut panel = ChatPanel::new();
        let id = ChatId::next();
        panel.begin_stream(id);
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.2), &mut panel);
        assert!(
            !beam_segments(&output, outcome.composer_card_rect).is_empty(),
            "in flight: the beam rides"
        );

        panel.complete_stream(id);
        let (output, outcome) = frame_out(&ctx, raw_input_with(vec![], 0.4), &mut panel);
        assert!(
            beam_segments(&output, outcome.composer_card_rect).is_empty(),
            "the beam dies with the stream"
        );
        // No repaint assertion here: `complete_stream` fires the
        // StreamGlow's one-shot `--ok` spark (`--dur-long`) and the
        // connected header orb breathes regardless (§9) — only the beam
        // shapes' absence is this test's business.
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
        frame(
            &ctx,
            raw_input_with(click_at(pill.center()), 0.0),
            &mut panel,
        );
        assert!(panel.is_agent_mode(), "the pill click toggles on");
        frame(
            &ctx,
            raw_input_with(click_at(pill.center()), 0.0),
            &mut panel,
        );
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
            raw_input_with(click_at(deny_rect.center()), 0.0),
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
            raw_input_with(click_at(deny_rect.center()), 0.0),
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
        let outcome = frame(&ctx, raw_input_with(click_at(stop), 0.0), &mut panel);
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
        let outcome = frame(&ctx, raw_input_with(click_at(stop), 0.0), &mut panel);
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
            raw_input_with(click_at(clear_rect.center()), 0.0),
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
        let outcome = frame(
            &ctx,
            raw_input_with(click_at(approve.center()), 0.0),
            &mut panel,
        );
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
