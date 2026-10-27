//! The embedded terminal backend: real Windows ConPTY sessions.
//!
//! One [`TerminalManager`] owns at most [`MAX_TERMINAL_SESSIONS`] live shells.
//! `TerminalCreate` opens a pseudo console through `portable-pty` (the ConPTY
//! implementation WezTerm uses), starts `pwsh` when it is installed and falls
//! back to Windows PowerShell, and roots the child at the open workspace's
//! canonical root. There is no cwd or shell field on the wire: the caller
//! cannot point the process at a path the workspace does not own.
//!
//! # Containment
//!
//! Each child is assigned to its own kill-on-close Job Object through the
//! helper in [`crate::workflows_process`]. Every later descendant joins the
//! job, so closing a session, switching or closing the workspace, or the app
//! exiting kills the whole tree — not just the shell. A job that cannot be
//! created or assigned fails the create and the child is killed; a terminal is
//! never left running uncontained.
//!
//! # Output
//!
//! The ConPTY reader feeds [`OutputBatcher`], which holds incomplete UTF-8
//! sequences, bounds what one event may carry ([`MAX_OUTPUT_BATCH_BYTES`]),
//! bounds the pending buffer ([`MAX_OUTPUT_PENDING_BYTES`]) and marks dropped
//! output with a truncation line. A flusher tick emits whatever is pending
//! within [`OUTPUT_FLUSH_INTERVAL`], so interactive typists see echoes without
//! one IPC call per byte.
//!
//! # Environment
//!
//! `CommandBuilder::env_clear` starts the child with nothing inherited. Only
//! [`ENV_ALLOWLIST`] names are copied from Selora's environment, and a name
//! that looks like a credential (`*_API_KEY`, `*TOKEN*`, `*SECRET*`, ...) is
//! dropped even if it was allowlisted. Gateway keys are never exported to a
//! PTY.
//!
//! # Errors
//!
//! Every failure is a typed [`ProtocolErrorDto`]: `invalid_terminal_size`,
//! `terminal_limit`, `terminal_not_found`, `terminal_cwd_unavailable`,
//! `terminal_spawn_failed`, `terminal_input_too_large`, `terminal_io`.

use selora_protocol::{
    FrontendEvent, ProtocolErrorDto, TerminalCreatedDto, TerminalExitedDto, TerminalOutputDto,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// Hard cap on concurrent sessions (the wire's `terminal_limit`).
pub(crate) const MAX_TERMINAL_SESSIONS: usize = 8;
/// Largest `data` string one `TerminalOutput` event may carry.
pub(crate) const MAX_OUTPUT_BATCH_BYTES: usize = 8 * 1024;
/// Largest byte buffer one session may hold before the oldest output drops.
pub(crate) const MAX_OUTPUT_PENDING_BYTES: usize = 64 * 1024;
/// How often pending output is flushed while the reader is idle.
pub(crate) const OUTPUT_FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// Largest `TerminalInput.data` accepted in one command.
pub(crate) const MAX_INPUT_BYTES: usize = 64 * 1024;
/// ConPTY size bounds (a zero would make the console invalid).
pub(crate) const MIN_TERMINAL_COLS: u16 = 1;
pub(crate) const MAX_TERMINAL_COLS: u16 = 1000;
pub(crate) const MIN_TERMINAL_ROWS: u16 = 1;
pub(crate) const MAX_TERMINAL_ROWS: u16 = 1000;

/// Emitted in place of dropped output so the user knows the stream has a gap.
pub(crate) const TRUNCATION_MARKER: &str = "\r\n[output truncated]\r\n";

/// Where a session sends its events. Production wraps `EventLog::record`;
/// tests collect into a vector.
pub(crate) type Emit = Arc<dyn Fn(FrontendEvent) + Send + Sync>;

// ---------------------------------------------------------------------------
// Environment allowlist.
// ---------------------------------------------------------------------------

/// Names a shell needs to run its startup path. Everything else is denied:
/// in particular the gateway key, which lives only in `selora-settings`.
pub(crate) const ENV_ALLOWLIST: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATH",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_IDENTIFIER",
    "OS",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "PSModulePath",
    "LANG",
    "LC_ALL",
];

/// Credential-shaped names are never copied, even if a future allowlist entry
/// is too broad. Matched case-insensitively against substrings.
fn is_secret_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
}

/// The environment one terminal child receives: allowlisted names from this
/// process plus a fixed terminal identity. No ambient inheritance.
pub(crate) fn allowed_environment() -> Vec<(String, String)> {
    let mut values = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in ENV_ALLOWLIST {
        if !seen.insert(name.to_ascii_uppercase()) || is_secret_env_name(name) {
            continue;
        }
        if let Some(value) = std::env::var_os(name) {
            if let Some(value) = value.to_str() {
                values.push(((*name).to_string(), value.to_string()));
            }
        }
    }
    values.push(("TERM".into(), "xterm-256color".into()));
    values.push(("COLORTERM".into(), "truecolor".into()));
    values.push(("TERM_PROGRAM".into(), "Selora".into()));
    values
}

