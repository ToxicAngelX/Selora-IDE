//! The workflow run journal (ARCHITECTURE.md §18.3, ROADMAP.md Phase 8b).
//!
//! The journal is the run's memory: every step, `ask` result, `world.run`
//! outcome and world read appends a line to
//! `.selora/workflow-runs/{run_id}/journal.jsonl` **before** the run proceeds,
//! so a crash or Stop leaves an inspectable, resumable run.
//!
//! Two layers live here:
//!
//! * [`Journal`] — the in-memory buffer 8a records into while the engine runs
//!   (the stub's shape, unchanged: `seq` + `kind` + opaque `payload`).
//! * [`JournalWriter`] — the durable append handle: [`Journal::open`] creates
//!   the run directory, seeds `.selora/.gitignore`, continues an existing
//!   run's sequence, and writes one line per [`JournalEvent`] with a single
//!   write + flush + `sync_data`.
//!
//! [`Journal::load`] parses the file and returns a [`ResumePlan`]:
//!
//! - world reads replay byte-identically — the model is not re-run against a
//!   filesystem that may have changed underneath it;
//! - finished asks are not re-paid (`name`-keyed amend/cache is v1.x; the v1
//!   journal key is the step id);
//! - unfinished steps re-dispatch in their original order.
//!
//! A damaged tail — a torn write from the crash that produced it — is
//! reported as a [`CorruptLine`] and skipped, never a panic: surviving that
//! crash intact is the journal's whole job. A journal whose last line is
//! incomplete still resumes from everything before it.
//!
//! The run id is also the directory name, so it is validated against a
//! conservative `[A-Za-z0-9_-]` alphabet: a script-thrown crash cannot turn a
//! run id into a path escape.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RunId;

/// Workspace-relative directory holding every run (ROADMAP.md Phase 8).
pub const JOURNAL_DIR: &str = ".selora/workflow-runs";
/// The journal file inside one run's directory.
pub const JOURNAL_FILE: &str = "journal.jsonl";

/// Longest accepted run id; a UUID is 36 characters.
const MAX_RUN_ID_LEN: usize = 128;
/// How much of a damaged line the report keeps.
const CORRUPT_PREVIEW_CHARS: usize = 160;

/// The directory of one run: `{workspace}/.selora/workflow-runs/{run_id}`.
#[must_use]
pub fn run_dir(workspace: &Path, run_id: &RunId) -> PathBuf {
    workspace.join(JOURNAL_DIR).join(run_id.as_str())
}

/// The journal path of one run.
#[must_use]
pub fn journal_path(workspace: &Path, run_id: &RunId) -> PathBuf {
    run_dir(workspace, run_id).join(JOURNAL_FILE)
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

/// One append-only journal record (the 8a shape, unchanged).
///
/// `kind` names the event and `payload` holds its fields. [`JournalEvent`]
/// is the typed vocabulary 8b writes; it serializes into exactly this shape,
/// so the file stays greppable and forward-additive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// 1-based, monotonic within a run.
    pub seq: u64,
    /// What happened (`run_started`, `ask_finished`, `world_read`, …).
    pub kind: String,
    /// The record payload.
    pub payload: Value,
}

/// An in-memory journal buffer for one run.
///
/// 8a records here while the script runs; the shell hands it to the report.
/// [`Journal::open`] returns the durable [`JournalWriter`] that mirrors every
/// append into one of these.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    /// The run this journal belongs to.
    pub run_id: Option<RunId>,
    /// Appended entries, in order.
    pub entries: Vec<JournalEntry>,
}

impl Journal {
    /// An empty journal for `run_id`.
    #[must_use]
    pub fn new(run_id: RunId) -> Self {
        Self {
            run_id: Some(run_id),
            entries: Vec::new(),
        }
    }

    /// Append one entry in memory, with the next sequence number.
    pub fn push(&mut self, kind: impl Into<String>, payload: Value) {
        let seq = self.next_seq();
        self.entries.push(JournalEntry {
            seq,
            kind: kind.into(),
            payload,
        });
    }

    /// The entries recorded so far.
    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Number of entries recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The sequence number a new in-memory entry would take.
    #[must_use]
    pub fn next_seq(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| entry.seq)
            .max()
            .map_or(1, |seq| seq.saturating_add(1))
    }

    /// Open (creating if needed) the run's JSONL journal for append.
    ///
    /// The journal is loaded first so a resumed run appends after its
    /// existing records instead of starting a second file.
    ///
    /// # Errors
    /// Invalid run id, or an IO/serialization failure while creating or
    /// reading the journal.
    pub fn open(
        workspace: impl AsRef<Path>,
        run_id: impl Into<RunId>,
    ) -> Result<JournalWriter, JournalError> {
        JournalWriter::open(workspace, run_id)
    }

    /// Load a run's journal without opening it for append.
    ///
    /// A missing journal is an empty journal (a run that never started), not
    /// an error. Damaged lines are collected in [`LoadedJournal::corrupt`]
    /// and skipped; the valid prefix remains usable for resume.
    ///
    /// # Errors
    /// Invalid run id, or the file exists but cannot be read.
    pub fn load(
        workspace: impl AsRef<Path>,
        run_id: impl Into<RunId>,
    ) -> Result<LoadedJournal, JournalError> {
        let run_id = run_id.into();
        validate_run_id(run_id.as_str())?;
        let dir = checked_run_dir(workspace.as_ref(), &run_id, false)?;
        let path = dir.join(JOURNAL_FILE);
        check_file(&path)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(source) => return Err(JournalError::Read { path, source }),
        };
        Ok(parse_journal(&path, &bytes))
    }
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// The step returned normally.
    Ok,
    /// The step failed; the script decides whether that ends the run.
    Failed,
    /// The step was not run (a gate branch, a loop cap, a replay).
    Skipped,
    /// The run stopped (Stop, crash, budget) while the step was in flight.
    Aborted,
}

