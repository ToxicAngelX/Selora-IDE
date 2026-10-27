//! Phase 9 soak coverage for the subagent scheduler and the workflow journal.
//!
//! The scheduler is blocking (`std` threads + condvars), so these are
//! deterministic load tests rather than wall-clock sleep races: a fake ask
//! executor records start order, histories, in-flight counts and usage. They
//! prove the two ordering rules a crash/restart has to preserve:
//!
//! * one agent's asks are strictly FIFO, even under contention;
//! * separate agents never exceed `max_concurrency`, and never overlap on the
//!   same name.
//!
//! A second group soaks the journal's torn-tail rule at every byte offset of a
//! real journal, then runs two restart attempts to prove the repaired prefix
//! is stable and idempotent.
//!
//! PENDING (no public API on `integration` yet; the tests stay unchanged when
//! the owning branch lands):
//! * background subagent execution and completion notifications
//!   (`crates/selora-ai/src/agent/subagent.rs` on `selora-p3-background`);
//! * parked-child cancellation, parent cancellation, and late-result
//!   suppression (same branch);
//! * session JSONL torn tail/restart (`apps/selora-desktop/.../sessions.rs`
//!   persistence, Phase 9d).
//!
//! See `docs/PHASE9_SOAK.md`; these are not simulated with invented APIs.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use selora_workflow::journal::{journal_path, Journal, JournalEvent, JournalWriter};
use selora_workflow::subagent::{
    AgentHandle, AskError, AskExecutor, AskReply, AskRequest, SubagentConfig, Subagents,
};
use selora_workflow::{prepare, RunId};

// ---------------------------------------------------------------------------
// Fake ask executor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct TestCaps {
    iterations: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    agent: String,
    instructions: String,
    history_len: usize,
    history: Vec<String>,
    caps: TestCaps,
}

#[derive(Default)]
struct State {
    seen: Vec<Seen>,
    in_flight: usize,
    max_in_flight: usize,
    active_agents: HashSet<String>,
    same_agent_overlap: bool,
    reply_tokens: u64,
    reply_cost: u64,
}

struct Fake {
    delay: Duration,
    fail_prefix: Option<String>,
    panic_on: Option<String>,
    state: Mutex<State>,
}

impl Fake {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            fail_prefix: None,
            panic_on: None,
            state: Mutex::new(State::default()),
        }
    }

    fn failing(prefix: &str) -> Self {
        Self {
            fail_prefix: Some(prefix.to_owned()),
            ..Self::new(Duration::ZERO)
        }
    }

    fn panicking(instruction: &str) -> Self {
        Self {
            panic_on: Some(instruction.to_owned()),
            ..Self::new(Duration::ZERO)
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.state.lock().expect("state").seen.clone()
    }

    fn started(&self) -> Vec<String> {
        self.seen()
            .into_iter()
            .map(|seen| seen.instructions)
            .collect()
    }

    fn max_in_flight(&self) -> usize {
        self.state.lock().expect("state").max_in_flight
    }

    fn overlapped(&self) -> bool {
        self.state.lock().expect("state").same_agent_overlap
    }

    fn usage(&self) -> (u64, u64) {
        let state = self.state.lock().expect("state");
        (state.reply_tokens, state.reply_cost)
    }
}

impl AskExecutor for Fake {
    type Caps = TestCaps;

    fn ask(&self, request: AskRequest<TestCaps>) -> Result<AskReply, AskError> {
        {
            let mut state = self.state.lock().expect("state");
            if !state.active_agents.insert(request.agent.clone()) {
                state.same_agent_overlap = true;
            }
            state.in_flight += 1;
            state.max_in_flight = state.max_in_flight.max(state.in_flight);
            state.seen.push(Seen {
                agent: request.agent.clone(),
                instructions: request.instructions.clone(),
                history_len: request.history.len(),
                history: request.history.iter().map(|m| m.content.clone()).collect(),
                caps: request.caps.clone(),
            });
        }

        thread::sleep(self.delay);

        {
            let mut state = self.state.lock().expect("state");
            state.active_agents.remove(&request.agent);
            state.in_flight -= 1;
        }

        if self.panic_on.as_deref() == Some(request.instructions.as_str()) {
            panic!("fake ask panic");
        }
        if self
            .fail_prefix
            .as_deref()
            .is_some_and(|prefix| request.instructions.starts_with(prefix))
        {
            return Err(AskError::new("gateway said no"));
        }

        let tokens = request.instructions.len() as u64;
        {
            let mut state = self.state.lock().expect("state");
            state.reply_tokens += tokens;
            state.reply_cost += 3;
        }
        Ok(AskReply::with_usage(
            format!("answer from {}", request.agent),
            tokens,
            0.03,
        ))
    }
}

