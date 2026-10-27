//! Phase 9 acceptance fixtures (ROADMAP.md Phase 9, Agent Workbench).
//!
//! These fixtures are deliberately headless and deterministic: no gateway, no
//! window, no clock, no key. Every side effect happens in a throwaway temp
//! workspace and every chat round is scripted through the public
//! [`ChatStream`] seam. They cover only the Phase 9 substrate that `integration`
//! actually exposes today:
//!
//! * step 1 substrate — a write is *planned* as a diff card, nothing touches
//!   disk, and only `execute_approved` applies it;
//! * step 2 substrate — `manage_todo` is a non-side-effecting tool result (the
//!   authoritative session todo list is a 9b/9d slice);
//! * step 6 substrate — the context budget is the documented 80% of the model
//!   window (the summarize round itself is a 9d slice);
//! * step 7 — `auto_approve_writes` skips file cards for one turn only; a fresh
//!   turn (the engine-restart analogue) defaults back to cards.
//!
//! The harness in `scripts/phase9-acceptance.ps1` reports the remaining steps
//! as PENDING until the runtime branches land. Keeping these tests on the
//! public API is what lets that harness run unchanged after those merges.

use std::collections::VecDeque;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;

use selora_ai::{
    execute_approved, execute_tool, run_agent_turn, AgentDecision, AgentEvent, AgentToolSettings,
    ApprovalCard, ChatEvent, ChatMessage, ChatReceipt, ChatRequest, ChatStream, Model,
    RouterToolRunner, ToolCall, ToolContext, ToolOutcome, TurnDecision, TurnEnd, TurnInput,
};

const TEST_MODEL: &str = "phase9-acceptance-model";
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Temp workspace and model fixtures
// ---------------------------------------------------------------------------

/// A unique temp workspace per test (tests run in parallel and can share a
/// process id, so the label is part of the path).
fn temp_workspace(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "selora-phase9-acceptance-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create the temp workspace");
    root
}

fn model() -> Model {
    Model {
        id: TEST_MODEL.into(),
        display_name: "Phase 9 Acceptance".into(),
        context_window: 200_000,
        price_per_1k_input: 0.01,
        price_per_1k_output: 0.03,
        supports_streaming: true,
        supports_tools: true,
        family: "test".into(),
    }
}

fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.to_string(),
    }
}

fn read_to_string(path: &Path) -> String {
    fs::read_to_string(path).expect("workspace file exists")
}

// ---------------------------------------------------------------------------
// Step 1 substrate — plan -> diff card -> approve -> write
// ---------------------------------------------------------------------------

#[tokio::test]
async fn step1_write_is_planned_as_a_diff_and_applied_only_after_approval() {
    let root = temp_workspace("step1-write");
    let target = root.join("NOTES.md");
    fs::write(&target, "old line\n").expect("seed the target file");
    let context = ToolContext::new(&root, AgentToolSettings::default());

    let call = tool_call(
        "write-1",
        "write_file",
        json!({ "path": "NOTES.md", "content": "old line\nnew line\n" }),
    );

    // Planning produces the card the user approves; the file is untouched.
    let card = match execute_tool(&context, call.clone()).await {
        ToolOutcome::NeedsApproval(card) => card,
        other => panic!("write_file must plan a diff card, got {other:?}"),
    };
    let ApprovalCard::Diff { path, diff } = &card else {
        panic!("write_file must produce a Diff card, got {card:?}");
    };
    assert_eq!(path, "NOTES.md");
    assert!(
        diff.contains("+new line"),
        "the card shows the addition: {diff}"
    );
    assert_eq!(
        read_to_string(&target),
        "old line\n",
        "nothing was written yet"
    );

    // Approval is the apply moment, and it writes exactly the approved bytes.
    let result = execute_approved(&context, call).await;
    assert!(result.ok, "approved write failed: {}", result.content);
    assert_eq!(read_to_string(&target), "old line\nnew line\n");
    assert!(
        root.join(".NOTES.md.selora-bak").exists(),
        "the overwrite keeps a recovery sidecar"
    );

    let _ = fs::remove_dir_all(&root);
}

