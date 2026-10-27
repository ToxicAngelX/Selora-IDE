//! Durable agent sessions (ARCHITECTURE.md Â§19.1, ROADMAP.md Phase 9d).
//!
//! A session is the unit the agent works in: a transcript, a todo list, a
//! mode, a plan, parked cards, the model, and the grown conversation. This
//! module owns the on-disk half:
//!
//! ```text
//! .selora/agent-sessions/{session_id}/
//!   transcript.jsonl   user / assistant / tool / tool_calls records
//!   todos.json         the todo list
//!   state.json         title, model, mode, plan, open parks, compaction
//! ```
//!
//! Invariants:
//! - the transcript is append-only while a turn runs (one `sync_data()` per
//!   record); a torn tail is ignored on load and truncated before the next
//!   append, never guessed at;
//! - the full transcript is preserved across compaction â€” compaction writes
//!   a cursor to `state.json`, it never rewrites history;
//! - the session auto-approve flag has no field here, by construction: trust
//!   is session/process state only, so it cannot be restored from disk;
//! - the API key is never a field. A store built with
//!   [`AgentSessionStore::with_redaction_key`] additionally scrubs the key
//!   from every string it writes (defence in depth for model-echoed text).

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use selora_core::SeloraError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use super::turn::ChatStream;
use super::ApprovalCard;
use crate::chat_types::ToolCall;
use crate::types::{ChatEvent, ChatMessage, ChatRequest, Role};

/// Root of the durable session store, workspace-relative.
pub const AGENT_SESSIONS_DIR: &str = ".selora/agent-sessions";
/// The append-only transcript file.
pub const TRANSCRIPT_FILE: &str = "transcript.jsonl";
/// The todo list file.
pub const TODOS_FILE: &str = "todos.json";
/// The session metadata file.
pub const STATE_FILE: &str = "state.json";
/// The on-disk schema. A bump refuses to load older stores.
pub const SESSION_SCHEMA: u32 = 1;

/// Compaction fires at this fraction of the context budget (Â§19.5: ~80%).
pub const COMPACTION_THRESHOLD: f64 = 0.80;
/// The default number of recent messages carried after the summary.
pub const COMPACTION_KEEP_LAST: usize = 6;
/// Upper bound for the generated summary, in characters.
pub const MAX_SUMMARY_CHARS: usize = 4_000;

/// Wall-clock milliseconds since the Unix epoch (the store's timestamp unit).
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// The persisted session mode. `Auto` is deliberately absent: full access is
/// session-scoped trust, never a durable mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionMode {
    #[default]
    Execute,
    Explore,
    Plan,
}

impl AgentSessionMode {
    /// The wire spelling used by the resume picker.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Explore => "explore",
            Self::Plan => "plan",
        }
    }
}

/// The session's plan state (Phase 9b's plan/apply split, persisted here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    #[default]
    None,
    Proposed,
    Approved,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanState {
    #[serde(default)]
    pub status: PlanStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default)]
    pub updated_at: u64,
}

/// One todo item. The router's `manage_todo` tool and the UI share this shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in progress",
            Self::Completed => "completed",
        }
    }
}

/// A card parked when the session was last written. A restored park is a
/// historical fact: it is shown once, is never auto-executed, and any
/// decision on it expires without reaching a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParkedCard {
    Approval {
        approval_id: String,
        tool_name: String,
        card: ApprovalCard,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        #[serde(default)]
        created_at: u64,
    },
    Question {
        question_id: String,
        prompt: String,
        #[serde(default)]
        options: Vec<String>,
        #[serde(default)]
        created_at: u64,
    },
}

impl ParkedCard {
    /// The one-line label the resume picker shows.
    pub fn summary(&self) -> String {
        match self {
            Self::Approval {
                tool_name, card, ..
            } => match card {
                ApprovalCard::Diff { path, .. } | ApprovalCard::Delete { path } => {
                    format!("{tool_name} {path}")
                }
                ApprovalCard::Command { command } => format!("{tool_name} {command}"),
                ApprovalCard::Workflow { name, .. } => format!("{tool_name} {name}"),
            },
            Self::Question { prompt, .. } => prompt.clone(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Approval { .. } => "approval",
            Self::Question { .. } => "question",
        }
    }
}

/// One transcript record (`transcript.jsonl`, one JSON object per line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptEntry {
    User {
        id: String,
        timestamp: u64,
        content: String,
    },
    Assistant {
        id: String,
        timestamp: u64,
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ToolCall>>,
    },
    /// The model's tool-call batch for one round (ordered before its results).
    ToolCalls {
        id: String,
        timestamp: u64,
        calls: Vec<ToolCall>,
    },
    Tool {
        id: String,
        timestamp: u64,
        call_id: String,
        ok: bool,
        content: String,
    },
}

impl TranscriptEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::User { id, .. }
            | Self::Assistant { id, .. }
            | Self::ToolCalls { id, .. }
            | Self::Tool { id, .. } => id,
        }
    }

    pub fn timestamp(&self) -> u64 {
        match self {
            Self::User { timestamp, .. }
            | Self::Assistant { timestamp, .. }
            | Self::ToolCalls { timestamp, .. }
            | Self::Tool { timestamp, .. } => *timestamp,
        }
    }
}