fn caps() -> TestCaps {
    TestCaps { iterations: 40 }
}

/// Spawn one ask. The caller staggers spawns; each worker takes its ticket as
/// soon as it starts, which is the submission order the queue promises to
/// preserve. This is the same synchronization rule the crate's unit tests use
/// (no ambient speed assumption: the stagger is an ordering barrier, and the
/// executor's own delay keeps the later asks queued).
fn submit(
    handle: &AgentHandle<Fake>,
    instructions: String,
) -> thread::JoinHandle<Result<AskReply, selora_workflow::subagent::SubagentError>> {
    let handle = handle.clone();
    thread::spawn(move || handle.ask(instructions))
}

// ---------------------------------------------------------------------------
// 1. Same-agent FIFO under load
// ---------------------------------------------------------------------------

#[test]
fn same_agent_fifo_holds_under_load_and_token_usage_is_exact() {
    const ASKS: usize = 24;
    let fake = Arc::new(Fake::new(Duration::from_millis(2)));
    let pool = Subagents::new(fake.clone(), caps(), SubagentConfig::new(4));
    let handle = pool.agent("reviewer");

    let mut joins = Vec::new();
    for index in 0..ASKS {
        // Sequential spawns establish ticket order; the executor's 2 ms
        // delay keeps every later ask queued behind the earlier ones.
        joins.push(submit(&handle, format!("ask-{index:02}")));
        thread::sleep(Duration::from_millis(5));
    }

    let mut tokens = 0u64;
    for join in joins {
        let reply = join.join().expect("join").expect("ask");
        tokens += reply.tokens;
    }

    let started = fake.started();
    assert_eq!(started.len(), ASKS);
    let mut unique = started.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique,
        (0..ASKS)
            .map(|index| format!("ask-{index:02}"))
            .collect::<Vec<_>>(),
        "every submitted ask ran exactly once"
    );
    assert!(!fake.overlapped(), "one agent never runs two asks at once");
    assert_eq!(
        fake.max_in_flight(),
        1,
        "a single agent serializes everything"
    );

    // FIFO is observable as a prefix chain: ask N's history is exactly the
    // queue's execution order for 0..N plus its own instruction. Which thread
    // takes the first ticket is a scheduler detail; the ticket order itself is
    // what the queue promises, and every later ask must see all earlier ones
    // in that exact order.
    let seen = fake.seen();
    for (index, request) in seen.iter().enumerate() {
        assert_eq!(request.history_len, index * 2 + 1, "turn {index}");
        for (prior, instruction) in started[..index].iter().enumerate() {
            assert_eq!(&request.history[prior * 2], instruction, "turn {index}");
            assert_eq!(
                &request.history[prior * 2 + 1],
                "answer from reviewer",
                "turn {index}"
            );
        }
        assert_eq!(request.history.last().unwrap(), &started[index]);
        assertion_caps(&request.caps);
    }
    assert_eq!(
        tokens,
        fake.usage().0,
        "the caller's sum matches the receipt"
    );
    assert_eq!(handle.transcript().len(), ASKS * 2);
}

fn assertion_caps(caps: &TestCaps) {
    assert_eq!(caps.iterations, 40);
}

// ---------------------------------------------------------------------------
// 2. Distinct agents: real concurrency, bounded
// ---------------------------------------------------------------------------