/// How a whole run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The script ran to completion.
    Completed,
    /// A step failed in a way the script did not catch.
    Failed,
    /// The user pressed Stop.
    Stopped,
    /// The workflow-level cost cap fired (between steps).
    BudgetExceeded,
}

/// Which read-only facade a [`ReadRequest`] targets (§18.2 `files`/`git`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFamily {
    /// `files.glob` / `files.grep` / `files.read`.
    Files,
    /// `git.changedFiles` / `git.diff` / `git.status` / `git.log`.
    Git,
}

/// The identity of one world read, and therefore its replay key.
///
/// The facade normalizes arguments into a deterministic string list. Equality
/// and hashing are structural, which is what makes resume's "replay
/// byte-identically" a map lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReadRequest {
    /// `files` or `git`.
    pub family: ReadFamily,
    /// The operation name without the family prefix (`read`, `grep`,
    /// `diff`, `log`, …).
    pub op: String,
    /// Normalized positional arguments (paths, patterns, base refs, counts).
    pub args: Vec<String>,
}

impl ReadRequest {
    /// A `files.*` read key; `args` normalizes into owned strings.
    pub fn files<A>(op: impl Into<String>, args: A) -> Self
    where
        A: IntoIterator,
        A::Item: Into<String>,
    {
        Self::new(ReadFamily::Files, op, args)
    }

    /// A `git.*` read key.
    pub fn git<A>(op: impl Into<String>, args: A) -> Self
    where
        A: IntoIterator,
        A::Item: Into<String>,
    {
        Self::new(ReadFamily::Git, op, args)
    }