/// The persisted session metadata (never trust: no auto-approve field).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionState {
    pub schema: u32,
    pub session_id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: AgentSessionMode,
    #[serde(default)]
    pub plan: PlanState,
    /// Open approval/question parks. A restore surfaces these but never
    /// executes them.
    #[serde(default)]
    pub parks: Vec<ParkedCard>,
    #[serde(default)]
    pub compaction: CompactionCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CompactionCursor {
    #[serde(default)]
    pub compacted_at: u64,
    #[serde(default)]
    pub messages_before: usize,
    #[serde(default)]
    pub messages_after: usize,
    #[serde(default)]
    pub summary_chars: usize,
    #[serde(default)]
    pub keep_last: usize,
}

impl CompactionCursor {
    pub fn is_compacted(&self) -> bool {
        self.compacted_at != 0
    }
}

/// What a resume picker needs without loading the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionSummary {
    pub session_id: String,
    pub title: String,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub mode: AgentSessionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<ParkSummary>,
    pub compacted: bool,
    pub message_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParkSummary {
    pub kind: String,
    pub summary: String,
}

/// A transcript load result, including tail-recovery information.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptLoad {
    pub entries: Vec<TranscriptEntry>,
    /// Byte length of the valid, parseable prefix.
    pub valid_bytes: usize,
    /// `true` when unparseable bytes were dropped from the tail.
    pub recovered_tail: bool,
}

/// A fully loaded session.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedAgentSession {
    pub state: AgentSessionState,
    pub todos: Vec<TodoItem>,
    pub transcript: TranscriptLoad,
}

impl LoadedAgentSession {
    /// The transcript as request messages. A `ToolCalls` record attaches to
    /// the preceding assistant message (its natural place in the wire shape).
    pub fn conversation(&self) -> Vec<ChatMessage> {
        transcript_conversation(&self.transcript.entries)
    }
}