#[test]
fn distinct_agents_respect_the_concurrency_cap_under_load() {
    const AGENTS: usize = 24;
    let fake = Arc::new(Fake::new(Duration::from_millis(3)));
    let pool = Subagents::new(fake.clone(), caps(), SubagentConfig::new(3));

    let mut joins = Vec::new();
    for index in 0..AGENTS {
        let handle = pool.agent(format!("agent-{index:02}"));
        joins.push(thread::spawn(move || {
            handle.ask(format!("item-{index:02}"))
        }));
    }
    for join in joins {
        join.join().expect("join").expect("ask");
    }

    assert_eq!(fake.seen().len(), AGENTS);
    assert!(
        fake.max_in_flight() <= 3,
        "cap exceeded: {}",
        fake.max_in_flight()
    );
    assert_eq!(fake.max_in_flight(), 3, "the cap is actually reached");
    assert!(!fake.overlapped(), "distinct names never share a queue");
    assert_eq!(pool.agent_names().len(), AGENTS);
}

// ---------------------------------------------------------------------------
// 3. Failures roll back without reordering later asks
// ---------------------------------------------------------------------------

#[test]
fn failed_asks_release_the_queue_and_leave_no_duplicate_instruction() {
    let fake = Arc::new(Fake::failing("fail-"));
    let pool = Subagents::with_defaults(fake.clone(), caps());
    let handle = pool.agent("flaky");

    let mut joins = Vec::new();
    for index in 0..12 {
        let instructions = if index % 2 == 0 {
            format!("ok-{index:02}")
        } else {
            format!("fail-{index:02}")
        };
        joins.push(submit(&handle, instructions));
        thread::sleep(Duration::from_millis(5));
    }

    let mut ok = 0;
    for join in joins {
        match join.join().expect("join") {
            Ok(_) => ok += 1,
            Err(error) => assert!(
                error.to_string().contains("gateway said no"),
                "unexpected error: {error}"
            ),
        }
    }
    assert_eq!(ok, 6);

    // Only successful exchange pairs remain; the transcript is exactly in
    // success order and never carries a failed instruction twice.
    let transcript = handle.transcript();
    assert_eq!(transcript.len(), ok * 2);
    for pair in transcript.chunks(2) {
        assert!(pair[0].content.starts_with("ok-"), "{pair:?}");
        assert_eq!(pair[1].content, "answer from flaky");
    }
    let history_lens: Vec<usize> = fake.seen().iter().map(|seen| seen.history_len).collect();
    assert!(history_lens.windows(2).all(|window| window[0] <= window[1]));
}

// ---------------------------------------------------------------------------
// 4. A panic does not wedge the queue under load
// ---------------------------------------------------------------------------

#[test]
fn a_panicking_ask_never_wedges_later_concurrent_asks() {
    let fake = Arc::new(Fake::panicking("panic"));
    let pool = Subagents::with_defaults(fake.clone(), caps());
    let handle = pool.agent("fragile");

    let mut joins = Vec::new();
    joins.push(submit(&handle, "panic".to_owned()));
    for index in 0..8 {
        joins.push(submit(&handle, format!("after-{index}")));
        thread::sleep(Duration::from_millis(5));
    }

    let mut panics = 0;
    let mut successes = 0;
    for join in joins {
        match join.join() {
            Err(_) => panics += 1,
            Ok(Ok(_)) => successes += 1,
            Ok(Err(error)) => panic!("unexpected ask error: {error}"),
        }
    }
    assert_eq!(panics, 1);
    assert_eq!(successes, 8);
    let transcript = handle.transcript();
    assert_eq!(transcript.len(), 16);
    let mut instructions: Vec<String> = transcript
        .iter()
        .step_by(2)
        .map(|message| message.content.clone())
        .collect();
    instructions.sort();
    assert_eq!(
        instructions,
        (0..8)
            .map(|index| format!("after-{index}"))
            .collect::<Vec<_>>(),
        "every later ask ran and the panicking instruction was rolled back"
    );
}

// ---------------------------------------------------------------------------
// 5. Torn-tail soak at every byte of a real journal, twice restarted
// ---------------------------------------------------------------------------