    fn new<A>(family: ReadFamily, op: impl Into<String>, args: A) -> Self
    where
        A: IntoIterator,
        A::Item: Into<String>,
    {
        Self {
            family,
            op: op.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

/// A `world.run` outcome (§17.5 spawn). The exit code is a value the script
/// branches on; `None` means the child was killed (timeout or Stop) before it
/// could exit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    /// `None` when the child was killed before exiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Captured stdout, already capped by the spawn (§17.5).
    pub stdout: String,
    /// Captured stderr, already capped by the spawn (§17.5).
    pub stderr: String,
    /// The spawn's timeout fired and the child was killed.
    #[serde(default)]
    pub timed_out: bool,
    /// Wall-clock duration of the child.
    #[serde(default)]
    pub duration_ms: u64,
}

impl RunOutcome {
    /// The gate's normal success test: a real exit status of zero.
    #[must_use]
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// A finished ask, as resume sees it: the validated result plus its receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskOutcome {
    /// The identity that asked.
    pub agent: String,
    /// The schema-validated result (8a validates at the boundary; the
    /// journal stores the value, not the raw text).
    pub result: Value,
    /// Receipt tokens for this ask.
    pub tokens: u64,
    /// Estimated USD for this ask.
    pub cost_usd: f64,
}

/// The typed journal vocabulary. Serialized with a `"kind"` tag, then split
/// into [`JournalEntry::kind`] + [`JournalEntry::payload`] so 8a's record
/// shape and 8b's typed events share one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalEvent {
    /// The confirmed run began. `script_hash` is the engine's opaque
    /// fingerprint of the approved script; resume refuses a different one.
    RunStarted {
        /// Opaque fingerprint of the confirmed script.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        script_hash: Option<String>,
        /// The §18.1 workflow-level cap in effect.
        max_cost_usd: f64,
    },
    /// A `phase()` marker — the panel's graph node.
    PhaseStarted {
        /// The script-supplied phase name.
        phase: String,
    },
    /// A step (an ask, a gate, a read, a batch) started.
    StepStarted {
        /// Stable step identity, unique within the run.
        step_id: String,
        /// The phase the step belongs to, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
        /// Human-readable label for the panel's step row.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A step ended (including `Aborted` on Stop/crash).
    StepFinished {
        /// The step identity from `StepStarted`.
        step_id: String,
        /// How it ended.
        status: StepStatus,
        /// Failure/abort detail for the panel log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// An ask was dispatched — journaled before the model is called, so a
    /// crash mid-ask leaves a visible unfinished step.
    AskStarted {
        /// The ask's step identity.
        step_id: String,
        /// The named agent.
        agent: String,
    },
    /// An ask finished: its validated result and receipt.
    AskFinished {
        /// The ask's step identity.
        step_id: String,
        /// The named agent.
        agent: String,
        /// The schema-validated result.
        result: Value,
        /// Receipt tokens.
        tokens: u64,
        /// Estimated USD.
        cost_usd: f64,
    },
    /// A `world.run` gate finished. The outcome was journaled before the
    /// script branched on it.
    WorldRun {
        /// The gate's step identity.
        step_id: String,
        /// The confirmed command literal.
        command: String,
        /// Its arguments.
        #[serde(default)]
        args: Vec<String>,
        /// Exit code and capped output.
        outcome: RunOutcome,
    },
    /// A read-only world read finished. `content` is replayed verbatim.
    WorldRead {
        /// The read's step identity.
        step_id: String,
        /// The read's replay key.
        read: ReadRequest,
        /// The read's exact text (JSON round-trips it byte-for-byte).
        content: String,
    },
    /// The run ended. Present only when the run reached a terminal state.
    RunFinished {
        /// Terminal status.
        status: RunStatus,
        /// Detail for the panel/run log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        /// Final workflow-level spend, authoritative over the per-ask sum
        /// when it is larger (a crashed run never writes this event).
        spent_usd: f64,
    },
}

impl JournalEvent {
    /// The record kind this event serializes as.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RunStarted { .. } => "run_started",
            Self::PhaseStarted { .. } => "phase_started",
            Self::StepStarted { .. } => "step_started",
            Self::StepFinished { .. } => "step_finished",
            Self::AskStarted { .. } => "ask_started",
            Self::AskFinished { .. } => "ask_finished",
            Self::WorldRun { .. } => "world_run",
            Self::WorldRead { .. } => "world_read",
            Self::RunFinished { .. } => "run_finished",
        }
    }
}

/// The record kinds [`JournalEvent`] knows. Unknown kinds are skipped by
/// resume rather than reported as damage — forward compatibility.
fn known_kind(kind: &str) -> bool {
    matches!(
        kind,
        "run_started"
            | "phase_started"
            | "step_started"
            | "step_finished"
            | "ask_started"
            | "ask_finished"
            | "world_run"
            | "world_read"
            | "run_finished"
    )
}

fn event_to_entry(seq: u64, event: JournalEvent) -> Result<JournalEntry, JournalError> {
    let value = serde_json::to_value(&event)?;
    let Value::Object(mut object) = value else {
        return Err(JournalError::NotAnObject);
    };
    let kind = object
        .remove("kind")
        .and_then(|kind| kind.as_str().map(str::to_owned))
        .ok_or(JournalError::NotAnObject)?;
    Ok(JournalEntry {
        seq,
        kind,
        payload: Value::Object(object),
    })
}

/// Decode a record into a typed event.
///
/// `None` means the kind is unknown (skip: a newer writer, not damage).
/// `Some(Err(..))` means a known kind carried a malformed payload.
fn event_from_entry(entry: &JournalEntry) -> Option<Result<JournalEvent, String>> {
    if !known_kind(&entry.kind) {
        return None;
    }
    let mut object = match &entry.payload {
        Value::Object(object) => object.clone(),
        other => return Some(Err(format!("payload is {other}, expected a JSON object"))),
    };
    object.insert("kind".to_owned(), Value::String(entry.kind.clone()));
    Some(serde_json::from_value(Value::Object(object)).map_err(|error| error.to_string()))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Errors from opening, appending to, or loading a journal.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// An untrusted path or damaged interior record cannot be resumed safely.
    #[error("unsafe workflow journal: {0}")]
    Unsafe(String),
    /// The run id would escape the runs directory or is not a safe name.
    #[error("invalid workflow run id {run_id:?}: expected 1-128 of [A-Za-z0-9_-]")]
    InvalidRunId {
        /// The rejected id.
        run_id: String,
    },
    /// The run directory could not be created.
    #[error("cannot create the workflow run directory {path}: {source}")]
    CreateDir {
        /// The directory in question.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// The journal file could not be opened for append.
    #[error("cannot open the workflow journal {path}: {source}")]
    Open {
        /// The file in question.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// A single append did not reach the file.
    #[error("cannot write the workflow journal {path}: {source}")]
    Append {
        /// The file in question.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// The journal could not be read for load/resume.
    #[error("cannot read the workflow journal {path}: {source}")]
    Read {
        /// The file in question.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// The record could not be serialized (never a user-facing condition;
    /// surfaced rather than swallowed).
    #[error("cannot serialize a workflow journal entry: {source}")]
    Serialize {
        /// The serializer error.
        #[from]
        source: serde_json::Error,
    },
    /// A journal event did not serialize to the expected `kind` + fields
    /// object. A programming error in an event definition, surfaced loudly.
    #[error("a workflow journal event did not serialize to a JSON object with a kind")]
    NotAnObject,
}

/// A journal loaded under the strict resume proof rule.
///
/// Unlike [`Journal::load`], which reports damage and keeps the surviving
/// prefix, [`load_verified`] accepts only a torn *last* line: the write a
/// crash interrupted. Any other damaged line makes the run state unprovable,
/// so resume refuses instead of silently dropping a journaled effect.
#[derive(Debug)]
pub(crate) struct VerifiedJournal {
    /// Valid records in file order, plus any damage proven to be a torn tail.
    pub(crate) loaded: LoadedJournal,
    /// The raw file bytes the load was derived from.
    pub(crate) bytes: Vec<u8>,
    /// Byte offset just past the last complete line.
    pub(crate) prefix: usize,
    /// The trailing bytes after `prefix` are an interrupted record.
    pub(crate) tail_is_torn: bool,
}

/// Load a run's journal and prove that any damage is only a torn tail.
///
/// This is the resume rule: a crash may leave one partial final line, which
/// the writer repairs; a damaged interior record could hide a completed
/// effect, so it is a typed [`JournalError::Unsafe`] instead.
///
/// # Errors
/// Invalid run id, an untrusted path, an IO failure, or interior damage.
pub(crate) fn load_verified(
    workspace: &Path,
    run_id: &RunId,
) -> Result<VerifiedJournal, JournalError> {
    validate_run_id(run_id.as_str())?;
    let dir = checked_run_dir(workspace, run_id, false)?;
    let path = dir.join(JOURNAL_FILE);
    check_file(&path)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(source) => return Err(JournalError::Read { path, source }),
    };
    let loaded = parse_journal(&path, &bytes);
    let prefix = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let tail_is_torn =
        prefix < bytes.len() && serde_json::from_slice::<JournalEntry>(&bytes[prefix..]).is_err();
    let interior_lines = bytes[..prefix].split(|b| *b == b'\n').count() - 1;
    if loaded
        .corrupt
        .iter()
        .any(|line| line.line <= interior_lines || !tail_is_torn)
    {
        return Err(JournalError::Unsafe("damaged interior record".into()));
    }
    Ok(VerifiedJournal {
        loaded,
        bytes,
        prefix,
        tail_is_torn,
    })
}

/// The durable append handle for one run's journal.
///
/// Opening an existing journal continues its sequence, so a resumed run
/// appends to the same file. Every [`JournalWriter::append`] writes exactly
/// one line with one `write_all`, then flushes and `sync_data`s it: a crash
/// either has the whole record or a torn tail that [`Journal::load`] reports.
#[derive(Debug)]
pub struct JournalWriter {
    path: PathBuf,
    file: File,
    journal: Journal,
    next_seq: u64,
}

impl JournalWriter {
    /// Open (creating if needed) a run's journal for append.
    ///
    /// # Errors
    /// See [`Journal::open`].
    pub fn open(
        workspace: impl AsRef<Path>,
        run_id: impl Into<RunId>,
    ) -> Result<Self, JournalError> {
        let run_id = run_id.into();
        validate_run_id(run_id.as_str())?;
        let workspace = workspace.as_ref();
        let dir = checked_run_dir(workspace, &run_id, true)?;
        ensure_selora_gitignored(workspace);

        let path = dir.join(JOURNAL_FILE);
        let verified = load_verified(workspace, &run_id)?;
        let bytes = verified.bytes;
        let prefix = verified.prefix;
        let tail_is_torn = verified.tail_is_torn;
        let next_seq = verified.loaded.next_seq();
        let entries = verified.loaded.entries;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| JournalError::Open {
                path: path.clone(),
                source,
            })?;
        if tail_is_torn {
            let repair = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(|source| JournalError::Open {
                    path: path.clone(),
                    source,
                })?;
            repair
                .set_len(prefix as u64)
                .map_err(|source| JournalError::Open {
                    path: path.clone(),
                    source,
                })?;
            repair.sync_data().map_err(|source| JournalError::Open {
                path: path.clone(),
                source,
            })?;
        } else if prefix < bytes.len() {
            // A complete final record without a newline must not glue to an append.
            (&file)
                .write_all(b"\n")
                .map_err(|source| JournalError::Open {
                    path: path.clone(),
                    source,
                })?;
        }
        Ok(Self {
            path,
            file,
            journal: Journal {
                run_id: Some(run_id),
                entries,
            },
            next_seq,
        })
    }

    /// The journal file this handle appends to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The sequence number the next append will use.
    #[must_use]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The in-memory mirror of the journal written so far.
    #[must_use]
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Consume the writer, handing back the in-memory mirror for the report.
    #[must_use]
    pub fn into_journal(self) -> Journal {
        self.journal
    }

    /// The entries written so far (including those loaded on open).
    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.journal.entries
    }

    /// Append one typed event as a single JSONL line, before the run
    /// proceeds. Returns the assigned sequence number.
    ///
    /// # Errors
    /// Serialization failure or an IO failure that prevented the line from
    /// reaching the file.
    pub fn append(&mut self, event: JournalEvent) -> Result<u64, JournalError> {
        let entry = event_to_entry(self.next_seq, event)?;
        self.write_entry(entry)
    }

    /// Append an already-shaped record (`kind` + `payload`), for 8a's
    /// identity/phase bookkeeping where the typed event is not needed.
    ///
    /// # Errors
    /// An IO failure that prevented the line from reaching the file.
    pub fn append_raw(
        &mut self,
        kind: impl Into<String>,
        payload: Value,
    ) -> Result<u64, JournalError> {
        let entry = JournalEntry {
            seq: self.next_seq,
            kind: kind.into(),
            payload,
        };
        self.write_entry(entry)
    }

    fn write_entry(&mut self, entry: JournalEntry) -> Result<u64, JournalError> {
        let seq = entry.seq;
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        self.file
            .write_all(line.as_bytes())
            .map_err(|source| JournalError::Append {
                path: self.path.clone(),
                source,
            })?;
        self.file.flush().map_err(|source| JournalError::Append {
            path: self.path.clone(),
            source,
        })?;
        // `flush` on a File is a no-op; durability is the sync. A workflow
        // step is orders of magnitude slower than this fsync.
        self.file
            .sync_data()
            .map_err(|source| JournalError::Append {
                path: self.path.clone(),
                source,
            })?;

        self.journal.entries.push(entry);
        self.next_seq = seq.saturating_add(1);
        Ok(seq)
    }
}

fn validate_run_id(run_id: &str) -> Result<(), JournalError> {
    let upper = run_id.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        });
    let valid = !run_id.is_empty()
        && !reserved
        && run_id.len() <= MAX_RUN_ID_LEN
        && run_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if valid {
        Ok(())
    } else {
        Err(JournalError::InvalidRunId {
            run_id: run_id.to_owned(),
        })
    }
}