// ---------------------------------------------------------------------------
// Shell resolution.
// ---------------------------------------------------------------------------

/// The shell a new session starts, as `(wire name, executable)`.
fn resolve_shell() -> (String, std::path::PathBuf) {
    #[cfg(windows)]
    {
        if let Some(pwsh) = find_pwsh() {
            return ("pwsh".into(), pwsh);
        }
        let root = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("windir"))
            .unwrap_or_else(|| "C:\\Windows".into());
        let powershell = std::path::Path::new(&root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if powershell.is_file() {
            return ("powershell".into(), powershell);
        }
        (
            "powershell".into(),
            std::path::PathBuf::from("powershell.exe"),
        )
    }
    #[cfg(not(windows))]
    {
        ("sh".into(), std::path::PathBuf::from("sh"))
    }
}

#[cfg(windows)]
fn find_pwsh() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join("pwsh.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let program_files = std::env::var_os("ProgramFiles")?;
    for relative in ["PowerShell/7/pwsh.exe", "PowerShell/7-preview/pwsh.exe"] {
        let candidate = std::path::Path::new(&program_files).join(relative);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Output batching.
// ---------------------------------------------------------------------------

/// Accumulates PTY bytes and turns them into bounded, complete-UTF-8 strings.
///
/// The batcher owns the retention rule: bytes that do not decode yet stay for
/// the next push (a multibyte character split across two reads must not become
/// replacement characters), and once the pending buffer exceeds
/// [`MAX_OUTPUT_PENDING_BYTES`] the oldest bytes are dropped and the next flush
/// starts with [`TRUNCATION_MARKER`].
pub(crate) struct OutputBatcher {
    pending: Vec<u8>,
    truncated: bool,
    last_flush: Instant,
}

impl OutputBatcher {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            pending: Vec::new(),
            truncated: false,
            last_flush: now,
        }
    }

    /// Append `bytes` and return the chunks that should be emitted now.
    ///
    /// Flushes when the buffer reaches one batch, when the flush interval has
    /// elapsed, or when retention had to drop bytes.
    pub(crate) fn push(&mut self, bytes: &[u8], now: Instant) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > MAX_OUTPUT_PENDING_BYTES {
            let excess = self.pending.len() - MAX_OUTPUT_PENDING_BYTES;
            // Dropping mid-character is fine: `drain_utf8` replaces an invalid
            // byte sequence rather than panicking.
            self.pending.drain(..excess);
            self.truncated = true;
        }
        if self.pending.len() >= MAX_OUTPUT_BATCH_BYTES
            || self.truncated
            || now.saturating_duration_since(self.last_flush) >= OUTPUT_FLUSH_INTERVAL
        {
            return self.flush(now);
        }
        Vec::new()
    }

    /// Emit everything that is complete UTF-8 now. An incomplete tail stays
    /// buffered. An empty buffer emits nothing.
    pub(crate) fn flush(&mut self, now: Instant) -> Vec<String> {
        self.last_flush = now;
        let mut chunks = Vec::new();
        if self.truncated {
            self.truncated = false;
            chunks.push(TRUNCATION_MARKER.to_string());
        }
        for text in self.drain_utf8() {
            for piece in split_batches(&text) {
                chunks.push(piece);
            }
        }
        chunks
    }

    /// Drain the buffer from the front, returning complete text and retaining
    /// an incomplete multibyte tail.
    fn drain_utf8(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = std::mem::take(&mut self.pending);
        loop {
            match std::str::from_utf8(&rest) {
                Ok(text) => {
                    if !text.is_empty() {
                        out.push(text.to_string());
                    }
                    rest = Vec::new();
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    if valid_up_to > 0 {
                        out.push(String::from_utf8_lossy(&rest[..valid_up_to]).into_owned());
                    }
                    match error.error_len() {
                        // The tail is an incomplete character: keep it.
                        None => {
                            rest = rest[valid_up_to..].to_vec();
                            break;
                        }
                        // Invalid bytes: keep them as a replacement sequence
                        // and continue after them.
                        Some(len) => {
                            out.push(
                                String::from_utf8_lossy(&rest[..valid_up_to + len]).into_owned(),
                            );
                            rest = rest[valid_up_to + len..].to_vec();
                        }
                    }
                }
            }
        }
        self.pending = rest;
        out
    }
}

/// Split `text` into pieces of at most [`MAX_OUTPUT_BATCH_BYTES`] bytes,
/// always on a character boundary.
fn split_batches(text: &str) -> Vec<String> {
    if text.len() <= MAX_OUTPUT_BATCH_BYTES {
        return vec![text.to_string()];
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + MAX_OUTPUT_BATCH_BYTES).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        // A single character can never exceed 4 bytes, so this makes progress.
        pieces.push(text[start..end].to_string());
        start = end;
    }
    pieces
}

fn invalid_size() -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "invalid_terminal_size".into(),
        message: format!(
            "Terminal size must be between {MIN_TERMINAL_COLS} and {MAX_TERMINAL_COLS} columns \
             and {MIN_TERMINAL_ROWS} and {MAX_TERMINAL_ROWS} rows."
        ),
        retryable: false,
    }
}

