//! Named-agent identities and the subagent scheduler (ARCHITECTURE.md §18.2–
//! 18.3, ROADMAP.md Phase 8b).
//!
//! A subagent is a conversation, not a process. Names are identities: unique
//! within a run and the key resume matches steps by.
//!
//! Two layers live here:
//!
//! * [`AgentRegistry`] — 8a's synchronous identity bookkeeping: the facade
//!   interns `agent(name, persona)` so one name can never silently fork into
//!   two personas. First persona wins (one name is one conversation).
//! * [`Subagents`] — 8b's scheduler. Each named agent owns one [`Message`]
//!   history; each ask is one Phase-5 turn with its own caps. Concurrent asks
//!   on one agent queue **FIFO**; separate agents run **concurrently** up to
//!   [`SubagentConfig::max_concurrency`] (default 4,
//!   `workflow.max_concurrency`).
//!
//! # Why this is blocking, and why the caps are generic
//!
//! This crate is a standalone workspace with no path dependencies on other
//! Selora crates (see `Cargo.toml`), and its engine is blocking by contract
//! (ARCHITECTURE.md §18.4: `run_workflow` runs on a `spawn_blocking` worker,
//! `Host` is blocking). The scheduler therefore uses `std` threads and
//! condvars rather than tokio, and carries the §17.3 caps as an associated
//! type: 8c instantiates [`Subagents`] with `selora_ai::TurnCaps` without this
//! crate depending on `selora-ai`, and the tests instantiate it with a tiny
//! local type. [`Message`] maps one-to-one onto `selora_ai::ChatMessage`
//! (`role` + `content`) at the host boundary.
//!
//! The actual turn is an injected [`AskExecutor`] — the shell wires its
//! agent loop there, and the tests wire a fake. Nothing here performs HTTP or
//! touches the filesystem, which is what makes the ordering tests
//! deterministic.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

/// §18.3's default: four subagents in flight (`workflow.max_concurrency`).
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

// ---------------------------------------------------------------------------
// 8a: identity bookkeeping
// ---------------------------------------------------------------------------

/// One named agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    /// Run-unique name (`"reviewer"`, `"default"`).
    pub name: String,
    /// The persona the script declared on first use.
    pub persona: Option<String>,
}

/// The run's agent registry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRegistry {
    agents: Vec<AgentRef>,
}

impl AgentRegistry {
    /// Intern `name`: the first call fixes the persona, later calls with the
    /// same name return the same identity (even if a different persona was
    /// passed — one name is one conversation).
    pub fn intern(&mut self, name: &str, persona: Option<&str>) -> AgentRef {
        if let Some(existing) = self.agents.iter().find(|agent| agent.name == name) {
            return existing.clone();
        }
        let agent = AgentRef {
            name: name.to_owned(),
            persona: persona.map(str::to_owned),
        };
        self.agents.push(agent.clone());
        agent
    }

    /// Identities in first-use order (the panel's subagent list).
    #[must_use]
    pub fn agents(&self) -> &[AgentRef] {
        &self.agents
    }

    /// Number of distinct identities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether no agent has been used yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}

// ---------------------------------------------------------------------------
// 8b: messages, the ask seam, and the scheduler
// ---------------------------------------------------------------------------

/// Who authored one conversation message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The persona/system prompt.
    System,
    /// The script's instructions.
    User,
    /// The model's answer.
    Assistant,
}

/// One message in a named agent's conversation.
///
/// The field shape is deliberately `selora_ai::ChatMessage`'s (`role` +
/// `content`); 8c converts at the host boundary without loss for the plain
/// messages the orchestrator keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who authored it.
    pub role: Role,
    /// Its text.
    pub content: String,
}

