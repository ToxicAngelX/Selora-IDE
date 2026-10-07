//! Real-gateway Agent Mode smoke — the ROADMAP.md Phase 5 acceptance list's
//! substance (read → deny → approve → bash → web → Stop → summary), driven
//! through the production loop (`spawn_agent_turn` + `ClientChatStream` +
//! `RouterToolRunner`) against the live gateway with a real key.
//!
//! Ignored by default: needs `SELORA_API_KEY` in the environment and network
//! access to the gateway + DuckDuckGo. Run:
//!
//! ```sh
//! cargo test -p selora-ai --test agent_real_gateway -- --ignored --nocapture
//! ```
//!
//! The card painting half of the acceptance list is covered by selora-ui's
//! headless paint tests and selora-app's wiremock end-to-end test; this file
//! exists to prove the real gateway, a real model's tool_calls, the decision
//! channel, and the §17.2 guards against a real workspace on disk.
//!
//! Money note: every turn here costs real (small) USD on the key. The caps
//! below bound each turn at $0.30 / 10 tool iterations / 240s.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use selora_ai::{
    spawn_agent_turn, AgentDecision, AgentEvent, AgentToolSettings, AgentTurnReport, ApprovalCard,
    ChatMessage, ClientChatStream, Model, RouterToolRunner, SeloraClient, ToolContext, TurnCaps,
    TurnDecision, TurnEnd, TurnInput, DENIED_MESSAGE,
};

const DEFAULT_BASE_URL: &str = "https://api.selora.lol";

/// The scratch workspace the agent reads/edits/checks. Distinctive markers so
/// a model answer can be checked against real file content.
const MAIN_RS: &str = "const SMOKE_MARKER: &str = \"amber-compass-7\";\n\nfn main() {\n    println!(\"selora agent smoke: {SMOKE_MARKER}\");\n}\n";

fn scratch_workspace() -> PathBuf {
    let root = std::env::temp_dir().join("selora-agent-smoke");
    let _ = std::fs::remove_dir_all(&root);
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir scratch src");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"agent-smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write scratch Cargo.toml");
    std::fs::write(src.join("main.rs"), MAIN_RS).expect("write scratch main.rs");
    root
}

/// The cheapest model the gateway offers (haiku-class first). The catalog
/// does not advertise capability flags (`supports_tools` deserializes to its
/// false default), so capability is proven by the turns themselves.
async fn pick_model(client: &SeloraClient) -> Model {
    let models = client.list_models().await.expect("list_models");
    assert!(!models.is_empty(), "the gateway returned no models");
    models
        .iter()
        .find(|m| m.id.contains("haiku"))
        .or_else(|| models.first())
        .cloned()
        .expect("a model from the catalog")
}

fn caps(model: &Model) -> TurnCaps {
    TurnCaps {
        max_tool_iterations: 10,
        max_turn_secs: 240,
        max_cost_usd: 0.30,
        ..TurnCaps::spec_defaults(model.context_window)
    }
}

struct Driven {
    report: AgentTurnReport,
    events: Vec<AgentEvent>,
    elapsed: Duration,
}

/// Run one turn, letting the scenario react to events (send decisions, stop)
/// as they arrive, exactly like the app's wiring does.
async fn drive(
    base_url: &str,
    key: &str,
    model: &Model,
    root: &Path,
    conversation: Vec<ChatMessage>,
    on_event: &mut dyn FnMut(&AgentEvent, &mpsc::UnboundedSender<TurnDecision>),
) -> Driven {
    let chat = ClientChatStream::new(SeloraClient::new(base_url, key));
    let tools = RouterToolRunner::new(ToolContext::new(root, AgentToolSettings::default()));
    let input = TurnInput::new(model.clone(), conversation).with_caps(caps(model));
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (decision_tx, join) = spawn_agent_turn(
        &tokio::runtime::Handle::current(),
        input,
        chat,
        tools,
        events_tx,
    );
    let start = Instant::now();
    let mut events = Vec::new();
    let drain = async {
        while let Some(event) = events_rx.recv().await {
            let done = matches!(event, AgentEvent::TurnDone(_));
            on_event(&event, &decision_tx);
            events.push(event);
            if done {
                break;
            }
        }
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(300), drain)
            .await
            .is_ok(),
        "turn did not end within 300s"
    );
    let report = join.await.expect("turn task joined");
    if let TurnEnd::Failed(message) = &report.end {
        panic!("the turn failed (check the key/model first): {message}");
    }
    Driven {
        report,
        events,
        elapsed: start.elapsed(),
    }
}

fn started_tools(events: &[AgentEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolCallStarted(call) => Some(call.name.clone()),
            _ => None,
        })
        .collect()
}