fn validate_size(cols: u16, rows: u16) -> Result<(), ProtocolErrorDto> {
    if !(MIN_TERMINAL_COLS..=MAX_TERMINAL_COLS).contains(&cols)
        || !(MIN_TERMINAL_ROWS..=MAX_TERMINAL_ROWS).contains(&rows)
    {
        return Err(invalid_size());
    }
    Ok(())
}

fn not_found(terminal_id: &str) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "terminal_not_found".into(),
        message: format!("Terminal {terminal_id} is not open."),
        retryable: false,
    }
}

fn limit_reached(max: usize) -> ProtocolErrorDto {
    ProtocolErrorDto {
        code: "terminal_limit".into(),
        message: format!("At most {max} terminals can be open at once. Close one and try again."),
        retryable: false,
    }
}

/// Canonicalize the workspace root a session must start in.
///
/// The terminal wire carries no path, so this is the only cwd a session can
/// get. A root that is gone (workspace closed mid-flight) or not a directory
/// is a typed refusal, never a fallback to the process cwd.
pub(crate) fn workspace_cwd(root: &Path) -> Result<std::path::PathBuf, ProtocolErrorDto> {
    let canonical = std::fs::canonicalize(root).map_err(|error| ProtocolErrorDto {
        code: "terminal_cwd_unavailable".into(),
        message: format!("The workspace folder is not available for a terminal: {error}"),
        retryable: false,
    })?;
    if !canonical.is_dir() {
        return Err(ProtocolErrorDto {
            code: "terminal_cwd_unavailable".into(),
            message: "The workspace root is not a directory.".into(),
            retryable: false,
        });
    }
    Ok(canonical)
}