impl Message {
    /// A system/persona message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// A user/instructions message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An assistant answer.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One ask, as handed to the executor — everything a turn needs, with no
/// ambient state.
#[derive(Debug, Clone, PartialEq)]
pub struct AskRequest<C> {
    /// The named agent's identity.
    pub agent: String,
    /// The agent's persona, when the script declared one.
    pub persona: Option<String>,
    /// The workflow step this ask journals under, when the facade has one.
    pub step_id: Option<String>,
    /// The script's instructions for this ask.
    pub instructions: String,
    /// The conversation after this ask's user message was appended — the
    /// exact message list the turn sends.
    pub history: Vec<Message>,
    /// This ask's own §17.3 limits (`selora_ai::TurnCaps` at 8c).
    pub caps: C,
}

/// One ask's answer plus its receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskReply {
    /// The model's final answer text (8a validates it against the script's
    /// declared type at the facade boundary).
    pub content: String,
    /// Receipt tokens, for the workflow budget and the cost meter.
    #[serde(default)]
    pub tokens: u64,
    /// Estimated USD, for the workflow budget and the cost meter.
    #[serde(default)]
    pub cost_usd: f64,
}

impl AskReply {
    /// A reply with no receipt figures (tests, or a turn without receipts).
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            tokens: 0,
            cost_usd: 0.0,
        }
    }

    /// A reply carrying its receipt.
    #[must_use]
    pub fn with_usage(content: impl Into<String>, tokens: u64, cost_usd: f64) -> Self {
        Self {
            content: content.into(),
            tokens,
            cost_usd,
        }
    }
}

/// A turn-level ask failure, carried through the scheduler unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("subagent ask failed: {message}")]
pub struct AskError {
    /// What the loop reported.
    pub message: String,
}

impl AskError {
    /// A failure with a human-readable reason.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Errors from the scheduler itself.
#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    /// The underlying turn failed.
    #[error(transparent)]
    Ask(#[from] AskError),
    /// A name was re-declared with a different persona.
    #[error("agent {name:?} already exists with a different persona")]
    PersonaMismatch {
        /// The agent name.
        name: String,
    },
}

/// The seam between the scheduler and the Phase-5 loop.
///
/// The production implementation drives `spawn_agent_turn` with
/// `request.history` (converted to `ChatMessage`) + `request.instructions`
/// under `request.caps`, and maps the turn's final answer and receipt into
/// [`AskReply`]. Tests implement it with a fake that records requests and
/// sleeps. The associated `Caps` keeps this crate dependency-free while still
/// carrying each ask's real `TurnCaps`.
pub trait AskExecutor: Send + Sync {
    /// This crate's view of `TurnCaps` (8c uses the real type).
    type Caps: Clone + Send + Sync;

    /// Run one ask to completion.
    ///
    /// # Errors
    /// Any refusal or failure of the underlying turn.
    fn ask(&self, request: AskRequest<Self::Caps>) -> Result<AskReply, AskError>;
}

/// Scheduler configuration (`workflow.max_concurrency`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentConfig {
    max_concurrency: usize,
}

impl SubagentConfig {
    /// A config with `max_concurrency` in-flight asks. Zero is normalized to
    /// one — a workflow that cannot make progress is never the intent, and a
    /// deadlocked run would still have spent money.
    #[must_use]
    pub const fn new(max_concurrency: usize) -> Self {
        Self {
            max_concurrency: if max_concurrency == 0 {
                1
            } else {
                max_concurrency
            },
        }
    }

    /// How many asks may be in flight across distinct agents.
    #[must_use]
    pub const fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONCURRENCY)
    }
}

/// The named-agent scheduler for one workflow run.
pub struct Subagents<E: AskExecutor> {
    executor: Arc<E>,
    caps: E::Caps,
    config: SubagentConfig,
    permits: Arc<Permits>,
    agents: Mutex<HashMap<String, Arc<AgentSlot>>>,
}

impl<E: AskExecutor> fmt::Debug for Subagents<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Subagents")
            .field("max_concurrency", &self.config.max_concurrency)
            .field("agents", &self.agent_names())
            .finish_non_exhaustive()
    }
}

impl<E: AskExecutor> Subagents<E> {
    /// Build the scheduler. `caps` is the per-ask default built from the
    /// active model and `settings.agent`; every ask clones it, so one ask can
    /// never mutate another's limits.
    pub fn new(executor: Arc<E>, caps: E::Caps, config: SubagentConfig) -> Self {
        Self {
            executor,
            caps,
            config,
            permits: Arc::new(Permits::new(config.max_concurrency)),
            agents: Mutex::new(HashMap::new()),
        }
    }

    /// Build with the §18.3 default concurrency.
    pub fn with_defaults(executor: Arc<E>, caps: E::Caps) -> Self {
        Self::new(executor, caps, SubagentConfig::default())
    }