/// Seed `.selora/.gitignore` with `*` when the workspace has none.
///
/// Best effort: failing the append because an ignore file could not be
/// created would be the wrong trade — the journal itself is the contract.
fn ensure_selora_gitignored(workspace: &Path) {
    let dir = workspace.join(".selora");
    let ignore = dir.join(".gitignore");
    if let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(ignore) {
        let _ = file.write_all(b"*\n");
    }
}

/// Resolve only beneath the canonical workspace, rejecting symlinks and reparse points.
pub(crate) fn checked_run_dir(
    workspace: &Path,
    run_id: &RunId,
    create: bool,
) -> Result<PathBuf, JournalError> {
    validate_run_id(run_id.as_str())?;
    let root = fs::canonicalize(workspace).map_err(|source| JournalError::Read {
        path: workspace.to_path_buf(),
        source,
    })?;
    let mut dir = root.clone();
    for name in [".selora", "workflow-runs", run_id.as_str()] {
        dir.push(name);
        if create && !dir.exists() {
            fs::create_dir(&dir).map_err(|source| JournalError::CreateDir {
                path: dir.clone(),
                source,
            })?;
        }
        match fs::symlink_metadata(&dir) {
            Ok(meta) => {
                if is_link(&meta)
                    || !meta.is_dir()
                    || !fs::canonicalize(&dir)
                        .map_err(|source| JournalError::Read {
                            path: dir.clone(),
                            source,
                        })?
                        .starts_with(&root)
                {
                    return Err(JournalError::Unsafe(format!(
                        "untrusted directory {}",
                        dir.display()
                    )));
                }
            }
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(JournalError::Read { path: dir, source }),
        }
    }
    Ok(dir)
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

pub(crate) fn check_file(path: &Path) -> Result<(), JournalError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) || !meta.is_file() => Err(JournalError::Unsafe(format!(
            "untrusted file {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(JournalError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

// ---------------------------------------------------------------------------
// Reading and resume
// ---------------------------------------------------------------------------

/// A line (or payload) that did not parse. Reported, never fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorruptLine {
    /// 1-based line number in the file.
    pub line: usize,
    /// The first [`CORRUPT_PREVIEW_CHARS`] chars of the line (lossy).
    pub text: String,
    /// The parser's complaint.
    pub detail: String,
}

/// A parsed journal: every record plus the damaged lines, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedJournal {
    /// The file that was read (whether or not it existed).
    pub path: PathBuf,
    /// Valid records in file order.
    pub entries: Vec<JournalEntry>,
    /// Damaged lines and malformed known-kind payloads, in file order.
    pub corrupt: Vec<CorruptLine>,
}

impl LoadedJournal {
    /// No records (an empty or entirely damaged journal).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// At least one line failed to parse; resume may be incomplete.
    #[must_use]
    pub fn is_damaged(&self) -> bool {
        !self.corrupt.is_empty()
    }

    /// The next sequence number: one past the highest valid record.
    #[must_use]
    pub fn next_seq(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| entry.seq)
            .max()
            .map_or(1, |seq| seq.saturating_add(1))
    }

    /// Fold the records into what a resume needs (see [`ResumePlan`]).
    #[must_use]
    pub fn resume_plan(&self) -> ResumePlan {
        ResumePlan::new(&self.entries)
    }
}