/// Convert transcript records into the chat wire shape.
pub fn transcript_conversation(entries: &[TranscriptEntry]) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = Vec::new();
    for entry in entries {
        match entry {
            TranscriptEntry::User { content, .. } => messages.push(ChatMessage::user(content)),
            TranscriptEntry::Assistant {
                content,
                tool_calls,
                ..
            } => {
                let mut message = ChatMessage::assistant(content);
                message.tool_calls = tool_calls.clone();
                messages.push(message);
            }
            TranscriptEntry::ToolCalls { calls, .. } => {
                // Merge into the immediately preceding assistant message when
                // it has no calls yet; otherwise start an assistant message
                // so the result pairing stays valid.
                let attached = messages
                    .last_mut()
                    .filter(|message| message.role == Role::Assistant)
                    .map(|message| {
                        if message.tool_calls.is_none() {
                            message.tool_calls = Some(calls.clone());
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !attached {
                    let mut message = ChatMessage::assistant("");
                    message.tool_calls = Some(calls.clone());
                    messages.push(message);
                }
            }
            TranscriptEntry::Tool {
                call_id, content, ..
            } => messages.push(ChatMessage::tool_result(call_id, content)),
        }
    }
    messages
}

/// The durable store rooted at one workspace.
#[derive(Debug, Clone)]
pub struct AgentSessionStore {
    root: PathBuf,
    redaction_key: Option<String>,
}

impl AgentSessionStore {
    /// A store at `<workspace_root>/.selora/agent-sessions`.
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            root: workspace_root.as_ref().join(AGENT_SESSIONS_DIR),
            redaction_key: None,
        }
    }

    /// Scrub this key from every string the store writes. The key is never a
    /// persisted field; this is defence in depth.
    pub fn with_redaction_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.redaction_key = if key.is_empty() { None } else { Some(key) };
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/{session_id}` after validating the id cannot escape the root.
    pub fn session_dir(&self, session_id: &str) -> Result<PathBuf, SeloraError> {
        validate_session_id(session_id)?;
        Ok(self.root.join(session_id))
    }

    pub fn exists(&self, session_id: &str) -> bool {
        self.session_dir(session_id)
            .map(|dir| dir.join(STATE_FILE).is_file())
            .unwrap_or(false)
    }

    /// Create a new session directory and state. The transcript starts empty.
    pub fn create(
        &self,
        session_id: &str,
        title: &str,
        model: Option<&str>,
        mode: AgentSessionMode,
    ) -> Result<AgentSessionState, SeloraError> {
        let dir = self.session_dir(session_id)?;
        fs::create_dir_all(&dir)?;
        let now = now_millis();
        let state = AgentSessionState {
            schema: SESSION_SCHEMA,
            session_id: session_id.to_owned(),
            title: self.scrub(title),
            created_at: now,
            updated_at: now,
            model: model.map(|model| self.scrub(model)),
            mode,
            plan: PlanState::default(),
            parks: Vec::new(),
            compaction: CompactionCursor::default(),
        };
        self.save_state(&state)?;
        self.save_todos(session_id, &[])?;
        // Materialize the transcript so a resume picker can see the session
        // even before its first record.
        let transcript = dir.join(TRANSCRIPT_FILE);
        if !transcript.exists() {
            let file = File::create(&transcript)?;
            file.sync_all()?;
        }
        Ok(state)
    }

    pub fn load_state(&self, session_id: &str) -> Result<Option<AgentSessionState>, SeloraError> {
        let dir = self.session_dir(session_id)?;
        let path = dir.join(STATE_FILE);
        if !path.is_file() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)?;
        let state: AgentSessionState = serde_json::from_str(&text).map_err(|error| {
            SeloraError::Config(format!("failed to read agent session state: {error}"))
        })?;
        if state.schema != SESSION_SCHEMA {
            return Err(SeloraError::Config(format!(
                "agent session schema {} is not supported",
                state.schema
            )));
        }
        Ok(Some(state))
    }

    pub fn save_state(&self, state: &AgentSessionState) -> Result<(), SeloraError> {
        let dir = self.session_dir(&state.session_id)?;
        fs::create_dir_all(&dir)?;
        let value = serde_json::to_value(state)
            .map_err(|error| SeloraError::Config(format!("session state: {error}")))?;
        let value = self.scrub_value(value);
        let json = serde_json::to_string_pretty(&value)
            .map_err(|error| SeloraError::Config(format!("session state: {error}")))?;
        atomic_write(&dir.join(STATE_FILE), json.as_bytes())
    }

    pub fn load_todos(&self, session_id: &str) -> Result<Vec<TodoItem>, SeloraError> {
        let dir = self.session_dir(session_id)?;
        let path = dir.join(TODOS_FILE);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&path)?;
        let todos: Vec<TodoItem> = serde_json::from_str(&text).map_err(|error| {
            SeloraError::Config(format!("failed to read agent session todos: {error}"))
        })?;
        Ok(todos)
    }

    pub fn save_todos(&self, session_id: &str, todos: &[TodoItem]) -> Result<(), SeloraError> {
        let dir = self.session_dir(session_id)?;
        fs::create_dir_all(&dir)?;
        let value = serde_json::to_value(todos)
            .map_err(|error| SeloraError::Config(format!("session todos: {error}")))?;
        let value = self.scrub_value(value);
        let json = serde_json::to_string_pretty(&value)
            .map_err(|error| SeloraError::Config(format!("session todos: {error}")))?;
        atomic_write(&dir.join(TODOS_FILE), json.as_bytes())
    }

    /// Read the transcript, recovering a torn/corrupt tail.
    pub fn load_transcript(&self, session_id: &str) -> Result<TranscriptLoad, SeloraError> {
        let dir = self.session_dir(session_id)?;
        let path = dir.join(TRANSCRIPT_FILE);
        if !path.is_file() {
            return Ok(TranscriptLoad {
                entries: Vec::new(),
                valid_bytes: 0,
                recovered_tail: false,
            });
        }
        let bytes = fs::read(&path)?;
        Ok(scan_transcript(&bytes))
    }

    /// Append one record atomically and fsync it. A damaged tail is truncated
    /// to the last valid record first (recovery is not a rewrite of history).
    pub fn append(&self, session_id: &str, entry: &TranscriptEntry) -> Result<(), SeloraError> {
        let dir = self.session_dir(session_id)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(TRANSCRIPT_FILE);
        let value = serde_json::to_value(entry)
            .map_err(|error| SeloraError::Config(format!("transcript entry: {error}")))?;
        let value = self.scrub_value(value);
        let line = serde_json::to_string(&value)
            .map_err(|error| SeloraError::Config(format!("transcript entry: {error}")))?;
        append_line(&path, line.as_bytes())
    }

    /// Atomically replace the transcript (retry truncation). Each record is
    /// written once to a temp file, fsynced, then renamed over the old file.
    pub fn rewrite_transcript(
        &self,
        session_id: &str,
        entries: &[TranscriptEntry],
    ) -> Result<(), SeloraError> {
        let dir = self.session_dir(session_id)?;
        fs::create_dir_all(&dir)?;
        let mut body = Vec::new();
        for entry in entries {
            let value = serde_json::to_value(entry)
                .map_err(|error| SeloraError::Config(format!("transcript entry: {error}")))?;
            let value = self.scrub_value(value);
            let line = serde_json::to_string(&value)
                .map_err(|error| SeloraError::Config(format!("transcript entry: {error}")))?;
            body.extend_from_slice(line.as_bytes());
            body.push(b'\n');
        }
        atomic_write(&dir.join(TRANSCRIPT_FILE), &body)
    }

    /// Load one session, or `None` when it does not exist.
    pub fn load(&self, session_id: &str) -> Result<Option<LoadedAgentSession>, SeloraError> {
        let Some(mut state) = self.load_state(session_id)? else {
            return Ok(None);
        };
        // The id on disk may not disagree with the directory (a copied store).
        state.session_id = session_id.to_owned();
        let todos = self.load_todos(session_id)?;
        let transcript = self.load_transcript(session_id)?;
        Ok(Some(LoadedAgentSession {
            state,
            todos,
            transcript,
        }))
    }

    /// The resume picker's list, newest first. Unreadable sessions are skipped
    /// (a corrupt directory must not hide the rest).
    pub fn list(&self) -> Result<Vec<AgentSessionSummary>, SeloraError> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let Some(session_id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(Some(state)) = self.load_state(&session_id) else {
                continue;
            };
            let transcript = self.load_transcript(&session_id).unwrap_or(TranscriptLoad {
                entries: Vec::new(),
                valid_bytes: 0,
                recovered_tail: true,
            });
            summaries.push(AgentSessionSummary {
                session_id,
                title: state.title,
                updated_at: state.updated_at,
                model: state.model,
                mode: state.mode,
                pending: state.parks.first().map(|park| ParkSummary {
                    kind: park.kind().to_owned(),
                    summary: park.summary(),
                }),
                compacted: state.compaction.is_compacted(),
                message_count: transcript.entries.len(),
            });
        }
        summaries.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        Ok(summaries)
    }

    /// Replace the session's open parks.
    pub fn set_parks(
        &self,
        state: &mut AgentSessionState,
        parks: Vec<ParkedCard>,
    ) -> Result<(), SeloraError> {
        state.parks = parks
            .into_iter()
            .map(|park| self.scrub_park(park))
            .collect();
        self.save_state(state)
    }

    /// Record a successful compaction. The transcript is untouched.
    pub fn record_compaction(
        &self,
        state: &mut AgentSessionState,
        cursor: CompactionCursor,
    ) -> Result<(), SeloraError> {
        state.compaction = cursor;
        state.updated_at = now_millis();
        self.save_state(state)
    }

    fn scrub(&self, text: &str) -> String {
        match &self.redaction_key {
            Some(key) => text.replace(key, "[REDACTED]"),
            None => text.to_owned(),
        }
    }

    fn scrub_value(&self, mut value: Value) -> Value {
        fn walk(value: &mut Value, key: &str) {
            match value {
                Value::String(text) => *text = text.replace(key, "[REDACTED]"),
                Value::Array(items) => {
                    for item in items {
                        walk(item, key);
                    }
                }
                Value::Object(map) => {
                    for item in map.values_mut() {
                        walk(item, key);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) => {}
            }
        }
        if let Some(key) = &self.redaction_key {
            walk(&mut value, key);
        }
        value
    }

    fn scrub_park(&self, park: ParkedCard) -> ParkedCard {
        match park {
            ParkedCard::Approval {
                approval_id,
                tool_name,
                card,
                request_id,
                turn_id,
                created_at,
            } => {
                let card = match card {
                    ApprovalCard::Diff { path, diff } => ApprovalCard::Diff {
                        path: self.scrub(&path),
                        diff: self.scrub(&diff),
                    },
                    ApprovalCard::Delete { path } => ApprovalCard::Delete {
                        path: self.scrub(&path),
                    },
                    ApprovalCard::Command { command } => ApprovalCard::Command {
                        command: self.scrub(&command),
                    },
                    ApprovalCard::Workflow {
                        name,
                        script,
                        commands,
                        max_cost_usd,
                    } => ApprovalCard::Workflow {
                        name: self.scrub(&name),
                        script: self.scrub(&script),
                        commands: commands.iter().map(|line| self.scrub(line)).collect(),
                        max_cost_usd,
                    },
                };
                ParkedCard::Approval {
                    approval_id,
                    tool_name: self.scrub(&tool_name),
                    card,
                    request_id,
                    turn_id,
                    created_at,
                }
            }
            ParkedCard::Question {
                question_id,
                prompt,
                options,
                created_at,
            } => ParkedCard::Question {
                question_id,
                prompt: self.scrub(&prompt),
                options: options.iter().map(|option| self.scrub(option)).collect(),
                created_at,
            },
        }
    }
}

fn validate_session_id(session_id: &str) -> Result<(), SeloraError> {
    let valid = !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if valid {
        Ok(())
    } else {
        Err(SeloraError::Config(format!(
            "invalid agent session id: {session_id:?}"
        )))
    }
}

/// Read a transcript body, keeping the longest valid prefix.
fn scan_transcript(bytes: &[u8]) -> TranscriptLoad {
    let mut entries = Vec::new();
    let mut valid_bytes = 0usize;
    let mut consumed = 0usize;
    for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
        consumed += chunk.len();
        if !chunk.ends_with(b"\n") {
            // A torn final record: ignore it (recovered below).
            break;
        }
        let line = &chunk[..chunk.len() - 1];
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            valid_bytes = consumed;
            continue;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            break;
        };
        match serde_json::from_str::<TranscriptEntry>(text) {
            Ok(entry) => {
                entries.push(entry);
                valid_bytes = consumed;
            }
            Err(_) => break,
        }
    }
    TranscriptLoad {
        entries,
        valid_bytes,
        recovered_tail: valid_bytes < bytes.len(),
    }
}

/// Append one newline-terminated record, truncating a damaged tail first.
fn append_line(path: &Path, line: &[u8]) -> Result<(), SeloraError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let valid = scan_transcript(&bytes).valid_bytes;
    if valid < bytes.len() {
        file.set_len(valid as u64)?;
    }
    file.seek(SeekFrom::End(0))?;
    file.write_all(line)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

/// Write a file atomically: temp file, fsync, rename over the target.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), SeloraError> {
    let parent = path
        .parent()
        .ok_or_else(|| SeloraError::Config(format!("path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)?;
    let mut tmp = path.to_path_buf().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            Err(SeloraError::Io(error))
        }
    }
}

// ---------------------------------------------------------------------------
// Compaction
// ---------------------------------------------------------------------------

/// A typed compaction failure. `SummaryFailed` carries the partial summary so
/// the caller can surface it instead of silently truncating history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactionError {
    SummaryFailed {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial_summary: Option<String>,
    },
    EmptySummary,
    PendingCards {
        count: usize,
    },
    NothingToCompact,
}

impl fmt::Display for CompactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SummaryFailed { message, .. } => {
                write!(formatter, "compaction summary failed: {message}")
            }
            Self::EmptySummary => formatter.write_str("compaction produced an empty summary"),
            Self::PendingCards { count } => write!(
                formatter,
                "compaction refused: {count} open card(s) must stay in the conversation"
            ),
            Self::NothingToCompact => {
                formatter.write_str("compaction refused: the conversation is too short")
            }
        }
    }
}

impl std::error::Error for CompactionError {}

/// What one successful compaction produced.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionOutcome {
    /// The bounded summary the model returned.
    pub summary: String,
    /// The conversation to continue with: summary + open todos + last N.
    pub conversation: Vec<ChatMessage>,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub cursor: CompactionCursor,
}

/// True when `estimated_tokens` has reached ~80% of `budget_tokens`.
pub fn should_compact(estimated_tokens: u64, budget_tokens: u64) -> bool {
    if budget_tokens == 0 {
        return false;
    }
    (estimated_tokens as f64) >= (budget_tokens as f64 * COMPACTION_THRESHOLD)
}

/// Bound a summary to [`MAX_SUMMARY_CHARS`] without splitting a character.
pub fn bound_summary(summary: &str) -> String {
    bound_summary_to(summary, MAX_SUMMARY_CHARS)
}

/// Bound a summary to `max_chars` (a trailing notice stays inside the bound).
pub fn bound_summary_to(summary: &str, max_chars: usize) -> String {
    let trimmed = summary.trim();
    let count = trimmed.chars().count();
    if max_chars == 0 {
        return String::new();
    }
    if count <= max_chars {
        return trimmed.to_owned();
    }
    let notice = "\n[summary truncated]";
    let notice_len = notice.chars().count();
    if max_chars <= notice_len {
        return trimmed.chars().take(max_chars).collect();
    }
    let keep = max_chars - notice_len;
    let mut bounded: String = trimmed.chars().take(keep).collect();
    bounded.push_str(notice);
    bounded
}

/// Build the summary request (one plain, tool-free round).
pub fn summary_request(
    model_id: &str,
    conversation: &[ChatMessage],
    todos: &[TodoItem],
) -> ChatRequest {
    let mut open = String::new();
    for todo in todos
        .iter()
        .filter(|todo| todo.status != TodoStatus::Completed)
    {
        open.push_str(&format!("- [{}] {}\n", todo.status.label(), todo.title));
    }
    if open.is_empty() {
        open.push_str("- (none)\n");
    }
    let instructions = format!(
        "You are compacting a long coding-agent conversation so the session can continue \
         inside its context window.\n\
         Write a dense summary of the work so far. Keep: the user's goals, decisions, files \
         touched, commands run, failures and their fixes, and what remains. Drop chatter.\n\
         Use at most {MAX_SUMMARY_CHARS} characters. Return only the summary.\n\n\
         Open todos:\n{open}"
    );
    let mut messages = vec![ChatMessage::system(instructions)];
    for message in conversation
        .iter()
        .filter(|message| message.role != Role::System)
    {
        let mut line = format!("{}: {}", role_label(message.role), message.content);
        if let Some(calls) = &message.tool_calls {
            let names: Vec<&str> = calls.iter().map(|call| call.name.as_str()).collect();
            line.push_str(&format!(" [tool calls: {}]", names.join(", ")));
        }
        messages.push(ChatMessage::user(line));
    }
    ChatRequest::new(model_id, messages).with_temperature(0.0)
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// Build the post-compaction conversation: the summary system message, the
/// open todos, then the last `keep_last` non-system messages.
pub fn compacted_conversation(
    summary: &str,
    todos: &[TodoItem],
    conversation: &[ChatMessage],
    keep_last: usize,
) -> Vec<ChatMessage> {
    let mut todos_text = String::new();
    for todo in todos
        .iter()
        .filter(|todo| todo.status != TodoStatus::Completed)
    {
        todos_text.push_str(&format!("- [{}] {}\n", todo.status.label(), todo.title));
    }
    if todos_text.is_empty() {
        todos_text.push_str("- (none)\n");
    }
    let system = format!(
        "You are continuing a compacted Selora coding-agent session. The full history is on \
         disk; this is the carried-forward state.\n\n\
         ## Summary of earlier work\n{summary}\n\n\
         ## Open todos\n{todos_text}"
    );
    let history: Vec<&ChatMessage> = conversation
        .iter()
        .filter(|message| message.role != Role::System)
        .collect();
    let mut carried: Vec<ChatMessage> = history
        .into_iter()
        .rev()
        .take(keep_last)
        .rev()
        .cloned()
        .collect();
    // A tool result without its call is invalid on the wire; drop the orphan
    // prefix. The summary already carries its outcome.
    while carried
        .first()
        .is_some_and(|message| message.role == Role::Tool)
    {
        carried.remove(0);
    }
    let mut conversation = Vec::with_capacity(carried.len() + 1);
    conversation.push(ChatMessage::system(system));
    conversation.extend(carried);
    conversation
}

/// Run one summarize round through the existing chat backend.
///
/// This never touches disk: the caller keeps the full transcript and records
/// the cursor separately. A pending card refuses compaction outright â€” a
/// decision must never be summarized away.
pub async fn compact_conversation<C: ChatStream>(
    chat: &mut C,
    model_id: &str,
    conversation: &[ChatMessage],
    todos: &[TodoItem],
    keep_last: usize,
    open_parks: usize,
) -> Result<CompactionOutcome, CompactionError> {
    if open_parks > 0 {
        return Err(CompactionError::PendingCards { count: open_parks });
    }
    if conversation.len() <= keep_last {
        return Err(CompactionError::NothingToCompact);
    }
    let request = summary_request(model_id, conversation, todos);
    let (tx, mut rx) = mpsc::unbounded_channel::<ChatEvent>();
    let receipt = chat.stream_round(request, tx).await;
    let mut text = String::new();
    while let Some(event) = rx.recv().await {
        if let ChatEvent::Delta(delta) = event {
            text.push_str(&delta);
        }
    }
    if let Err(error) = receipt {
        let partial = if text.trim().is_empty() {
            None
        } else {
            Some(bound_summary(&text))
        };
        return Err(CompactionError::SummaryFailed {
            message: error.to_string(),
            partial_summary: partial,
        });
    }
    let summary = bound_summary(&text);
    if summary.is_empty() {
        return Err(CompactionError::EmptySummary);
    }
    let keep_last = keep_last.max(1);
    let compacted = compacted_conversation(&summary, todos, conversation, keep_last);
    let tokens_before = super::turn::estimate_conversation_tokens(conversation);
    let tokens_after = super::turn::estimate_conversation_tokens(&compacted);
    let cursor = CompactionCursor {
        compacted_at: now_millis(),
        messages_before: conversation.len(),
        messages_after: compacted.len(),
        summary_chars: summary.chars().count(),
        keep_last,
    };
    Ok(CompactionOutcome {
        summary,
        conversation: compacted,
        tokens_before,
        tokens_after,
        cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use crate::types::ChatReceipt;

    static TEST_DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> PathBuf {
        let n = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "selora-agent-session-test-{}-{n}",
            std::process::id()
        ))
    }

    fn tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: r#"{"path":"src/main.rs"}"#.into(),
        }
    }

    fn entries() -> Vec<TranscriptEntry> {
        vec![
            TranscriptEntry::User {
                id: "m1".into(),
                timestamp: 1,
                content: "fix the build".into(),
            },
            TranscriptEntry::Assistant {
                id: "m2".into(),
                timestamp: 2,
                content: "reading".into(),
                tool_calls: None,
            },
            TranscriptEntry::ToolCalls {
                id: "m3".into(),
                timestamp: 3,
                calls: vec![tool_call("c1", "read_file")],
            },
            TranscriptEntry::Tool {
                id: "m4".into(),
                timestamp: 4,
                call_id: "c1".into(),
                ok: true,
                content: "fn main() {}".into(),
            },
        ]
    }

    #[test]
    fn transcript_round_trips_and_a_torn_tail_is_recovered() {
        let root = temp_root();
        let store = AgentSessionStore::new(&root);
        store
            .create(
                "s1",
                "Fix the build",
                Some("model-a"),
                AgentSessionMode::Execute,
            )
            .expect("create");
        for entry in entries() {
            store.append("s1", &entry).expect("append");
        }
        let loaded = store.load("s1").expect("load").expect("session");
        assert_eq!(loaded.transcript.entries, entries());
        assert!(!loaded.transcript.recovered_tail);
        let conversation = loaded.conversation();
        assert_eq!(conversation.len(), 3);
        assert_eq!(conversation[1].tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(conversation[2].tool_call_id.as_deref(), Some("c1"));

        // A process died mid-append: a partial JSON object with no newline.
        let path = store.session_dir("s1").unwrap().join(TRANSCRIPT_FILE);
        let mut file = OpenOptions::new().append(true).open(&path).expect("open");
        file.write_all(br#"{"kind":"assistant","id":"torn""#)
            .expect("write torn");
        file.sync_all().expect("sync");
        drop(file);

        let loaded = store.load("s1").expect("load").expect("session");
        assert!(loaded.transcript.recovered_tail);
        assert_eq!(loaded.transcript.entries, entries(), "valid prefix kept");

        // The next append truncates the torn tail instead of gluing to it.
        store
            .append(
                "s1",
                &TranscriptEntry::User {
                    id: "m5".into(),
                    timestamp: 5,
                    content: "continue".into(),
                },
            )
            .expect("append after torn tail");
        let loaded = store.load("s1").expect("load").expect("session");
        assert_eq!(loaded.transcript.entries.len(), 5);
        assert!(!loaded.transcript.recovered_tail);
        assert_eq!(loaded.transcript.entries[4].id(), "m5");

        // A corrupt record mid-file is a damaged tail too: recovery keeps the
        // parseable prefix and never invents a record.
        let mut body = fs::read(&path).expect("read");
        body.extend_from_slice(b"\n{ not json }\n");
        body.extend_from_slice(
            b"{\"kind\":\"user\",\"id\":\"good\",\"timestamp\":9,\"content\":\"x\"}\n",
        );
        fs::write(&path, &body).expect("write damaged");
        let loaded = store.load("s1").expect("load").expect("session");
        assert_eq!(loaded.transcript.entries.len(), 5);
        assert!(loaded.transcript.recovered_tail);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn state_and_parks_round_trip_and_auto_approve_is_absent() {
        let root = temp_root();
        let store = AgentSessionStore::new(&root);
        let mut state = store
            .create("s1", "Resume me", Some("model-a"), AgentSessionMode::Plan)
            .expect("create");
        state.plan = PlanState {
            status: PlanStatus::Proposed,
            plan: Some("1. read\n2. write".into()),
            updated_at: 5,
        };
        let parks = vec![
            ParkedCard::Approval {
                approval_id: Uuid::nil().to_string(),
                tool_name: "edit_file".into(),
                card: ApprovalCard::Diff {
                    path: "src/main.rs".into(),
                    diff: "@@ -1 +1 @@".into(),
                },
                request_id: Some("req-1".into()),
                turn_id: Some("turn-1".into()),
                created_at: 6,
            },
            ParkedCard::Question {
                question_id: "q1".into(),
                prompt: "Which branch?".into(),
                options: vec!["main".into(), "release".into()],
                created_at: 7,
            },
        ];
        store.set_parks(&mut state, parks.clone()).expect("parks");
        store
            .save_todos(
                "s1",
                &[TodoItem {
                    id: "t1".into(),
                    title: "write tests".into(),
                    status: TodoStatus::InProgress,
                }],
            )
            .expect("todos");

        let loaded = store.load("s1").expect("load").expect("session");
        assert_eq!(loaded.state.parks, parks);
        assert_eq!(loaded.state.mode, AgentSessionMode::Plan);
        assert_eq!(loaded.state.plan.status, PlanStatus::Proposed);
        assert_eq!(loaded.todos.len(), 1);

        // Trust is not a durable field: the JSON cannot arm a restored session.
        let json = fs::read_to_string(store.session_dir("s1").unwrap().join(STATE_FILE))
            .expect("state json");
        assert!(!json.contains("auto_approve"));
        assert!(!json.contains("autoApprove"));

        let list = store.list().expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].pending.as_ref().unwrap().kind, "approval");
        assert!(!list[0].compacted);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_api_key_is_never_written() {
        const KEY: &str = "sk-selora-test-key-do-not-persist";
        let root = temp_root();
        let store = AgentSessionStore::new(&root).with_redaction_key(KEY);
        let mut state = store
            .create("s1", "key test", Some(KEY), AgentSessionMode::Execute)
            .expect("create");
        state.plan.plan = Some(format!("use {KEY} only in memory"));
        store.save_state(&state).expect("state");
        store
            .append(
                "s1",
                &TranscriptEntry::User {
                    id: "m1".into(),
                    timestamp: 1,
                    content: format!("my key is {KEY}"),
                },
            )
            .expect("append");
        store
            .set_parks(
                &mut state,
                vec![ParkedCard::Approval {
                    approval_id: "a1".into(),
                    tool_name: "bash".into(),
                    card: ApprovalCard::Command {
                        command: format!("echo {KEY}"),
                    },
                    request_id: None,
                    turn_id: None,
                    created_at: 0,
                }],
            )
            .expect("park");

        for name in [STATE_FILE, TRANSCRIPT_FILE, TODOS_FILE] {
            let text =
                fs::read_to_string(store.session_dir("s1").unwrap().join(name)).unwrap_or_default();
            assert!(!text.contains(KEY), "{name} leaked the API key");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_and_load_are_session_scoped_and_validated() {
        let root = temp_root();
        let store = AgentSessionStore::new(&root);
        store
            .create("a", "A", None, AgentSessionMode::Execute)
            .expect("a");
        store
            .create("b", "B", None, AgentSessionMode::Explore)
            .expect("b");
        assert_eq!(store.list().expect("list").len(), 2);
        assert!(store.load("missing").expect("load").is_none());
        assert!(store.session_dir("../escape").is_err());
        assert!(store.load("../escape").is_err());
        let _ = fs::remove_dir_all(root);
    }

    // -----------------------------------------------------------------------
    // Compaction
    // -----------------------------------------------------------------------

    struct ScriptedChat {
        deltas: Vec<String>,
        error: Option<SeloraError>,
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    impl ScriptedChat {
        fn ok(deltas: &[&str]) -> Self {
            Self {
                deltas: deltas.iter().map(|delta| (*delta).to_owned()).collect(),
                error: None,
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ChatStream for ScriptedChat {
        fn stream_round(
            &mut self,
            request: ChatRequest,
            events: mpsc::UnboundedSender<ChatEvent>,
        ) -> crate::agent::turn::ChatRoundFuture<'_> {
            self.requests.lock().expect("requests").push(request);
            let deltas = self.deltas.clone();
            let error = self
                .error
                .as_ref()
                .map(|error| SeloraError::Net(error.to_string()));
            Box::pin(async move {
                for delta in deltas {
                    let _ = events.send(ChatEvent::Delta(delta));
                }
                match error {
                    Some(error) => Err(error),
                    None => Ok(ChatReceipt {
                        request_id: Uuid::nil(),
                        model: "m".into(),
                        prompt_tokens: 10,
                        completion_tokens: 10,
                        cost_usd: 0.0,
                        duration_ms: 1,
                        estimated: true,
                    }),
                }
            })
        }
    }

    #[test]
    fn threshold_is_eighty_percent() {
        assert!(!should_compact(799, 1000));
        assert!(should_compact(800, 1000));
        assert!(should_compact(900, 1000));
        assert!(!should_compact(100, 0), "unknown budget never compacts");
    }

    #[tokio::test]
    async fn compaction_carries_summary_todos_and_last_messages() {
        let conversation: Vec<ChatMessage> = (0..12)
            .map(|index| {
                let filler = "x".repeat(200);
                if index % 2 == 0 {
                    ChatMessage::user(format!("user {index} {filler}"))
                } else {
                    ChatMessage::assistant(format!("assistant {index} {filler}"))
                }
            })
            .collect();
        let todos = vec![TodoItem {
            id: "t1".into(),
            title: "finish compaction".into(),
            status: TodoStatus::Pending,
        }];
        let mut chat = ScriptedChat::ok(&["summary part one. ", "summary part two."]);

        let outcome = compact_conversation(&mut chat, "m", &conversation, &todos, 4, 0)
            .await
            .expect("compaction");
        assert_eq!(outcome.summary, "summary part one. summary part two.");
        assert_eq!(outcome.cursor.messages_before, 12);
        assert_eq!(outcome.cursor.keep_last, 4);
        assert_eq!(outcome.conversation.len(), 5, "summary + last 4");
        let system = &outcome.conversation[0];
        assert_eq!(system.role, Role::System);
        assert!(system.content.contains("summary part one"));
        assert!(system.content.contains("finish compaction"));
        assert!(outcome.conversation[4].content.starts_with("assistant 11"));
        assert!(outcome.tokens_after < outcome.tokens_before);

        let requests = chat.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1, "exactly one summarize round");
        assert!(requests[0].tools.is_none(), "the summary is tool-free");

        // The full history is untouched in the transcript (compaction is a
        // cursor, not a rewrite).
        assert_eq!(conversation.len(), 12);
    }

    #[tokio::test]
    async fn compaction_refuses_pending_cards_and_surfaces_failures() {
        let conversation: Vec<ChatMessage> = (0..8)
            .map(|index| ChatMessage::user(format!("m{index}")))
            .collect();
        let mut chat = ScriptedChat::ok(&["never used"]);
        let error = compact_conversation(&mut chat, "m", &conversation, &[], 2, 1)
            .await
            .expect_err("pending card");
        assert_eq!(error, CompactionError::PendingCards { count: 1 });

        let mut failing = ScriptedChat::ok(&["partial summary"]);
        failing.error = Some(SeloraError::Net("gateway down".into()));
        let error = compact_conversation(&mut failing, "m", &conversation, &[], 2, 0)
            .await
            .expect_err("summary failure");
        match error {
            CompactionError::SummaryFailed {
                message,
                partial_summary,
            } => {
                assert!(message.contains("gateway down"));
                assert_eq!(partial_summary.as_deref(), Some("partial summary"));
            }
            other => panic!("wrong error: {other:?}"),
        }

        let mut empty = ScriptedChat::ok(&["   "]);
        assert_eq!(
            compact_conversation(&mut empty, "m", &conversation, &[], 2, 0)
                .await
                .unwrap_err(),
            CompactionError::EmptySummary
        );
    }

    #[test]
    fn summaries_are_bounded_without_splitting_characters() {
        let summary = "é".repeat(200);
        let bounded = bound_summary_to(&summary, 100);
        assert_eq!(bounded.chars().count(), 100);
        assert!(bounded.ends_with("[summary truncated]"));
        assert_eq!(bound_summary_to(&summary, 10).chars().count(), 10);
        assert_eq!(bound_summary_to("short", 10), "short");
    }

    #[test]
    fn compaction_metadata_round_trips_and_keeps_the_transcript() {
        let root = temp_root();
        let store = AgentSessionStore::new(&root);
        let mut state = store
            .create("s1", "compacted", None, AgentSessionMode::Execute)
            .expect("create");
        for entry in entries() {
            store.append("s1", &entry).expect("append");
        }
        let before = store.load_transcript("s1").expect("transcript");
        store
            .record_compaction(
                &mut state,
                CompactionCursor {
                    compacted_at: 42,
                    messages_before: 4,
                    messages_after: 3,
                    summary_chars: 12,
                    keep_last: 2,
                },
            )
            .expect("record");
        let after = store.load_transcript("s1").expect("transcript");
        assert_eq!(after.entries, before.entries, "history preserved");
        let loaded = store.load("s1").expect("load").expect("session");
        assert!(loaded.state.compaction.is_compacted());
        let _ = fs::remove_dir_all(root);
    }
}