    /// The per-ask caps this scheduler hands out.
    #[must_use]
    pub fn caps(&self) -> &E::Caps {
        &self.caps
    }

    /// The concurrency configuration.
    #[must_use]
    pub fn config(&self) -> SubagentConfig {
        self.config
    }

    /// The handle for `name`, creating its conversation on first use.
    ///
    /// Idempotent: the same name is always the same conversation.
    pub fn agent(&self, name: impl Into<String>) -> AgentHandle<E> {
        let name = name.into();
        let slot = self.slot_plain(&name);
        self.handle(slot)
    }

    /// The handle for `name`, declaring its persona on first use.
    ///
    /// A persona cannot change after the conversation exists: names are
    /// identities, and resume matches steps by them.
    ///
    /// # Errors
    /// [`SubagentError::PersonaMismatch`] when the name already exists with a
    /// different persona.
    pub fn agent_with_persona(
        &self,
        name: impl Into<String>,
        persona: impl Into<String>,
    ) -> Result<AgentHandle<E>, SubagentError> {
        let name = name.into();
        let persona = persona.into();
        let slot = self.slot_checked(&name, Some(&persona))?;
        Ok(self.handle(slot))
    }

    /// Convenience: one ask on `name`'s conversation.
    ///
    /// # Errors
    /// See [`AgentHandle::ask`].
    pub fn ask(
        &self,
        name: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<AskReply, SubagentError> {
        self.agent(name).ask(instructions)
    }

    /// Every named agent, sorted — the panel's agent list.
    #[must_use]
    pub fn agent_names(&self) -> Vec<String> {
        let mut names: Vec<String> = lock_agents(&self.agents).keys().cloned().collect();
        names.sort();
        names
    }

    fn handle(&self, slot: Arc<AgentSlot>) -> AgentHandle<E> {
        AgentHandle {
            inner: slot,
            executor: Arc::clone(&self.executor),
            permits: Arc::clone(&self.permits),
            caps: self.caps.clone(),
        }
    }

    /// Create `name`'s conversation if absent; return the existing one too.
    fn slot_plain(&self, name: &str) -> Arc<AgentSlot> {
        let mut agents = lock_agents(&self.agents);
        if let Some(existing) = agents.get(name) {
            return Arc::clone(existing);
        }
        let slot = new_slot(name, None);
        agents.insert(name.to_owned(), Arc::clone(&slot));
        slot
    }

    /// Like [`Subagents::slot_plain`], but rejects a conflicting persona.
    fn slot_checked(
        &self,
        name: &str,
        persona: Option<&str>,
    ) -> Result<Arc<AgentSlot>, SubagentError> {
        let mut agents = lock_agents(&self.agents);
        if let Some(existing) = agents.get(name) {
            if let Some(persona) = persona {
                if existing.persona.as_deref() != Some(persona) {
                    return Err(SubagentError::PersonaMismatch {
                        name: name.to_owned(),
                    });
                }
            }
            return Ok(Arc::clone(existing));
        }
        let slot = new_slot(name, persona);
        agents.insert(name.to_owned(), Arc::clone(&slot));
        Ok(slot)
    }
}

fn new_slot(name: &str, persona: Option<&str>) -> Arc<AgentSlot> {
    Arc::new(AgentSlot {
        name: name.to_owned(),
        persona: persona.map(str::to_owned),
        queue: Mutex::new(Queue::default()),
        wake: Condvar::new(),
    })
}

/// One named agent's identity and conversation.
struct AgentSlot {
    name: String,
    persona: Option<String>,
    /// Ticket queue: waiters are served strictly in the order they arrived.
    queue: Mutex<Queue>,
    /// Signalled when the serving ticket finishes.
    wake: Condvar,
}

#[derive(Default)]
struct Queue {
    messages: Vec<Message>,
    next_ticket: u64,
    serving: u64,
}

/// A cheap, cloneable handle to one named agent's conversation.
pub struct AgentHandle<E: AskExecutor> {
    inner: Arc<AgentSlot>,
    executor: Arc<E>,
    permits: Arc<Permits>,
    caps: E::Caps,
}

impl<E: AskExecutor> Clone for AgentHandle<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            executor: Arc::clone(&self.executor),
            permits: Arc::clone(&self.permits),
            caps: self.caps.clone(),
        }
    }
}