fn parse_journal(path: &Path, bytes: &[u8]) -> LoadedJournal {
    let mut loaded = LoadedJournal {
        path: path.to_path_buf(),
        entries: Vec::new(),
        corrupt: Vec::new(),
    };
    for (index, raw) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line_number = index + 1;
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<JournalEntry>(line) {
            Ok(entry) => {
                // A known kind with a malformed payload is damage too: the
                // record is readable, but resume cannot use it.
                if let Some(Err(detail)) = event_from_entry(&entry) {
                    loaded.corrupt.push(CorruptLine {
                        line: line_number,
                        text: preview(line),
                        detail,
                    });
                }
                loaded.entries.push(entry);
            }
            Err(error) => loaded.corrupt.push(CorruptLine {
                line: line_number,
                text: preview(line),
                detail: error.to_string(),
            }),
        }
    }
    loaded
}

fn preview(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .chars()
        .take(CORRUPT_PREVIEW_CHARS)
        .collect()
}

/// What a relaunch needs to continue a run (ARCHITECTURE.md §18.3).
///
/// Built by [`LoadedJournal::resume_plan`]; the engine consults it at each
/// facade call:
///
/// - [`ResumePlan::finished_ask`] → return the journaled result, do not pay
///   for the ask again;
/// - [`ResumePlan::world_read`] → return the journaled bytes, do not touch
///   the filesystem again;
/// - [`ResumePlan::world_run`] → return the journaled gate outcome (a gate
///   that already ran is not re-run: it may have had side effects);
/// - [`ResumePlan::unfinished_steps`] → dispatch, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResumePlan {
    started: bool,
    finished: Option<RunStatus>,
    phases: Vec<String>,
    step_order: Vec<String>,
    finished_steps: HashSet<String>,
    unfinished: Vec<String>,
    finished_asks: HashMap<String, AskOutcome>,
    world_runs: HashMap<String, RunOutcome>,
    world_reads: HashMap<ReadRequest, String>,
    spent_usd: f64,
    tokens: u64,
}

impl ResumePlan {
    /// Fold a journal's records into a plan.
    #[must_use]
    pub fn new(entries: &[JournalEntry]) -> Self {
        let mut plan = Self::default();
        // Unified first-seen order: an ask is a step the panel and resume
        // order by, so both event kinds feed one dispatch sequence.
        let mut start_order: Vec<String> = Vec::new();
        let mut ask_ids: HashSet<String> = HashSet::new();

        for entry in entries {
            let Some(event) = event_from_entry(entry).and_then(Result::ok) else {
                continue;
            };
            match &event {
                JournalEvent::RunStarted { .. } => plan.started = true,
                JournalEvent::PhaseStarted { phase } => {
                    if !plan.phases.contains(phase) {
                        plan.phases.push(phase.clone());
                    }
                }
                JournalEvent::StepStarted { step_id, .. } => {
                    if !plan.step_order.contains(step_id) {
                        plan.step_order.push(step_id.clone());
                    }
                    if !start_order.contains(step_id) {
                        start_order.push(step_id.clone());
                    }
                }
                JournalEvent::StepFinished { step_id, .. } => {
                    plan.finished_steps.insert(step_id.clone());
                }
                JournalEvent::AskStarted { step_id, .. } => {
                    ask_ids.insert(step_id.clone());
                    if !start_order.contains(step_id) {
                        start_order.push(step_id.clone());
                    }
                }
                JournalEvent::AskFinished {
                    step_id,
                    agent,
                    result,
                    tokens,
                    cost_usd,
                } => {
                    // A retried ask is paid twice; the plan keeps the spend
                    // truthfully and the latest result for replay.
                    plan.tokens = plan.tokens.saturating_add(*tokens);
                    plan.spent_usd += *cost_usd;
                    plan.finished_asks.insert(
                        step_id.clone(),
                        AskOutcome {
                            agent: agent.clone(),
                            result: result.clone(),
                            tokens: *tokens,
                            cost_usd: *cost_usd,
                        },
                    );
                }
                JournalEvent::WorldRun {
                    step_id, outcome, ..
                } => {
                    plan.world_runs.insert(step_id.clone(), outcome.clone());
                }
                JournalEvent::WorldRead { read, content, .. } => {
                    // First record wins: a healthy run journals a read once
                    // per key; keeping the first makes replay deterministic
                    // even after a pathological double-append.
                    plan.world_reads
                        .entry(read.clone())
                        .or_insert_with(|| content.clone());
                }
                JournalEvent::RunFinished {
                    status, spent_usd, ..
                } => {
                    plan.finished = Some(*status);
                    // The run summary is authoritative when it is larger (it
                    // accounts for charges no ask recorded), never a double
                    // count.
                    plan.spent_usd = plan.spent_usd.max(*spent_usd);
                }
            }
        }

        // Unfinished steps, in the order they first started, where
        // "finished" means either a `StepFinished` or a journaled ask result.
        // An ask that crashed mid-flight is unfinished even though no
        // `StepFinished` ever arrived for it.
        for step_id in &start_order {
            let ask_finished =
                ask_ids.contains(step_id) && plan.finished_asks.contains_key(step_id);
            if !plan.finished_steps.contains(step_id) && !ask_finished {
                plan.unfinished.push(step_id.clone());
            }
        }

        plan
    }