#[tokio::test]
async fn step1_edit_diff_revalidates_against_the_current_file() {
    let root = temp_workspace("step1-edit");
    let target = root.join("NOTES.md");
    fs::write(&target, "alpha\nbeta\n").expect("seed the target file");
    let context = ToolContext::new(&root, AgentToolSettings::default());

    let call = tool_call(
        "edit-1",
        "edit_file",
        json!({ "path": "NOTES.md", "old_str": "beta", "new_str": "gamma" }),
    );
    match execute_tool(&context, call.clone()).await {
        ToolOutcome::NeedsApproval(ApprovalCard::Diff { diff, .. }) => {
            assert!(diff.contains("-beta"));
            assert!(diff.contains("+gamma"));
        }
        other => panic!("edit_file must plan a diff card, got {other:?}"),
    }
    assert_eq!(read_to_string(&target), "alpha\nbeta\n");

    // A different writer changes the file between the card and the approval:
    // the approve path re-reads and refuses the now-stale exact match.
    fs::write(&target, "alpha\nBETA changed\n").expect("simulate a concurrent edit");
    let result = execute_approved(&context, call).await;
    assert!(!result.ok, "a stale exact-match approval must be refused");
    assert_eq!(
        read_to_string(&target),
        "alpha\nBETA changed\n",
        "the approval never clobbers the newer content"
    );

    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Step 2 / 6 substrate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn step2_manage_todo_is_ready_and_never_builds_an_approval_card() {
    let root = temp_workspace("step2-todo");
    let context = ToolContext::new(&root, AgentToolSettings::default());
    let call = tool_call(
        "todo-1",
        "manage_todo",
        json!({ "todos": [{ "title": "inspect", "status": "in_progress" }] }),
    );

    match execute_tool(&context, call).await {
        ToolOutcome::Ready(result) => {
            assert!(result.ok);
            assert!(
                result.content.contains("inspect"),
                "the tool acknowledges the supplied list: {}",
                result.content
            );
        }
        other => panic!("manage_todo is read-only and must not card, got {other:?}"),
    }
    assert!(
        root.read_dir()
            .expect("workspace readable")
            .next()
            .is_none(),
        "no file was written"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn step6_context_budget_is_eighty_percent_of_the_window() {
    let caps = selora_ai::TurnCaps::spec_defaults(1_000);
    assert_eq!(
        caps.context_budget_tokens(),
        Some(800),
        "the context cap is window minus the 20% reserve"
    );
    assert_eq!(
        selora_ai::TurnCaps::spec_defaults(0).context_budget_tokens(),
        None
    );
}

// ---------------------------------------------------------------------------
// Step 7 — auto-approve is turn-scoped, never carried across a restart
// ---------------------------------------------------------------------------

struct ScriptedChat {
    rounds: VecDeque<Vec<ChatEvent>>,
}

impl ScriptedChat {
    fn one_write_then_answer(path: &str, content: &str) -> Self {
        let args = json!({ "path": path, "content": content }).to_string();
        Self {
            rounds: VecDeque::from(vec![
                vec![
                    ChatEvent::ToolCallDelta {
                        index: 0,
                        id: Some("write-auto".into()),
                        name: Some("write_file".into()),
                        arguments: args,
                    },
                    ChatEvent::Done(receipt(0.01)),
                ],
                vec![
                    ChatEvent::Delta("written".into()),
                    ChatEvent::Done(receipt(0.01)),
                ],
            ]),
        }
    }
}

impl ChatStream for ScriptedChat {
    fn stream_round(
        &mut self,
        _request: ChatRequest,
        events: mpsc::UnboundedSender<ChatEvent>,
    ) -> Pin<Box<dyn Future<Output = Result<ChatReceipt, selora_core::SeloraError>> + Send + '_>>
    {
        let round = self
            .rounds
            .pop_front()
            .expect("the script has one round per request");
        Box::pin(async move {
            for event in round {
                let _ = events.send(event);
            }
            Ok(receipt(0.01))
        })
    }
}

fn receipt(cost_usd: f64) -> ChatReceipt {
    ChatReceipt {
        request_id: uuid::Uuid::new_v4(),
        model: TEST_MODEL.into(),
        prompt_tokens: 10,
        completion_tokens: 5,
        cost_usd,
        duration_ms: 1,
        estimated: false,
    }
}

/// Run one scripted turn. `deny` answers the first card with Deny (so the
/// test ends deterministically); `false` expects no card at all.
async fn run_scripted(
    root: &Path,
    label: &str,
    path: &str,
    content: &str,
    auto_approve: bool,
) -> Vec<AgentEvent> {
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (decision_tx, decision_rx) = mpsc::unbounded_channel::<TurnDecision>();
    let input = TurnInput::new(model(), vec![ChatMessage::user(label)])
        .with_auto_approve_writes(auto_approve);
    let tools = RouterToolRunner::new(ToolContext::new(root, AgentToolSettings::default()));

    let task = tokio::spawn(run_agent_turn(
        input,
        ScriptedChat::one_write_then_answer(path, content),
        tools,
        events_tx,
        decision_rx,
    ));

    let mut seen = Vec::new();
    let mut denied = false;
    let report = tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let Some(event) = events_rx.recv().await else {
                panic!("the event channel closed before TurnDone");
            };
            if let AgentEvent::ApprovalNeeded { approval_id, .. } = &event {
                assert!(!auto_approve, "auto-approve must never park a file card");
                if !denied {
                    decision_tx
                        .send((*approval_id, AgentDecision::Deny))
                        .expect("send the Deny");
                    denied = true;
                }
            }
            seen.push(event);
            if let Some(AgentEvent::TurnDone(report)) = seen.last() {
                break report.clone();
            }
        }
    })
    .await
    .expect("the turn never ended");
    let joined = task.await.expect("the turn task joined");
    assert_eq!(joined.end, report.end);
    assert!(
        !matches!(report.end, TurnEnd::Failed(_)),
        "the scripted turn failed: {:?}",
        report.end
    );
    seen
}