impl<E: AskExecutor> fmt::Debug for AgentHandle<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentHandle")
            .field("name", &self.inner.name)
            .field("persona", &self.inner.persona)
            .finish_non_exhaustive()
    }
}

impl<E: AskExecutor> AgentHandle<E> {
    /// The agent's name (its identity within the run).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// The agent's persona, if the script declared one.
    #[must_use]
    pub fn persona(&self) -> Option<&str> {
        self.inner.persona.as_deref()
    }

    /// Ask this agent, under the scheduler's default per-ask caps.
    ///
    /// Concurrent asks on the same agent queue FIFO; asks on different agents
    /// run concurrently up to the configured cap.
    ///
    /// # Errors
    /// [`SubagentError::Ask`] when the underlying turn fails (the pending
    /// instruction is rolled back out of the conversation so a retry does not
    /// duplicate it).
    pub fn ask(&self, instructions: impl Into<String>) -> Result<AskReply, SubagentError> {
        self.dispatch(None, instructions.into(), self.caps.clone())
    }

    /// Ask this agent, journaling under `step_id` (resume's key).
    ///
    /// # Errors
    /// See [`AgentHandle::ask`].
    pub fn ask_at(
        &self,
        step_id: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<AskReply, SubagentError> {
        self.dispatch(Some(step_id.into()), instructions.into(), self.caps.clone())
    }

    /// Ask this agent under explicit caps for this ask only.
    ///
    /// # Errors
    /// See [`AgentHandle::ask`].
    pub fn ask_with_caps(
        &self,
        instructions: impl Into<String>,
        caps: E::Caps,
    ) -> Result<AskReply, SubagentError> {
        self.dispatch(None, instructions.into(), caps)
    }

    fn dispatch(
        &self,
        step_id: Option<String>,
        instructions: String,
        caps: E::Caps,
    ) -> Result<AskReply, SubagentError> {
        // FIFO first: take the next ticket and wait for our turn. Only then
        // take a global permit, so the concurrency cap cannot reorder the
        // queue. The queue lock is released for the turn itself.
        let mut queue = self.acquire_turn();
        queue.messages.push(Message::user(instructions.clone()));
        let history = queue.messages.clone();
        drop(queue);

        let request = AskRequest {
            agent: self.inner.name.clone(),
            persona: self.inner.persona.clone(),
            step_id,
            instructions,
            history,
            caps,
        };

        let permit = self.permits.acquire();
        // A panic in the executor must not wedge this agent's queue: advance
        // the ticket and roll back before unwinding.
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.executor.ask(request)));
        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                self.finish_turn(None);
                drop(permit);
                std::panic::resume_unwind(payload);
            }
        };
        match result {
            Ok(reply) => {
                self.finish_turn(Some(&reply));
                drop(permit);
                Ok(reply)
            }
            Err(error) => {
                // Roll the pending user message back out: a transport
                // failure must not leave duplicated instructions behind.
                self.finish_turn(None);
                drop(permit);
                Err(SubagentError::Ask(error))
            }
        }
    }

    /// Wait for this ask's FIFO turn and return the locked queue.
    fn acquire_turn(&self) -> MutexGuard<'_, Queue> {
        let mut queue = lock_queue(&self.inner.queue);
        let ticket = queue.next_ticket;
        queue.next_ticket = queue.next_ticket.wrapping_add(1);
        while queue.serving != ticket {
            queue = self
                .inner
                .wake
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        queue
    }

    /// Append the answer (or drop the pending instruction) and hand the
    /// queue to the next ticket.
    fn finish_turn(&self, reply: Option<&AskReply>) {
        let mut queue = lock_queue(&self.inner.queue);
        match reply {
            Some(reply) => queue
                .messages
                .push(Message::assistant(reply.content.clone())),
            None => {
                queue.messages.pop();
            }
        }
        queue.serving = queue.serving.wrapping_add(1);
        self.inner.wake.notify_all();
    }

    /// A snapshot of this agent's conversation (tests, the panel's transcript
    /// view). While an ask is in flight the snapshot includes that ask's
    /// pending user message and not its answer.
    #[must_use]
    pub fn transcript(&self) -> Vec<Message> {
        lock_queue(&self.inner.queue).messages.clone()
    }
}