    /// The run reached a terminal state; there is nothing to resume.
    #[must_use]
    pub fn is_run_finished(&self) -> bool {
        self.finished.is_some()
    }

    /// The journaled terminal status, if the run ended.
    #[must_use]
    pub fn run_status(&self) -> Option<RunStatus> {
        self.finished
    }

    /// Whether the journal shows the run started at all.
    #[must_use]
    pub fn has_started(&self) -> bool {
        self.started
    }

    /// Phase names in the order the script declared them.
    #[must_use]
    pub fn phases(&self) -> &[String] {
        &self.phases
    }

    /// Every step that started, in order.
    #[must_use]
    pub fn steps_started(&self) -> &[String] {
        &self.step_order
    }

    /// Whether a step recorded a `StepFinished`.
    #[must_use]
    pub fn is_step_finished(&self, step_id: &str) -> bool {
        self.finished_steps.contains(step_id)
    }

    /// The steps to re-dispatch, in their original order.
    #[must_use]
    pub fn unfinished_steps(&self) -> &[String] {
        &self.unfinished
    }

    /// A finished ask's journaled result — resume returns this instead of
    /// paying for the ask again.
    #[must_use]
    pub fn finished_ask(&self, step_id: &str) -> Option<&AskOutcome> {
        self.finished_asks.get(step_id)
    }

    /// A gate's journaled outcome — resume does not re-run side effects.
    #[must_use]
    pub fn world_run(&self, step_id: &str) -> Option<&RunOutcome> {
        self.world_runs.get(step_id)
    }

    /// A world read's journaled bytes — resume replays them verbatim.
    #[must_use]
    pub fn world_read(&self, request: &ReadRequest) -> Option<&str> {
        self.world_reads.get(request).map(String::as_str)
    }

    /// How many reads replay from the journal.
    #[must_use]
    pub fn world_read_count(&self) -> usize {
        self.world_reads.len()
    }

    /// Spend recorded by finished asks, plus the run summary when larger.
    /// Seeds [`crate::budget::Budget::with_spent`] on resume.
    #[must_use]
    pub fn spent_usd(&self) -> f64 {
        self.spent_usd
    }

