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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, Key, Rect, Sense, Ui, Vec2};

use selora_ai::{ChatMessage, Model, ModelId, Role};

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
    /// The transcript (user, assistant, and the streaming bubble).
    messages: Vec<Message>,
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
            messages: Vec::new(),
            composer: String::new(),
            want_focus: false,
            in_flight: None,
            model: None,
            models: Vec::new(),
            picker_open: false,
            menu_open: false,
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

    /// Number of transcript entries (including the streaming bubble).
    pub fn turn_count(&self) -> usize {
        self.messages.len()
    }

    /// The full transcript as wire-ready [`ChatMessage`]s for the next
    /// `ChatRequest`. The in-progress streaming bubble (if any) is
    /// excluded — it cannot be part of its own request — and so are
    /// failed turns (the shell surfaces those as toasts, not context).
    pub fn conversation(&self) -> Vec<ChatMessage> {
        self.messages
            .iter()
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
    /// stable state and are skipped. The shell serializes whatever this
    /// returns.
    pub fn snapshot_turns(&self) -> Vec<ChatTurn> {
        let stable: Vec<&Message> = self
            .messages
            .iter()
            .filter(|message| !message.streaming && message.error.is_none())
            .collect();
        let start = stable.len().saturating_sub(MAX_PERSISTED_TURNS);
        stable[start..]
            .iter()
            .map(|message| message.completed_turn())
            .collect()
    }

    /// Replace the transcript from persisted turns (app boot, before the
    /// first frame). Restored turns are all complete; nothing streams.
    pub fn restore_turns(&mut self, turns: Vec<ChatTurn>) {
        self.messages = turns.into_iter().map(turn_into_message).collect();
        self.in_flight = None;
    }

    /// The transcript as a plain-text transcript ("You: …" /
    /// "Selora: …" blocks) — the overflow menu's "Copy transcript"
    /// payload.
    pub fn transcript(&self) -> String {
        self.messages
            .iter()
            .map(|message| format!("{}: {}", role_label(message.role), message.content.trim()))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    // -- streaming lifecycle (the shell drives these) --------------------------

    /// Append a user bubble (programmatic injection — the Enter path
    /// already echoes its own; do not call both for one message).
    pub fn push_user(&mut self, text: &str) {
        self.messages.push(Message::user(text));
    }

    /// Feed one streamed delta into the assistant bubble (from
    /// `AppMessage::ChatDelta`). Stale ids — anything but the in-flight
    /// request — are dropped. The first delta opens the bubble and
    /// starts its fade-in.
    pub fn push_assistant_delta(&mut self, id: ChatId, delta: &str) {
        if self.in_flight != Some(id) {
            return;
        }
        let now = Instant::now();
        let opens_bubble = !self
            .messages
            .last()
            .is_some_and(|message| message.streaming && message.role == Role::Assistant);
        if opens_bubble {
            self.messages.push(Message {
                role: Role::Assistant,
                content: delta.to_owned(),
                streaming: true,
                stream_started: Some(now),
                last_delta: Some(now),
                error: None,
            });
            return;
        }
        let message = self.messages.last_mut().expect("checked above");
        message.content.push_str(delta);
        message.last_delta = Some(now);
    }

    /// End the stream (from `AppMessage::ChatDone`): the bubble stops
    /// streaming; a bubble that never received a delta is dropped.
    pub fn complete_stream(&mut self, id: ChatId) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        if let Some(message) = self.messages.last_mut() {
            message.streaming = false;
        }
        let is_empty_assistant = self
            .messages
            .last()
            .is_some_and(|message| message.role == Role::Assistant && message.content.is_empty());
        if is_empty_assistant {
            self.messages.pop();
        }
    }

    /// Fail the stream (from `AppMessage::ChatError`): the error is kept
    /// on the bubble and rendered under it; the composer unlocks.
    pub fn fail_stream(&mut self, id: ChatId, error: &str) {
        if self.in_flight != Some(id) {
            return;
        }
        self.in_flight = None;
        match self.messages.last_mut() {
            Some(message) if message.role == Role::Assistant => {
                message.streaming = false;
                message.error = Some(error.to_owned());
            }
            _ => self.messages.push(Message {
                role: Role::Assistant,
                content: String::new(),
                streaming: false,
                stream_started: None,
                last_delta: None,
                error: Some(error.to_owned()),
            }),
        }
    }

    /// Empty the transcript (the overflow menu's "Clear conversation").
    /// The shell still gets the [`ChatPanelEvent::Clear`] event and is
    /// responsible for cancelling any in-flight stream.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.in_flight = None;
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

    /// The conversation scroll: bubbles, code blocks, streaming visuals.
    fn paint_conversation(&mut self, ui: &mut Ui, frame: &mut FrameArtifacts) {
        let messages = &self.messages;
        egui::ScrollArea::vertical()
            .id_salt("selora_chat_transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |scroll| {
                scroll.add_space(AREA_PADDING);
                let content_width = scroll.available_width();
                for (index, message) in messages.iter().enumerate() {
                    paint_message(scroll, message, index, content_width, frame);
                    scroll.add_space(BUBBLE_GAP);
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
    /// the input, emit [`ChatPanelEvent::Send`]. A no-op while empty or
    /// mid-stream.
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
        events.push(ChatPanelEvent::Send { id, text });
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
            panel.messages.last().and_then(|m| m.error.as_deref()),
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
        assert!(panel
            .messages
            .last()
            .is_some_and(|m| m.error.as_deref() == Some("key invalid")));
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
        panel.messages.push(turn_into_message(ChatTurn::assistant(
            "here:\n\n```rust\nfn main() {}\n```\n",
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
        panel
            .messages
            .push(turn_into_message(ChatTurn::assistant("```\nfn partial(\n")));

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
}