/// A poisoned lock is recovered: a panic elsewhere must not wedge every later
/// ask in the run.
fn lock_agents(
    agents: &Mutex<HashMap<String, Arc<AgentSlot>>>,
) -> MutexGuard<'_, HashMap<String, Arc<AgentSlot>>> {
    agents
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_queue(queue: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The global concurrency cap: a counting semaphore that blocking asks acquire
/// around the executor call.
struct Permits {
    capacity: usize,
    in_use: Mutex<usize>,
    released: Condvar,
}

impl Permits {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            in_use: Mutex::new(0),
            released: Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> PermitGuard {
        let mut in_use = self
            .in_use
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *in_use >= self.capacity {
            in_use = self
                .released
                .wait(in_use)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *in_use += 1;
        PermitGuard {
            permits: Arc::clone(self),
        }
    }
}

/// Releases one concurrency permit, even while unwinding.
struct PermitGuard {
    permits: Arc<Permits>,
}

impl Drop for PermitGuard {
    fn drop(&mut self) {
        let mut in_use = self
            .permits
            .in_use
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *in_use = in_use.saturating_sub(1);
        self.permits.released.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn intern_is_idempotent_and_first_persona_wins() {
        let mut registry = AgentRegistry::default();
        let first = registry.intern("reviewer", Some("terse"));
        let again = registry.intern("reviewer", Some("verbose"));
        assert_eq!(first, again);
        assert_eq!(again.persona.as_deref(), Some("terse"));
        assert_eq!(registry.len(), 1);

        let other = registry.intern("planner", None);
        assert_eq!(other.persona, None);
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.agents()[0].name, "reviewer");
    }

    /// This crate's stand-in for `selora_ai::TurnCaps` (8c uses the real one).
    #[derive(Debug, Clone, PartialEq)]
    struct TestCaps {
        iterations: u32,
    }

    fn caps(iterations: u32) -> TestCaps {
        TestCaps { iterations }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Seen {
        agent: String,
        persona: Option<String>,
        step_id: Option<String>,
        instructions: String,
        history: Vec<Message>,
        caps: TestCaps,
    }

    #[derive(Default)]
    struct FakeState {
        seen: Vec<Seen>,
        in_flight: usize,
        max_in_flight: usize,
        active_agents: HashSet<String>,
        same_agent_overlap: bool,
    }

    struct FakeExecutor {
        delay: Duration,
        fail_on: Option<String>,
        panic_on: Option<String>,
        state: Mutex<FakeState>,
    }

    impl FakeExecutor {
        fn new(delay: Duration) -> Self {
            Self {
                delay,
                fail_on: None,
                panic_on: None,
                state: Mutex::new(FakeState::default()),
            }
        }

        fn failing(instructions: &str) -> Self {
            Self {
                fail_on: Some(instructions.to_owned()),
                ..Self::new(Duration::ZERO)
            }
        }

        fn panicking(instructions: &str) -> Self {
            Self {
                panic_on: Some(instructions.to_owned()),
                ..Self::new(Duration::ZERO)
            }
        }

        fn seen(&self) -> Vec<Seen> {
            self.state.lock().expect("state lock").seen.clone()
        }

        fn started_order(&self) -> Vec<String> {
            self.seen()
                .into_iter()
                .map(|seen| seen.instructions)
                .collect()
        }

        fn max_in_flight(&self) -> usize {
            self.state.lock().expect("state lock").max_in_flight
        }

        fn overlapped_an_agent(&self) -> bool {
            self.state.lock().expect("state lock").same_agent_overlap
        }
    }

    impl AskExecutor for FakeExecutor {
        type Caps = TestCaps;

        fn ask(&self, request: AskRequest<TestCaps>) -> Result<AskReply, AskError> {
            {
                let mut state = self.state.lock().expect("state lock");
                if !state.active_agents.insert(request.agent.clone()) {
                    state.same_agent_overlap = true;
                }
                state.in_flight += 1;
                state.max_in_flight = state.max_in_flight.max(state.in_flight);
                state.seen.push(Seen {
                    agent: request.agent.clone(),
                    persona: request.persona.clone(),
                    step_id: request.step_id.clone(),
                    instructions: request.instructions.clone(),
                    history: request.history.clone(),
                    caps: request.caps.clone(),
                });
            }
            thread::sleep(self.delay);
            {
                let mut state = self.state.lock().expect("state lock");
                state.active_agents.remove(&request.agent);
                state.in_flight -= 1;
            }
            if self.panic_on.as_deref() == Some(request.instructions.as_str()) {
                panic!("fake executor panic");
            }
            if self.fail_on.as_deref() == Some(request.instructions.as_str()) {
                return Err(AskError::new("gateway said no"));
            }
            Ok(AskReply::with_usage(
                format!("answer from {}", request.agent),
                request.instructions.len() as u64,
                0.01,
            ))
        }
    }

    #[test]
    fn concurrent_asks_on_one_agent_are_fifo() {
        let fake = Arc::new(FakeExecutor::new(Duration::from_millis(60)));
        let pool = Subagents::new(fake.clone(), caps(40), SubagentConfig::new(4));
        let reviewer = pool.agent("reviewer");

        thread::scope(|scope| {
            let mut joins = Vec::new();
            for index in 0..3 {
                let handle = reviewer.clone();
                joins.push(scope.spawn(move || handle.ask(format!("ask-{index}"))));
                // Spawn sequentially so ticket order is submission order; the
                // 60ms fake keeps the later asks queued behind the first.
                thread::sleep(Duration::from_millis(10));
            }
            for join in joins {
                join.join().expect("join").expect("ask");
            }
        });

        assert_eq!(
            fake.started_order(),
            ["ask-0".to_owned(), "ask-1".to_owned(), "ask-2".to_owned()]
        );
        assert!(!fake.overlapped_an_agent(), "a shared agent is serialized");
        // Every ask saw the exchange before it.
        let seen = fake.seen();
        assert_eq!(seen[0].history.len(), 1);
        assert_eq!(seen[1].history.len(), 3);
        assert_eq!(seen[2].history.len(), 5);
    }

    #[test]
    fn separate_agents_run_concurrently_up_to_the_cap() {
        let fake = Arc::new(FakeExecutor::new(Duration::from_millis(40)));
        let pool = Subagents::new(fake.clone(), caps(40), SubagentConfig::new(2));

        thread::scope(|scope| {
            let mut joins = Vec::new();
            for index in 0..5 {
                let handle = pool.agent(format!("agent-{index}"));
                joins.push(scope.spawn(move || handle.ask(format!("item-{index}"))));
            }
            for join in joins {
                join.join().expect("join").expect("ask");
            }
        });

        assert_eq!(fake.seen().len(), 5);
        assert_eq!(
            fake.max_in_flight(),
            2,
            "the cap bounds in-flight asks across agents"
        );
        assert_eq!(pool.config().max_concurrency(), 2);
        assert_eq!(pool.agent_names().len(), 5);
    }

    #[test]
    fn separate_agents_have_separate_conversations() {
        let fake = Arc::new(FakeExecutor::new(Duration::ZERO));
        let pool = Subagents::with_defaults(fake.clone(), caps(40));

        let writer = pool.agent("writer");
        let reviewer = pool.agent("reviewer");
        writer.ask("alpha").expect("writer ask");
        reviewer.ask("beta").expect("reviewer ask");
        writer.ask("gamma").expect("writer ask again");

        let seen = fake.seen();
        assert_eq!(seen[0].agent, "writer");
        assert_eq!(seen[0].history.len(), 1);
        assert_eq!(seen[1].agent, "reviewer");
        // The reviewer never sees the writer's turn, even though it ran
        // between the writer's two asks.
        assert_eq!(seen[1].history.len(), 1);
        assert_eq!(seen[1].history[0].content, "beta");
        // The writer's third ask sees exactly its own two exchanges.
        assert_eq!(seen[2].history.len(), 3);
        assert_eq!(seen[2].history[0].content, "alpha");
        assert_eq!(seen[2].history[2].content, "gamma");

        let writer_history = writer.transcript();
        assert_eq!(writer_history.len(), 4);
        assert_eq!(writer_history[1].content, "answer from writer");
        let reviewer_history = reviewer.transcript();
        assert_eq!(reviewer_history.len(), 2);
        assert_eq!(reviewer_history[0].content, "beta");
        assert_eq!(reviewer_history[1].content, "answer from reviewer");
    }

    #[test]
    fn the_same_name_is_the_same_conversation() {
        let fake = Arc::new(FakeExecutor::new(Duration::ZERO));
        let pool = Subagents::with_defaults(fake.clone(), caps(40));

        pool.agent("reviewer").ask("one").expect("ask");
        pool.agent("reviewer").ask("two").expect("ask");

        let seen = fake.seen();
        assert_eq!(seen[0].history.len(), 1);
        assert_eq!(seen[1].history.len(), 3);
        assert_eq!(pool.agent_names(), ["reviewer".to_owned()]);
    }

    #[test]
    fn personas_are_declared_once_and_reach_the_turn() {
        let fake = Arc::new(FakeExecutor::new(Duration::ZERO));
        let pool = Subagents::with_defaults(fake.clone(), caps(40));

        let handle = pool
            .agent_with_persona("security", "a terse security reviewer")
            .expect("declare persona");
        let same = pool.agent_with_persona("security", "a terse security reviewer");
        assert!(same.is_ok(), "re-declaring the same persona is fine");
        let conflict = pool.agent_with_persona("security", "an optimistic writer");
        assert!(matches!(
            conflict,
            Err(SubagentError::PersonaMismatch { .. })
        ));

        // `agent(name)` reuses the persona-bearing conversation.
        pool.agent("security").ask("audit").expect("ask");
        let seen = fake.seen();
        assert_eq!(seen[0].agent, "security");
        assert_eq!(
            seen[0].persona.as_deref(),
            Some("a terse security reviewer")
        );
        assert_eq!(seen[0].history[0].content, "audit");
        assert_eq!(handle.persona(), Some("a terse security reviewer"));
    }

    #[test]
    fn each_ask_carries_its_own_caps_and_step() {
        let fake = Arc::new(FakeExecutor::new(Duration::ZERO));
        let pool = Subagents::with_defaults(fake.clone(), caps(40));
        let handle = pool.agent("reviewer");

        handle.ask_at("step-1", "default caps").expect("ask");
        handle.ask_with_caps("tight caps", caps(3)).expect("ask");

        let seen = fake.seen();
        assert_eq!(seen[0].caps.iterations, 40);
        assert_eq!(seen[0].step_id.as_deref(), Some("step-1"));
        assert_eq!(seen[1].caps.iterations, 3);
        assert_eq!(seen[1].step_id, None);
        // The scheduler's default caps are untouched by the override.
        assert_eq!(pool.caps().iterations, 40);
    }

    #[test]
    fn a_failed_ask_is_reported_and_rolled_back() {
        let fake = Arc::new(FakeExecutor::failing("doomed"));
        let pool = Subagents::with_defaults(fake.clone(), caps(40));
        let handle = pool.agent("flaky");

        handle.ask("first").expect("first ask");
        let error = handle.ask("doomed").expect_err("second ask fails");
        assert!(matches!(error, SubagentError::Ask(_)));
        assert_eq!(error.to_string(), "subagent ask failed: gateway said no");

        // The failed instructions are not left in the conversation.
        let history = handle.transcript();
        assert_eq!(history.len(), 2, "{history:?}");
        assert_eq!(history[0].content, "first");
        assert_eq!(history[1].content, "answer from flaky");
    }

    #[test]
    fn a_panicking_ask_does_not_wedge_the_agent() {
        let fake = Arc::new(FakeExecutor::panicking("panic"));
        let pool = Subagents::with_defaults(fake.clone(), caps(40));
        let handle = pool.agent("reviewer");

        let failed = thread::scope(|scope| scope.spawn(|| handle.ask("panic").map(|_| ())).join());
        assert!(failed.is_err(), "the panic propagates to the caller");

        // The ticket advanced and the pending instruction was rolled back.
        handle.ask("after").expect("the agent still works");
        let history = handle.transcript();
        assert_eq!(history.len(), 2, "{history:?}");
        assert_eq!(history[0].content, "after");
        assert_eq!(history[1].content, "answer from reviewer");
    }

    #[test]
    fn zero_concurrency_normalizes_to_one() {
        assert_eq!(SubagentConfig::new(0).max_concurrency(), 1);
        assert_eq!(SubagentConfig::default().max_concurrency(), 4);
    }
}