    /// Receipt tokens recorded by finished asks.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.tokens
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "selora-wf-journal-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&root).expect("create the temp workspace");
            Self { root }
        }

        fn path(&self) -> &Path {
            &self.root
        }

        fn run(&self, run_id: &str) -> RunId {
            RunId::from(run_id)
        }

        fn journal_file(&self, run_id: &str) -> PathBuf {
            journal_path(&self.root, &self.run(run_id))
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn sequence_numbers_are_monotonic() {
        let mut journal = Journal::new(RunId::from("run-1"));
        journal.push("run_started", serde_json::json!({ "name": "review" }));
        journal.push("phase", serde_json::json!({ "name": "plan" }));
        assert_eq!(journal.len(), 2);
        assert_eq!(journal.entries()[0].seq, 1);
        assert_eq!(journal.entries()[1].seq, 2);
        assert_eq!(journal.next_seq(), 3);
    }

    #[test]
    fn append_load_round_trips_every_event_kind() {
        let workspace = TempWorkspace::new();
        let run_id = "run-round-trip";
        let read = ReadRequest::files("read", ["src/main.rs"]);
        // Newlines, CRLF, quotes and non-ASCII must survive the JSONL round
        // trip exactly — resume replays these bytes to the script.
        let content = "line one\nline two\r\n\"quoted\" \u{00e9}\u{4e16}\u{754c}";

        let mut journal = Journal::open(workspace.path(), workspace.run(run_id)).expect("open");
        assert_eq!(journal.next_seq(), 1);
        let seq = journal
            .append(JournalEvent::RunStarted {
                script_hash: Some("sha:abc".to_owned()),
                max_cost_usd: 5.0,
            })
            .expect("append run start");
        assert_eq!(seq, 1);
        journal
            .append(JournalEvent::PhaseStarted {
                phase: "review".to_owned(),
            })
            .expect("append phase");
        journal
            .append(JournalEvent::StepStarted {
                step_id: "s1".to_owned(),
                phase: Some("review".to_owned()),
                label: Some("read main".to_owned()),
            })
            .expect("append step start");
        journal
            .append(JournalEvent::WorldRead {
                step_id: "s1".to_owned(),
                read: read.clone(),
                content: content.to_owned(),
            })
            .expect("append read");
        journal
            .append(JournalEvent::AskStarted {
                step_id: "a1".to_owned(),
                agent: "reviewer".to_owned(),
            })
            .expect("append ask start");
        journal
            .append(JournalEvent::AskFinished {
                step_id: "a1".to_owned(),
                agent: "reviewer".to_owned(),
                result: serde_json::json!({ "verdict": "ship", "count": 2 }),
                tokens: 120,
                cost_usd: 0.0125,
            })
            .expect("append ask finish");
        journal
            .append(JournalEvent::WorldRun {
                step_id: "g1".to_owned(),
                command: "cargo".to_owned(),
                args: vec!["test".to_owned()],
                outcome: RunOutcome {
                    exit_code: Some(1),
                    stdout: "ok".to_owned(),
                    stderr: "1 failed".to_owned(),
                    timed_out: false,
                    duration_ms: 4321,
                },
            })
            .expect("append gate");
        journal
            .append(JournalEvent::StepFinished {
                step_id: "s1".to_owned(),
                status: StepStatus::Ok,
                detail: None,
            })
            .expect("append step finish");
        journal
            .append(JournalEvent::RunFinished {
                status: RunStatus::Completed,
                detail: None,
                spent_usd: 0.0125,
            })
            .expect("append run finish");
        assert_eq!(journal.next_seq(), 10);
        assert_eq!(journal.entries().len(), 9);
        drop(journal);

        let loaded = Journal::load(workspace.path(), run_id).expect("load");
        assert!(loaded.corrupt.is_empty(), "{:?}", loaded.corrupt);
        assert_eq!(loaded.entries.len(), 9);
        assert_eq!(loaded.next_seq(), 10);
        assert_eq!(loaded.entries[0].seq, 1);
        assert_eq!(loaded.entries[8].seq, 9);

        let plan = loaded.resume_plan();
        assert!(plan.has_started());
        assert_eq!(plan.run_status(), Some(RunStatus::Completed));
        assert_eq!(plan.phases(), ["review".to_owned()]);
        assert!(plan.is_step_finished("s1"));
        assert!(plan.unfinished_steps().is_empty());
        assert_eq!(plan.world_read(&read), Some(content));
        assert_eq!(plan.world_run("g1").map(RunOutcome::success), Some(false));
        assert_eq!(plan.spent_usd(), 0.0125);
        assert_eq!(plan.tokens(), 120);
        let ask = plan.finished_ask("a1").expect("finished ask");
        assert_eq!(ask.agent, "reviewer");
        assert_eq!(ask.result["verdict"], "ship");
        assert_eq!(ask.tokens, 120);
    }

    #[test]
    fn resume_skips_finished_asks_and_redispatches_the_rest() {
        let workspace = TempWorkspace::new();
        let run_id = "run-resume";
        let mut journal = Journal::open(workspace.path(), workspace.run(run_id)).expect("open");
        // A finished ask and a finished read.
        journal
            .append(JournalEvent::StepStarted {
                step_id: "a1".to_owned(),
                phase: None,
                label: None,
            })
            .expect("append");
        journal
            .append(JournalEvent::AskStarted {
                step_id: "a1".to_owned(),
                agent: "reviewer".to_owned(),
            })
            .expect("append");
        journal
            .append(JournalEvent::AskFinished {
                step_id: "a1".to_owned(),
                agent: "reviewer".to_owned(),
                result: serde_json::json!({ "ok": true }),
                tokens: 10,
                cost_usd: 0.5,
            })
            .expect("append");
        journal
            .append(JournalEvent::StepFinished {
                step_id: "a1".to_owned(),
                status: StepStatus::Ok,
                detail: None,
            })
            .expect("append");
        // An ask that crashed mid-flight.
        journal
            .append(JournalEvent::AskStarted {
                step_id: "a2".to_owned(),
                agent: "fixer".to_owned(),
            })
            .expect("append");
        // A step that never finished (no ask involved).
        journal
            .append(JournalEvent::StepStarted {
                step_id: "g1".to_owned(),
                phase: Some("gate".to_owned()),
                label: None,
            })
            .expect("append");
        drop(journal);

        let loaded = Journal::load(workspace.path(), run_id).expect("load");
        let plan = loaded.resume_plan();
        assert!(!plan.is_run_finished());
        assert_eq!(plan.finished_ask("a1").map(|ask| ask.tokens), Some(10));
        assert!(plan.finished_ask("a2").is_none());
        assert_eq!(plan.unfinished_steps(), ["a2".to_owned(), "g1".to_owned()]);
        assert!(!plan.unfinished_steps().contains(&"a1".to_owned()));
        assert_eq!(plan.spent_usd(), 0.5);
        assert_eq!(plan.tokens(), 10);

        // A resumed run appends after the existing records, not over them.
        let mut resumed = Journal::open(workspace.path(), workspace.run(run_id)).expect("reopen");
        assert_eq!(resumed.next_seq(), 7);
        resumed
            .append(JournalEvent::AskFinished {
                step_id: "a2".to_owned(),
                agent: "fixer".to_owned(),
                result: serde_json::json!({ "ok": true }),
                tokens: 3,
                cost_usd: 0.25,
            })
            .expect("append");
        drop(resumed);

        let plan = Journal::load(workspace.path(), run_id)
            .expect("load")
            .resume_plan();
        assert_eq!(plan.finished_ask("a2").map(|ask| ask.tokens), Some(3));
        assert_eq!(plan.unfinished_steps(), ["g1".to_owned()]);
        assert_eq!(plan.spent_usd(), 0.75);
    }

    #[test]
    fn corrupted_line_is_reported_not_panicked() {
        let workspace = TempWorkspace::new();
        let run_id = "run-corrupt";
        let mut journal = Journal::open(workspace.path(), workspace.run(run_id)).expect("open");
        journal
            .append(JournalEvent::StepStarted {
                step_id: "s1".to_owned(),
                phase: None,
                label: None,
            })
            .expect("append");
        drop(journal);

        // A torn write: a partial record with no newline, an unknown kind, a
        // known kind with a bad payload, and an invalid-UTF-8 line.
        let path = workspace.journal_file(run_id);
        let mut bytes = fs::read(&path).expect("read journal");
        bytes.extend_from_slice(b"{\"seq\":2,\"kind\":\"ask_fin");
        bytes.push(b'\n');
        bytes.extend_from_slice(b"{\"seq\":3,\"kind\":\"step_started\",\"payload\":{}}\n");
        bytes.extend_from_slice(b"{\"seq\":4,\"kind\":\"not_a_kind\",\"payload\":{}}\n");
        bytes.extend_from_slice(b"\xff\xfe not json\n");
        fs::write(&path, bytes).expect("damage the journal");

        let loaded = Journal::load(workspace.path(), run_id).expect("load is not fatal");
        assert_eq!(loaded.entries.len(), 3);
        assert_eq!(loaded.corrupt.len(), 3);
        assert!(loaded.is_damaged());
        assert_eq!(loaded.corrupt[0].line, 2);
        assert!(loaded.corrupt[0].text.starts_with("{\"seq\":2"));
        // The malformed known-kind payload is line 3; the unknown kind is
        // skipped as forward-compatible, not reported.
        assert_eq!(loaded.corrupt[1].line, 3);
        assert!(loaded.corrupt[1].detail.contains("step_id"));
        assert_eq!(loaded.corrupt[2].line, 5);
        assert!(loaded.corrupt[2].text.contains("not json"));
        assert_eq!(loaded.next_seq(), 5);

        // Opening a damaged journal still works and appends after it.
        // That legacy policy applies only to a torn tail now. Interior damage
        // must fail closed instead of silently dropping a completed effect.
        assert!(matches!(
            Journal::open(workspace.path(), workspace.run(run_id)),
            Err(JournalError::Unsafe(_))
        ));
    }

    #[test]
    fn missing_journal_loads_empty() {
        let workspace = TempWorkspace::new();
        let loaded = Journal::load(workspace.path(), "never-ran").expect("load");
        assert!(loaded.is_empty());
        assert!(!loaded.is_damaged());
        assert_eq!(loaded.next_seq(), 1);
        let plan = loaded.resume_plan();
        assert!(!plan.has_started());
        assert!(plan.unfinished_steps().is_empty());
    }

    #[test]
    fn run_ids_cannot_escape_the_runs_directory() {
        let workspace = TempWorkspace::new();
        for bad in ["", "..", "../escape", "a/b", "a\\b", "run id", "."] {
            let error = Journal::open(workspace.path(), bad).expect_err("must reject");
            assert!(
                matches!(error, JournalError::InvalidRunId { .. }),
                "{bad:?}"
            );
        }
        assert!(Journal::open(workspace.path(), "7f2d4c1e-ok_id").is_ok());
    }

    #[test]
    fn open_gitignores_the_selora_directory() {
        let workspace = TempWorkspace::new();
        let _journal = Journal::open(workspace.path(), "run-ignore").expect("open");
        let ignore = workspace.path().join(".selora").join(".gitignore");
        assert_eq!(
            fs::read_to_string(ignore).expect("read .selora/.gitignore"),
            "*\n"
        );
    }

    #[test]
    fn journal_paths_are_workspace_relative() {
        let workspace = Path::new("C:/project");
        let run = RunId::from("abc");
        assert_eq!(
            journal_path(workspace, &run),
            Path::new("C:/project/.selora/workflow-runs/abc/journal.jsonl")
        );
        assert_eq!(
            run_dir(workspace, &run),
            Path::new("C:/project/.selora/workflow-runs/abc")
        );
    }

    #[test]
    fn read_requests_are_structural_replay_keys() {
        let a = ReadRequest::git("diff", ["main", "src/lib.rs"]);
        let b = ReadRequest::git("diff", ["main", "src/lib.rs"]);
        let c = ReadRequest::git("diff", ["main", "src/lb.rs"]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.family, ReadFamily::Git);
        assert_eq!(a.op, "diff");
        assert_eq!(a.args, ["main", "src/lib.rs"]);
    }

    #[test]
    fn raw_records_and_typed_events_share_the_file() {
        let workspace = TempWorkspace::new();
        let run_id = "run-raw";
        let mut journal = Journal::open(workspace.path(), workspace.run(run_id)).expect("open");
        journal
            .append_raw("phase", serde_json::json!({ "name": "plan" }))
            .expect("raw append");
        journal
            .append(JournalEvent::PhaseStarted {
                phase: "done".to_owned(),
            })
            .expect("typed append");
        drop(journal);

        let loaded = Journal::load(workspace.path(), run_id).expect("load");
        assert!(loaded.corrupt.is_empty());
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].kind, "phase");
        assert_eq!(loaded.entries[0].payload["name"], "plan");
        assert_eq!(loaded.entries[1].kind, "phase_started");
        // An unknown (8a-only) kind does not disturb resume.
        let plan = loaded.resume_plan();
        assert_eq!(plan.phases(), ["done".to_owned()]);
    }

    #[test]
    fn only_one_line_per_record() {
        let workspace = TempWorkspace::new();
        let run_id = "run-lines";
        let mut journal = Journal::open(workspace.path(), workspace.run(run_id)).expect("open");
        journal
            .append(JournalEvent::PhaseStarted {
                phase: "one".to_owned(),
            })
            .expect("append");
        journal
            .append(JournalEvent::PhaseStarted {
                phase: "two".to_owned(),
            })
            .expect("append");
        drop(journal);
        let text = fs::read_to_string(workspace.journal_file(run_id)).expect("read");
        assert_eq!(text.lines().count(), 2, "{text:?}");
        assert!(text.ends_with('\n'));
    }
}