fn results(events: &[AgentEvent]) -> Vec<&selora_ai::ToolResult> {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real key + network: run explicitly"]
async fn real_gateway_agent_smoke() {
    let Ok(key) = std::env::var("SELORA_API_KEY") else {
        eprintln!("skipping: SELORA_API_KEY not set");
        return;
    };
    let base_url = std::env::var("SELORA_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into());
    let root = scratch_workspace();
    let client = SeloraClient::new(&base_url, &key);
    let model = pick_model(&client).await;
    eprintln!("model: {} (context {})", model.id, model.context_window);

    let mut conversation: Vec<ChatMessage> = Vec::new();
    let mut total_tokens = 0u64;
    let mut total_cost = 0.0f64;
    let mut total_tools = 0u32;
    let mut total_approved = 0u32;
    let mut total_denied = 0u32;

    // -- Steps 1-2: read_file card completes; answer cites real content ------
    conversation.push(ChatMessage::user(
        "Use the read_file tool to read the file src/main.rs in this workspace, \
         then tell me the value of the SMOKE_MARKER constant.",
    ));
    let read_turn = drive(
        &base_url,
        &key,
        &model,
        &root,
        conversation.clone(),
        &mut |_, _| {},
    )
    .await;
    assert_eq!(
        started_tools(&read_turn.events),
        vec!["read_file".to_string()],
        "the read turn should call exactly read_file: {:?}",
        started_tools(&read_turn.events)
    );
    let read_results = results(&read_turn.events);
    assert!(read_results.iter().all(|r| r.ok), "read_file must succeed");
    assert!(
        read_turn.report.final_message.contains("amber-compass-7"),
        "the answer must cite the real file content, got: {}",
        read_turn.report.final_message
    );
    assert!(
        read_turn.report.end.is_completed(),
        "read turn: {:?}",
        read_turn.report.end
    );
    eprintln!(
        "steps 1-2 ok: read_file answered with real content ({:.1}s)",
        read_turn.elapsed.as_secs_f32()
    );
    conversation = read_turn.report.conversation.clone();
    total_tokens += read_turn.report.summary.tokens;
    total_cost += read_turn.report.summary.cost_usd;
    total_tools += read_turn.report.summary.tools_used;

    // -- Steps 3-4: a diff card appears; Deny continues, disk untouched ------
    let edit_ask = "Use the edit_file tool to insert a new first line `// entry point` into \
                    src/main.rs. Make exactly this one edit and nothing else.";
    conversation.push(ChatMessage::user(edit_ask));
    let mut deny_sent = false;
    let deny_turn = drive(
        &base_url,
        &key,
        &model,
        &root,
        conversation.clone(),
        &mut |event, decisions| {
            if let AgentEvent::ApprovalNeeded { approval_id, card } = event {
                assert!(
                    matches!(card, ApprovalCard::Diff { path, .. } if path.ends_with("main.rs")),
                    "the edit must surface a diff card for src/main.rs"
                );
                if !deny_sent {
                    deny_sent = true;
                    decisions
                        .send((*approval_id, AgentDecision::Deny))
                        .expect("decision channel open");
                }
            }
        },
    )
    .await;
    assert!(deny_sent, "the edit turn must pause on a diff card");
    let denied = results(&deny_turn.events);
    assert!(
        denied.iter().any(|r| !r.ok && r.content == DENIED_MESSAGE),
        "the denied tool result feeds `user denied this action` back to the model"
    );
    let on_disk = std::fs::read_to_string(root.join("src/main.rs")).expect("read scratch main.rs");
    assert_eq!(on_disk, MAIN_RS, "a denied edit never touches the disk");
    eprintln!(
        "steps 3-4 ok: diff card denied, loop continued, disk unchanged ({:.1}s)",
        deny_turn.elapsed.as_secs_f32()
    );
    conversation = deny_turn.report.conversation.clone();
    total_tokens += deny_turn.report.summary.tokens;
    total_cost += deny_turn.report.summary.cost_usd;
    total_tools += deny_turn.report.summary.tools_used;
    total_denied += deny_turn.report.summary.denied;

    // -- Step 5: ask again; Approve writes the file ---------------------------
    conversation.push(ChatMessage::user(format!(
        "Please try again now that I will approve it: {edit_ask}"
    )));
    let mut approve_sent = false;
    let approve_turn = drive(
        &base_url,
        &key,
        &model,
        &root,
        conversation.clone(),
        &mut |event, decisions| {
            if let AgentEvent::ApprovalNeeded { approval_id, card } = event {
                assert!(
                    matches!(card, ApprovalCard::Diff { .. }),
                    "edit again → diff card"
                );
                if !approve_sent {
                    approve_sent = true;
                    decisions
                        .send((*approval_id, AgentDecision::Approve))
                        .expect("decision channel open");
                }
            }
        },
    )
    .await;
    assert!(approve_sent, "the retry must pause on a diff card again");
    let approved = results(&approve_turn.events);
    assert!(
        approved.iter().any(|r| r.ok),
        "the approved edit executes: {approved:?}"
    );
    let on_disk = std::fs::read_to_string(root.join("src/main.rs")).expect("read scratch main.rs");
    assert!(
        on_disk.starts_with("// entry point\n"),
        "the approved edit is on disk: {on_disk}"
    );
    eprintln!(
        "step 5 ok: approved edit landed on disk ({:.1}s)",
        approve_turn.elapsed.as_secs_f32()
    );
    conversation = approve_turn.report.conversation.clone();
    total_tokens += approve_turn.report.summary.tokens;
    total_cost += approve_turn.report.summary.cost_usd;
    total_tools += approve_turn.report.summary.tools_used;
    total_approved += approve_turn.report.summary.approved;

    // -- Steps 6-7: cargo check is allowlisted — auto-runs, no command card ---
    conversation.push(ChatMessage::user(
        "Use the bash tool to run `cargo check` in this workspace and tell me whether it passed.",
    ));
    let mut command_card_seen = false;
    let bash_turn = drive(
        &base_url,
        &key,
        &model,
        &root,
        conversation.clone(),
        &mut |event, _| {
            if let AgentEvent::ApprovalNeeded { card, .. } = event {
                if matches!(card, ApprovalCard::Command { .. }) {
                    command_card_seen = true;
                }
            }
        },
    )
    .await;
    assert!(
        !command_card_seen,
        "an allowlisted command never shows a command card (§17.5)"
    );
    let bash_results = results(&bash_turn.events);
    assert!(
        bash_results
            .iter()
            .any(|r| r.ok && r.content.contains("(exit code 0)")),
        "cargo check ran and exited 0: {bash_results:?}"
    );
    eprintln!(
        "steps 6-7 ok: allowlisted cargo check auto-ran with exit 0 ({:.1}s)",
        bash_turn.elapsed.as_secs_f32()
    );
    conversation = bash_turn.report.conversation.clone();
    total_tokens += bash_turn.report.summary.tokens;
    total_cost += bash_turn.report.summary.cost_usd;
    total_tools += bash_turn.report.summary.tools_used;

    // -- Steps 8-9: web_search returns real results ---------------------------
    conversation.push(ChatMessage::user(
        "Use the web_search tool to search for \"ropey crate\", then tell me in one \
         sentence what ropey is, based on the results.",
    ));
    let web_turn = drive(
        &base_url,
        &key,
        &model,
        &root,
        conversation.clone(),
        &mut |_, _| {},
    )
    .await;
    let web_results = results(&web_turn.events);
    assert!(
        web_results.iter().any(|r| r.ok && !r.content.is_empty()),
        "web_search returned real results: {web_results:?}"
    );
    assert!(
        web_turn
            .report
            .final_message
            .to_lowercase()
            .contains("rope"),
        "the answer is based on the fetched results: {}",
        web_turn.report.final_message
    );
    eprintln!(
        "steps 8-9 ok: web_search answered from real results ({:.1}s)",
        web_turn.elapsed.as_secs_f32()
    );
    conversation = web_turn.report.conversation.clone();
    total_tokens += web_turn.report.summary.tokens;
    total_cost += web_turn.report.summary.cost_usd;
    total_tools += web_turn.report.summary.tools_used;

    // -- Step 10: Stop ends a turn promptly -----------------------------------
    conversation.push(ChatMessage::user(
        "Do these steps in order: read src/main.rs with read_file, then run `cargo test` \
         with bash, then read Cargo.toml with read_file, then summarize everything.",
    ));
    let mut stops = 0u32;
    let stop_turn = drive(
        &base_url,
        &key,
        &model,
        &root,
        conversation.clone(),
        &mut |event, decisions| {
            // Stop once the turn is clearly mid-work (second tool start) or at
            // the first approval — whichever comes first.
            let should_stop = matches!(event, AgentEvent::ToolCallStarted(_))
                || matches!(event, AgentEvent::ApprovalNeeded { .. });
            if should_stop {
                stops += 1;
                if stops >= 2 {
                    decisions
                        .send((uuid::Uuid::nil(), AgentDecision::Stop))
                        .expect("decision channel open");
                }
            }
        },
    )
    .await;
    assert_eq!(
        stop_turn.report.end,
        TurnEnd::StoppedByUser,
        "Stop ends the turn: {:?}",
        stop_turn.report.end
    );
    assert!(
        stop_turn.elapsed < Duration::from_secs(60),
        "the turn ended promptly after Stop: {:?}",
        stop_turn.elapsed
    );
    assert!(
        !stop_turn.report.summary.completed,
        "a stopped turn is not completed"
    );
    eprintln!(
        "step 10 ok: Stop ended the turn in {:.1}s",
        stop_turn.elapsed.as_secs_f32()
    );
    total_tokens += stop_turn.report.summary.tokens;
    total_cost += stop_turn.report.summary.cost_usd;
    total_tools += stop_turn.report.summary.tools_used;

    // -- Step 11: the turn summary totals are real ----------------------------
    assert!(total_tokens > 0, "receipts carried real token counts");
    assert!(total_cost > 0.0, "receipts carried a real cost");
    assert!(
        total_tools >= 5,
        "tools used across the smoke: {total_tools}"
    );
    assert_eq!(total_denied, 1, "exactly one denial this run");
    assert_eq!(total_approved, 1, "exactly one approval this run");
    eprintln!(
        "step 11 ok: {total_tools} tools · {total_approved} approved · {total_denied} denied · \
         {total_tokens} tokens · ${total_cost:.3}"
    );
    eprintln!("REAL-GATEWAY AGENT SMOKE: PASS");
}