#[tokio::test]
async fn step7_auto_approve_skips_cards_for_one_turn_and_resets_on_restart() {
    let root = temp_workspace("step7-restart");
    let target = root.join("AUTO.md");

    // A session with full access: the file write applies with no card.
    let auto_events = run_scripted(&root, "auto turn", "AUTO.md", "auto\n", true).await;
    assert!(
        !auto_events
            .iter()
            .any(|event| matches!(event, AgentEvent::ApprovalNeeded { .. })),
        "auto-approve emitted a card: {auto_events:?}"
    );
    assert_eq!(
        read_to_string(&target),
        "auto\n",
        "auto mode wrote the file"
    );

    // "Restart": a brand-new TurnInput (the shell builds one per send) has the
    // default off, so the same write parks on a card again. The flag lived on
    // the first turn only — nothing in the engine carried it forward.
    let fresh = TurnInput::new(model(), vec![ChatMessage::user("after restart")]);
    assert!(
        !fresh.auto_approve_writes,
        "a fresh turn defaults to approval cards"
    );

    let restarted_events = run_scripted(&root, "after restart", "AUTO.md", "second\n", false).await;
    assert!(
        restarted_events
            .iter()
            .any(|event| matches!(event, AgentEvent::ApprovalNeeded { .. })),
        "the restarted turn must park on a card: {restarted_events:?}"
    );
    assert_eq!(
        read_to_string(&target),
        "auto\n",
        "Deny on the restarted turn leaves the file untouched"
    );

    let _ = fs::remove_dir_all(&root);
}