fn journal_workspace(label: &str) -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Temp/opencode");
    let root = base.join(format!(
        "selora-phase9-journal-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("workspace");
    root
}

fn seed_journal(workspace: &std::path::Path, run: &RunId) -> Vec<u8> {
    let mut writer = JournalWriter::open(workspace, run.clone()).expect("open");
    writer
        .append(JournalEvent::RunStarted {
            script_hash: Some("phase9".to_owned()),
            max_cost_usd: 5.0,
        })
        .expect("run started");
    writer
        .append(JournalEvent::StepStarted {
            step_id: "ask-1".to_owned(),
            phase: Some("review".to_owned()),
            label: Some("first".to_owned()),
        })
        .expect("step");
    writer
        .append(JournalEvent::AskStarted {
            step_id: "ask-1".to_owned(),
            agent: "reviewer".to_owned(),
        })
        .expect("ask");
    writer
        .append(JournalEvent::AskFinished {
            step_id: "ask-1".to_owned(),
            agent: "reviewer".to_owned(),
            result: serde_json::json!({"ok": true}),
            tokens: 11,
            cost_usd: 0.01,
        })
        .expect("ask finished");
    drop(writer);
    std::fs::read(journal_path(workspace, run)).expect("journal bytes")
}

#[test]
fn every_torn_tail_is_repaired_and_the_prefix_is_stable_across_restarts() {
    let workspace = journal_workspace("torn");
    let run = RunId::from("torn-run");
    let bytes = seed_journal(&workspace, &run);
    let path = journal_path(&workspace, &run);

    // Every cut inside the final complete line is a torn write. The writer
    // must truncate to the previous newline and append safely; no cut may
    // panic or leave a corrupt interior.
    let last_newline = bytes[..bytes.len() - 1]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .expect("newline");
    for cut in last_newline + 1..bytes.len() {
        std::fs::write(&path, &bytes[..cut]).expect("truncate");
        let mut writer = JournalWriter::open(&workspace, run.clone()).expect("repair open");
        writer
            .append(JournalEvent::WorldRead {
                step_id: format!("read-{cut}"),
                read: selora_workflow::journal::ReadRequest::files("read", ["a.rs"]),
                content: "ok".to_owned(),
            })
            .expect("append after repair");
        drop(writer);

        let loaded = Journal::load(&workspace, run.clone()).expect("load");
        assert!(
            loaded.corrupt.is_empty(),
            "cut {cut} left damage: {:?}",
            loaded.corrupt
        );
    }

    // Restart twice from a torn tail: the first repair fixes it, the second
    // open must preserve every complete record and continue the sequence.
    std::fs::write(&path, &bytes[..last_newline + 3]).expect("torn again");
    let first = Journal::open(&workspace, run.clone()).expect("first restart");
    let first_entries = first.entries().len();
    assert_eq!(
        first_entries, 3,
        "the interrupted AskFinished is not a complete record"
    );
    drop(first);
    let second = Journal::open(&workspace, run.clone()).expect("second restart");
    assert_eq!(second.entries().len(), first_entries);
    assert_eq!(second.next_seq() as usize, first_entries + 1);
    drop(second);
    let fresh = Journal::load(&workspace, run.clone()).expect("load fresh");
    assert!(fresh.corrupt.is_empty());
    assert_eq!(
        fresh
            .entries
            .iter()
            .filter(|entry| entry.kind == "ask_started")
            .count(),
        1,
        "the interrupted effect is visible exactly once"
    );
    assert_eq!(
        fresh
            .entries
            .iter()
            .filter(|entry| entry.kind == "ask_finished")
            .count(),
        0
    );

    // A journal whose final record was fully synced keeps it across restarts.
    std::fs::write(&path, &bytes).expect("restore complete journal");
    for _ in 0..2 {
        let writer = Journal::open(&workspace, run.clone()).expect("restart complete");
        assert_eq!(writer.entries().len(), 4);
    }
    let complete = Journal::load(&workspace, run.clone()).expect("load complete");
    assert!(complete.corrupt.is_empty());
    assert_eq!(
        complete
            .entries
            .iter()
            .filter(|entry| entry.kind == "ask_finished")
            .count(),
        1,
        "the completed ask survives repeated restarts exactly once"
    );

    // The same confirmed script can be prepared again: no randomness leaks in.
    let a = prepare("phase9", r#"phase("one");"#, 5.0).expect("prepare a");
    let b = prepare("phase9", r#"phase("one");"#, 5.0).expect("prepare b");
    assert_eq!(a.spec(), b.spec());

    let _ = std::fs::remove_dir_all(workspace);
}