// ---------------------------------------------------------------------------
// Windows implementation.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::workflows_process::ProcessTree;
    use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
    use selora_workflow::HostError;
    use std::{
        collections::HashMap,
        io::{Read, Write},
        thread,
    };

    struct Session {
        /// Kept behind a lock so a resize owns the pty while other commands run.
        master: Mutex<Box<dyn MasterPty + Send>>,
        writer: Mutex<Box<dyn Write + Send>>,
        killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
        /// The job that makes `close` a tree kill. Dropping it is a tree kill
        /// too (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`).
        tree: ProcessTree,
        /// Set on close or exit; the reader/flusher stop emitting.
        stop: Arc<AtomicBool>,
    }

    /// The live-session registry. Cloned into each waiter thread so a natural
    /// exit removes its own entry.
    #[derive(Clone)]
    pub(crate) struct TerminalManager {
        sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
        max_sessions: usize,
    }

    impl Default for TerminalManager {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TerminalManager {
        pub(crate) fn new() -> Self {
            Self::with_limit(MAX_TERMINAL_SESSIONS)
        }

        /// Tests use a smaller cap so the limit path does not need eight
        /// PowerShell processes; production always uses
        /// [`MAX_TERMINAL_SESSIONS`].
        pub(crate) fn with_limit(max_sessions: usize) -> Self {
            Self {
                sessions: Arc::new(Mutex::new(HashMap::new())),
                max_sessions: max_sessions.max(1),
            }
        }

        /// Start one session at `root` (the open workspace root).
        ///
        /// The returned DTO is what `TerminalCreated` carried; emitting it is
        /// the manager's job, so the event always precedes output.
        pub(crate) fn create(
            &self,
            emit: &Emit,
            root: &Path,
            cols: u16,
            rows: u16,
        ) -> Result<TerminalCreatedDto, ProtocolErrorDto> {
            validate_size(cols, rows)?;
            let cwd = workspace_cwd(root)?;
            {
                let sessions = self.lock_sessions();
                if sessions.len() >= self.max_sessions {
                    return Err(limit_reached(self.max_sessions));
                }
            }

            let (shell, program) = resolve_shell();
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|error| spawn_failed(&error.to_string()))?;

            let mut command = CommandBuilder::new(program.as_os_str());
            command.cwd(&cwd);
            command.env_clear();
            for (name, value) in allowed_environment() {
                command.env(name, value);
            }
            // No profile: the workspace root must stay the shell's cwd, and a
            // user profile must not be able to move it or print secrets.
            command.arg("-NoLogo");
            command.arg("-NoProfile");

            let mut child = pair
                .slave
                .spawn_command(command)
                .map_err(|error| spawn_failed(&error.to_string()))?;
            // The slave end is not needed after the spawn; closing it lets the
            // ConPTY report EOF when the shell exits.
            drop(pair.slave);

            let tree = match ProcessTree::create() {
                Ok(tree) => tree,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(spawn_failed(&error.message));
                }
            };
            let raw_handle = child.as_raw_handle();
            let assigned = match raw_handle {
                Some(handle) => tree.assign(handle),
                None => Err(HostError::new(
                    "the PTY child exposed no process handle, so its tree could not be contained",
                )),
            };
            if let Err(error) = assigned {
                // Fail closed: a shell that might escape its job never runs.
                let _ = child.kill();
                let _ = child.wait();
                return Err(spawn_failed(&error.message));
            }

            let reader = match pair.master.try_clone_reader() {
                Ok(reader) => reader,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(spawn_failed(&error.to_string()));
                }
            };
            let writer = match pair.master.take_writer() {
                Ok(writer) => writer,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(spawn_failed(&error.to_string()));
                }
            };
            let killer = child.clone_killer();

            let terminal_id = uuid::Uuid::new_v4().to_string();
            let created = TerminalCreatedDto {
                terminal_id: terminal_id.clone(),
                shell: shell.clone(),
                cwd: cwd.to_string_lossy().to_string(),
            };
            let stop = Arc::new(AtomicBool::new(false));
            let pending = Arc::new(Mutex::new(OutputBatcher::new(Instant::now())));
            let session = Arc::new(Session {
                master: Mutex::new(pair.master),
                writer: Mutex::new(writer),
                killer: Mutex::new(killer),
                tree,
                stop: Arc::clone(&stop),
            });

            // Register before the created event and before any thread can emit,
            // so a frontend that reacts by closing finds the session.
            self.lock_sessions()
                .insert(terminal_id.clone(), Arc::clone(&session));

            emit(FrontendEvent::TerminalCreated(created.clone()));

            let reader_emit = Arc::clone(emit);
            let flusher_emit = Arc::clone(emit);
            let waiter_emit = Arc::clone(emit);
            let reader_pending = Arc::clone(&pending);
            let flusher_pending = Arc::clone(&pending);
            let waiter_pending = Arc::clone(&pending);
            let reader_stop = Arc::clone(&stop);
            let flusher_stop = Arc::clone(&stop);
            let waiter_stop = Arc::clone(&stop);
            let waiter_id = terminal_id.clone();
            let reader_id = terminal_id.clone();
            let flusher_id = terminal_id.clone();
            let sessions = Arc::clone(&self.sessions);

            let reader_done = Arc::new(AtomicBool::new(false));
            let reader_done_thread = Arc::clone(&reader_done);

            thread::spawn(move || {
                read_output(
                    reader,
                    reader_id,
                    reader_emit,
                    reader_pending,
                    reader_stop,
                    reader_done_thread,
                );
            });

            thread::spawn(move || loop {
                thread::sleep(OUTPUT_FLUSH_INTERVAL);
                flush_pending(&flusher_pending, &flusher_emit, &flusher_id);
                if flusher_stop.load(Ordering::SeqCst) {
                    return;
                }
            });

            thread::spawn(move || {
                let code = match child.wait() {
                    Ok(status) => status.exit_code() as i32,
                    Err(_) => -1,
                };
                // Let the reader drain the ConPTY's buffered bytes first, so
                // the final prompt/error text precedes `TerminalExited`. Only
                // stop it when the pty did not reach EOF on its own.
                let deadline = Instant::now() + Duration::from_millis(500);
                while Instant::now() < deadline && !reader_done.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(5));
                }
                waiter_stop.store(true, Ordering::SeqCst);
                flush_pending(&waiter_pending, &waiter_emit, &waiter_id);
                waiter_emit(FrontendEvent::TerminalExited(TerminalExitedDto {
                    terminal_id: waiter_id.clone(),
                    code,
                }));
                sessions
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .remove(&waiter_id);
            });

            Ok(created)
        }

        /// Write raw input bytes to the shell (keystrokes and pastes alike).
        pub(crate) fn input(&self, terminal_id: &str, data: &str) -> Result<(), ProtocolErrorDto> {
            if data.len() > MAX_INPUT_BYTES {
                return Err(ProtocolErrorDto {
                    code: "terminal_input_too_large".into(),
                    message: format!("Terminal input is limited to {MAX_INPUT_BYTES} bytes."),
                    retryable: false,
                });
            }
            let session = self.session(terminal_id)?;
            let mut writer = session
                .writer
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            writer.write_all(data.as_bytes()).map_err(io_error)?;
            writer.flush().map_err(io_error)
        }

        /// Resize the ConPTY; the shell receives the new window size.
        pub(crate) fn resize(
            &self,
            terminal_id: &str,
            cols: u16,
            rows: u16,
        ) -> Result<(), ProtocolErrorDto> {
            validate_size(cols, rows)?;
            let session = self.session(terminal_id)?;
            let master = session
                .master
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(pty_error)
        }

        /// Close one session: remove it first (so a racing create sees the free
        /// slot), then kill the whole job and the direct child.
        pub(crate) fn close(&self, terminal_id: &str) -> Result<(), ProtocolErrorDto> {
            let session = {
                let mut sessions = self.lock_sessions();
                sessions.remove(terminal_id)
            }
            .ok_or_else(|| not_found(terminal_id))?;
            kill_session(&session);
            Ok(())
        }

        /// Kill every live session. Used on workspace switch, close and app
        /// exit; returns how many sessions were stopped.
        pub(crate) fn close_all(&self) -> usize {
            let drained: Vec<Arc<Session>> = {
                let mut sessions = self.lock_sessions();
                sessions.drain().map(|(_, session)| session).collect()
            };
            let count = drained.len();
            for session in drained {
                kill_session(&session);
            }
            count
        }

        /// How many sessions are registered right now (test and bootstrap use).
        #[allow(dead_code)]
        pub(crate) fn len(&self) -> usize {
            self.lock_sessions().len()
        }

        /// The pty's current size, read back from ConPTY (tests prove a resize
        /// reached the console rather than only this registry).
        #[cfg(test)]
        pub(crate) fn size(&self, terminal_id: &str) -> Option<(u16, u16)> {
            let session = self.session(terminal_id).ok()?;
            let master = session
                .master
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            master.get_size().ok().map(|size| (size.cols, size.rows))
        }

        fn session(&self, terminal_id: &str) -> Result<Arc<Session>, ProtocolErrorDto> {
            self.lock_sessions()
                .get(terminal_id)
                .cloned()
                .ok_or_else(|| not_found(terminal_id))
        }

        fn lock_sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
            self.sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        }
    }

    fn spawn_failed(message: &str) -> ProtocolErrorDto {
        ProtocolErrorDto {
            code: "terminal_spawn_failed".into(),
            message: format!("Could not start a terminal: {message}"),
            retryable: true,
        }
    }

    fn io_error(error: std::io::Error) -> ProtocolErrorDto {
        ProtocolErrorDto {
            code: "terminal_io".into(),
            message: format!("Terminal input/output failed: {error}"),
            retryable: true,
        }
    }

    fn pty_error(error: impl std::fmt::Display) -> ProtocolErrorDto {
        ProtocolErrorDto {
            code: "terminal_io".into(),
            message: format!("Terminal input/output failed: {error}"),
            retryable: true,
        }
    }

    fn kill_session(session: &Session) {
        session.stop.store(true, Ordering::SeqCst);
        // The job is authoritative for the tree; the direct kill is the
        // belt-and-braces fallback when assignment or termination failed.
        let _ = session.tree.terminate();
        let _ = session
            .killer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .kill();
    }

    fn emit_chunks(emit: &Emit, terminal_id: &str, chunks: Vec<String>) {
        for chunk in chunks {
            emit(FrontendEvent::TerminalOutput(TerminalOutputDto {
                terminal_id: terminal_id.to_string(),
                data: chunk,
            }));
        }
    }

    fn flush_pending(pending: &Mutex<OutputBatcher>, emit: &Emit, terminal_id: &str) {
        let mut batcher = pending.lock().unwrap_or_else(|poison| poison.into_inner());
        let chunks = batcher.flush(Instant::now());
        emit_chunks(emit, terminal_id, chunks);
    }

    fn read_output(
        mut reader: Box<dyn Read + Send>,
        terminal_id: String,
        emit: Emit,
        pending: Arc<Mutex<OutputBatcher>>,
        stop: Arc<AtomicBool>,
        reader_done: Arc<AtomicBool>,
    ) {
        let mut buffer = [0u8; 8192];
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let chunks = {
                        let mut batcher =
                            pending.lock().unwrap_or_else(|poison| poison.into_inner());
                        batcher.push(&buffer[..read], Instant::now())
                    };
                    for chunk in chunks {
                        emit(FrontendEvent::TerminalOutput(TerminalOutputDto {
                            terminal_id: terminal_id.clone(),
                            data: chunk,
                        }));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        flush_pending(&pending, &emit, &terminal_id);
        reader_done.store(true, Ordering::SeqCst);
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    /// Placeholder on non-Windows hosts: no ConPTY, so no session may start.
    #[derive(Clone, Default)]
    pub(crate) struct TerminalManager;

    impl TerminalManager {
        pub(crate) fn new() -> Self {
            Self
        }

        pub(crate) fn with_limit(_max_sessions: usize) -> Self {
            Self
        }

        pub(crate) fn create(
            &self,
            _emit: &Emit,
            _root: &Path,
            _cols: u16,
            _rows: u16,
        ) -> Result<TerminalCreatedDto, ProtocolErrorDto> {
            Err(unsupported())
        }

        pub(crate) fn input(
            &self,
            _terminal_id: &str,
            _data: &str,
        ) -> Result<(), ProtocolErrorDto> {
            Err(unsupported())
        }

        pub(crate) fn resize(
            &self,
            _terminal_id: &str,
            _cols: u16,
            _rows: u16,
        ) -> Result<(), ProtocolErrorDto> {
            Err(unsupported())
        }

        pub(crate) fn close(&self, _terminal_id: &str) -> Result<(), ProtocolErrorDto> {
            Err(unsupported())
        }

        pub(crate) fn close_all(&self) -> usize {
            0
        }

        pub(crate) fn len(&self) -> usize {
            0
        }
    }

    fn unsupported() -> ProtocolErrorDto {
        ProtocolErrorDto {
            code: "terminal_unsupported".into(),
            message: "The embedded terminal requires Windows ConPTY.".into(),
            retryable: false,
        }
    }
}

pub(crate) use platform::TerminalManager;

#[cfg(test)]
mod tests {
    use super::*;
    use selora_protocol::FrontendEvent;
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
        thread,
    };

    /// One collected event stream plus the `Emit` the manager calls.
    fn collector() -> (Emit, Arc<Mutex<Vec<FrontendEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let emit: Emit = Arc::new(move |event| {
            sink.lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(event);
        });
        (emit, events)
    }

    fn output_text(events: &Mutex<Vec<FrontendEvent>>) -> String {
        let events = events.lock().unwrap_or_else(|poison| poison.into_inner());
        events
            .iter()
            .filter_map(|event| match event {
                FrontendEvent::TerminalOutput(output) => Some(output.data.clone()),
                _ => None,
            })
            .collect()
    }

    fn wait_for_output(
        events: &Mutex<Vec<FrontendEvent>>,
        needle: &str,
        timeout: Duration,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let text = output_text(events);
            if text.contains(needle) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; output so far:\n{text}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_output_count(
        events: &Mutex<Vec<FrontendEvent>>,
        needle: &str,
        minimum: usize,
        timeout: Duration,
    ) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            let text = output_text(events);
            if text.matches(needle).count() >= minimum {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {minimum} x {needle:?}; output so far:\n{text}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_exit(
        events: &Mutex<Vec<FrontendEvent>>,
        timeout: Duration,
    ) -> Option<selora_protocol::TerminalExitedDto> {
        let deadline = Instant::now() + timeout;
        loop {
            let found = events
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .find_map(|event| match event {
                    FrontendEvent::TerminalExited(exited) => Some(exited.clone()),
                    _ => None,
                });
            if found.is_some() {
                return found;
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("workspace tempdir")
    }

    fn create(
        manager: &TerminalManager,
        emit: &Emit,
        root: &Path,
        events: &Arc<Mutex<Vec<FrontendEvent>>>,
    ) -> TerminalCreatedDto {
        let created = manager.create(emit, root, 80, 24).expect("terminal starts");
        answer_cursor_queries(manager, events, &created.terminal_id);
        created
    }

    /// PowerShell asks the terminal for the cursor position (`ESC[6n`) while
    /// it boots and blocks until the answer arrives. A real frontend (xterm.js)
    /// replies; this harness does the same so the tests exercise the shell
    /// rather than its DSR handshake. The reply is 1-based: row 1, column 1.
    fn answer_cursor_queries(
        manager: &TerminalManager,
        events: &Arc<Mutex<Vec<FrontendEvent>>>,
        terminal_id: &str,
    ) {
        let manager = manager.clone();
        let events = Arc::clone(events);
        let terminal_id = terminal_id.to_string();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut answered = 0usize;
            while Instant::now() < deadline {
                let (queries, exited) = {
                    let events = events.lock().unwrap_or_else(|poison| poison.into_inner());
                    let text: String = events
                        .iter()
                        .filter_map(|event| match event {
                            FrontendEvent::TerminalOutput(output) => Some(output.data.as_str()),
                            _ => None,
                        })
                        .collect();
                    let exited = events
                        .iter()
                        .any(|event| matches!(event, FrontendEvent::TerminalExited(_)));
                    (text.matches("\u{1b}[6n").count(), exited)
                };
                if exited {
                    return;
                }
                while answered < queries {
                    if manager.input(&terminal_id, "\u{1b}[1;1R").is_err() {
                        return;
                    }
                    answered += 1;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
    }

    fn ps_literal(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "''"))
    }

    // ------------------------------------------------------------------
    // Batcher units.
    // ------------------------------------------------------------------

    #[test]
    fn output_batcher_keeps_each_event_under_the_cap() {
        let now = Instant::now();
        let mut batcher = OutputBatcher::new(now);
        let bulk = vec![b'x'; MAX_OUTPUT_PENDING_BYTES * 3];
        let mut chunks = batcher.push(&bulk, now);
        chunks.extend(batcher.flush(now));
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(
                chunk.len() <= MAX_OUTPUT_BATCH_BYTES,
                "chunk of {} bytes exceeds the cap",
                chunk.len()
            );
        }
        assert_eq!(chunks[0], TRUNCATION_MARKER, "dropped output is announced");
        assert!(chunks.len() > 1, "the bulk really was split");
    }

    #[test]
    fn output_batcher_retains_an_incomplete_utf8_character() {
        let now = Instant::now();
        let mut batcher = OutputBatcher::new(now);
        let euro = "€".as_bytes();
        assert!(batcher.push(&euro[..2], now).is_empty());
        assert!(batcher.flush(now).is_empty(), "no partial character leaks");
        let chunks = batcher.push(&euro[2..], now + OUTPUT_FLUSH_INTERVAL);
        assert_eq!(chunks.concat(), "€");
    }

    #[test]
    fn output_batcher_flushes_after_the_interval() {
        let now = Instant::now();
        let mut batcher = OutputBatcher::new(now);
        assert!(batcher.push(b"hello", now).is_empty());
        assert_eq!(
            batcher.push(b" world", now + OUTPUT_FLUSH_INTERVAL),
            vec!["hello world".to_string()]
        );
    }

    #[test]
    fn environment_allowlist_drops_credentials() {
        assert!(is_secret_env_name("OPENAI_API_KEY"));
        assert!(is_secret_env_name("github_token"));
        assert!(!is_secret_env_name("SystemRoot"));
        let names: Vec<String> = allowed_environment()
            .into_iter()
            .map(|(name, _)| name.to_ascii_uppercase())
            .collect();
        assert!(!names.iter().any(|name| is_secret_env_name(name)));
        assert!(names.contains(&"SYSTEMROOT".to_string()));
        assert!(names.contains(&"TERM".to_string()));
    }

    #[test]
    fn size_bounds_are_typed_errors() {
        assert_eq!(
            validate_size(0, 24).expect_err("zero columns").code,
            "invalid_terminal_size"
        );
        assert_eq!(
            validate_size(80, 0).expect_err("zero rows").code,
            "invalid_terminal_size"
        );
        assert!(validate_size(MAX_TERMINAL_COLS, MAX_TERMINAL_ROWS).is_ok());
    }

    #[test]
    fn workspace_cwd_rejects_a_missing_root() {
        let missing = std::env::temp_dir().join(format!(
            "selora-terminal-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        assert_eq!(
            workspace_cwd(&missing).expect_err("missing root").code,
            "terminal_cwd_unavailable"
        );
    }

    // ------------------------------------------------------------------
    // Live ConPTY gates (Windows only).
    // ------------------------------------------------------------------

    #[cfg(windows)]
    #[test]
    fn create_starts_the_shell_at_the_workspace_root() {
        let workspace = workspace();
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        assert!(created.shell == "pwsh" || created.shell == "powershell");
        assert_eq!(
            PathBuf::from(&created.cwd),
            std::fs::canonicalize(workspace.path()).expect("canonical workspace")
        );
        // The first event is Created, before any output.
        {
            let collected = events.lock().unwrap_or_else(|poison| poison.into_inner());
            match collected.first() {
                Some(FrontendEvent::TerminalCreated(event)) => {
                    assert_eq!(event.terminal_id, created.terminal_id)
                }
                other => panic!("created must lead the stream: {other:?}"),
            }
        }
        manager
            .input(&created.terminal_id, "Write-Output 'selora-cwd-probe'\r")
            .expect("input accepted");
        wait_for_output(&events, "selora-cwd-probe", Duration::from_secs(20));
        assert!(
            manager.size(&created.terminal_id).is_some(),
            "the pty is live"
        );
        manager.close_all();
    }

    #[cfg(windows)]
    #[test]
    fn echo_round_trip_reaches_the_frontend() {
        let workspace = workspace();
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        manager
            .input(&created.terminal_id, "Write-Output 'selora-echo-ok'\r")
            .expect("input accepted");
        let text = wait_for_output_count(&events, "selora-echo-ok", 2, Duration::from_secs(20));
        // Two occurrences: the ConPTY echo of the typed line and the shell's
        // actual output. That proves the write reached the shell, not only the
        // console's input path.
        assert!(text.matches("selora-echo-ok").count() >= 2, "{text}");
        manager.close_all();
    }

    #[cfg(windows)]
    #[test]
    fn live_output_never_exceeds_the_batch_cap() {
        let workspace = workspace();
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        manager
            .input(
                &created.terminal_id,
                "1..2000 | ForEach-Object { 'x' * 80 }\r",
            )
            .expect("input accepted");
        // The generated text is ~160 KiB: many batches, each bounded.
        wait_for_output_count(&events, "xxxxxxxx", 1, Duration::from_secs(20));
        thread::sleep(Duration::from_millis(500));
        let events = events.lock().unwrap_or_else(|poison| poison.into_inner());
        let mut seen = 0;
        for event in events.iter() {
            if let FrontendEvent::TerminalOutput(output) = event {
                seen += 1;
                assert!(
                    output.data.len() <= MAX_OUTPUT_BATCH_BYTES,
                    "an output event carried {} bytes",
                    output.data.len()
                );
            }
        }
        assert!(seen > 1, "the stream really was batched");
        drop(events);
        manager.close_all();
    }

    #[cfg(windows)]
    #[test]
    fn resize_reaches_the_conpty() {
        let workspace = workspace();
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        assert_eq!(manager.size(&created.terminal_id), Some((80, 24)));
        manager
            .resize(&created.terminal_id, 120, 40)
            .expect("resize accepted");
        assert_eq!(manager.size(&created.terminal_id), Some((120, 40)));
        assert_eq!(
            manager
                .resize(&created.terminal_id, 0, 40)
                .expect_err("zero columns")
                .code,
            "invalid_terminal_size"
        );
        assert!(
            manager.resize("missing-terminal", 80, 24).is_err(),
            "unknown ids are typed errors"
        );
        manager.close_all();
    }

    #[cfg(windows)]
    #[test]
    fn shell_exit_code_is_reported() {
        let workspace = workspace();
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        manager
            .input(&created.terminal_id, "exit 7\r")
            .expect("input accepted");
        let exited = wait_for_exit(&events, Duration::from_secs(20)).expect("exit event");
        assert_eq!(exited.terminal_id, created.terminal_id);
        assert_eq!(exited.code, 7);
        assert_eq!(manager.len(), 0, "the waiter removed the session");
    }

    #[cfg(windows)]
    #[test]
    fn child_environment_has_no_api_keys_and_no_ambient_secrets() {
        let workspace = workspace();
        let sentinel = format!("selora-terminal-secret-{}", std::process::id());
        // A credential-shaped variable in Selora's own environment must not
        // reach the PTY.
        std::env::set_var("OPENAI_API_KEY", &sentinel);
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        manager
            .input(
                &created.terminal_id,
                "Write-Output ('KEY=' + $env:OPENAI_API_KEY)\r",
            )
            .expect("input accepted");
        let text = wait_for_output(&events, "KEY=", Duration::from_secs(20));
        assert!(
            !text.contains(&sentinel),
            "the API key leaked into the terminal: {text}"
        );
        manager.close_all();
    }

    #[cfg(windows)]
    #[test]
    fn session_limit_refuses_the_next_terminal_until_one_closes() {
        let workspace = workspace();
        let (emit, events) = collector();
        let manager = TerminalManager::with_limit(2);
        assert_eq!(MAX_TERMINAL_SESSIONS, 8, "the product cap is eight");
        let first = create(&manager, &emit, workspace.path(), &events);
        let second = create(&manager, &emit, workspace.path(), &events);
        let error = manager
            .create(&emit, workspace.path(), 80, 24)
            .expect_err("third terminal over the limit");
        assert_eq!(error.code, "terminal_limit");
        manager.close(&first.terminal_id).expect("close first");
        let third = create(&manager, &emit, workspace.path(), &events);
        assert_ne!(third.terminal_id, second.terminal_id);
        assert_eq!(manager.len(), 2);
        manager.close_all();
        assert_eq!(manager.len(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn close_kills_the_shell_and_every_grandchild() {
        let workspace = workspace();
        let pid_file = workspace.path().join("grandchild.pid");
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let created = create(&manager, &emit, workspace.path(), &events);
        // Start a grandchild that would outlive the shell, and record its pid.
        let script = format!(
            "$p = Start-Process -FilePath \"$env:SystemRoot\\System32\\cmd.exe\" \
             -ArgumentList '/c','ping -n 120 127.0.0.1 > NUL' -PassThru; \
             Set-Content -LiteralPath {} -Value $p.Id\r",
            ps_literal(&pid_file)
        );
        manager
            .input(&created.terminal_id, &script)
            .expect("input accepted");
        let grandchild = wait_for_pid(&pid_file, Duration::from_secs(20));
        assert!(process_alive(grandchild), "the grandchild started");

        manager.close(&created.terminal_id).expect("close accepted");
        assert!(
            wait_until_gone(grandchild, Duration::from_secs(10)),
            "grandchild {grandchild} survived the terminal close"
        );
        wait_for_exit(&events, Duration::from_secs(10)).expect("the shell reported exit");
    }

    #[cfg(windows)]
    #[test]
    fn close_all_kills_every_session_on_a_workspace_switch() {
        let workspace = workspace();
        let pid_file = workspace.path().join("switch.pid");
        let (emit, events) = collector();
        let manager = TerminalManager::new();
        let first = create(&manager, &emit, workspace.path(), &events);
        let second = create(&manager, &emit, workspace.path(), &events);
        let script = format!(
            "$p = Start-Process -FilePath \"$env:SystemRoot\\System32\\cmd.exe\" \
             -ArgumentList '/c','ping -n 120 127.0.0.1 > NUL' -PassThru; \
             Set-Content -LiteralPath {} -Value $p.Id\r",
            ps_literal(&pid_file)
        );
        manager
            .input(&first.terminal_id, &script)
            .expect("input accepted");
        let grandchild = wait_for_pid(&pid_file, Duration::from_secs(20));
        assert_eq!(manager.close_all(), 2, "both sessions stopped");
        assert_eq!(manager.len(), 0);
        assert!(
            wait_until_gone(grandchild, Duration::from_secs(10)),
            "grandchild {grandchild} survived the workspace switch"
        );
        assert!(manager.input(&second.terminal_id, "x").is_err());
    }

    #[cfg(windows)]
    fn wait_for_pid(path: &Path, timeout: Duration) -> u32 {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "no pid file at {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[cfg(windows)]
    fn process_alive(pid: u32) -> bool {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            Storage::FileSystem::SYNCHRONIZE,
            System::Threading::{
                OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: `OpenProcess` takes plain access flags and a pid. The handle
        // is closed exactly once; an invalid handle is checked before use.
        unsafe {
            let process = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return false;
            }
            let alive = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
            CloseHandle(process);
            alive
        }
    }

    #[cfg(windows)]
    fn wait_until_gone(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !process_alive(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }
}
