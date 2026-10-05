//! Integration test against a real rust-analyzer (ARCHITECTURE.md §13: if
//! `rust-analyzer` is not on PATH, this test is skipped, not failed).
//!
//! Exercises the full process path: spawn, initialize handshake, didOpen,
//! publishDiagnostics for a syntactically broken file, hover over a known
//! item, and a definition lookup.
//!
//! The fixture is a tiny standalone cargo package (an empty `[workspace]`
//! table keeps it out of the parent workspace) placed under the build
//! `target/` dir so rust-analyzer's analysis cache survives between runs —
//! the first run pays the sysroot scan, later runs answer in seconds.

use std::path::PathBuf;
use std::time::Duration;

use selora_lsp::{path_to_uri, ClientEvent, LspClient, ServerSpec, ServerStatus};
use tokio::sync::mpsc;

/// The broken file is `src/lib.rs` (the crate root — a non-member file
/// would never get rustc-side diagnostics), and `src/good.rs` is its module.
fn fixture_workspace() -> PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/ra-probe");
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("mkdir");
    // A missing `)` is a *syntax* error: it squiggles without the crate
    // needing to compile.
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"ra-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(src.join("lib.rs"), "mod good;\npub fn broken( {\n").expect("write lib.rs");
    std::fs::write(
        src.join("good.rs"),
        "pub fn good() -> Vec<u8> {\n    Vec::new()\n}\n",
    )
    .expect("write good.rs");
    root
}

async fn next_event(events: &mut mpsc::UnboundedReceiver<ClientEvent>) -> ClientEvent {
    tokio::time::timeout(Duration::from_secs(120), events.recv())
        .await
        .expect("timed out waiting for a rust-analyzer event")
        .expect("event channel open")
}

async fn await_ready(events: &mut mpsc::UnboundedReceiver<ClientEvent>) {
    match next_event(events).await {
        ClientEvent::Status(ServerStatus::Starting) => {}
        other => panic!("expected Starting, got {other:?}"),
    }
    match next_event(events).await {
        ClientEvent::Status(ServerStatus::Ready) => {}
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn real_rust_analyzer_diagnoses_a_broken_file_and_answers_hover() {
    let Some(rust_analyzer) = selora_lsp::find_on_path("rust-analyzer") else {
        // SKIPPED, not failed (ARCHITECTURE.md §13). Real-server acceptance
        // needs rust-analyzer installed
        // (`rustup component add rust-analyzer`).
        eprintln!("skipping: rust-analyzer is not on PATH");
        return;
    };
    let workspace = fixture_workspace();

    let spec = ServerSpec {
        name: "rust-analyzer".into(),
        program: rust_analyzer,
        args: vec![],
    };
    let (events_tx, mut events) = mpsc::unbounded_channel();
    let client = LspClient::spawn(&spec, Some(&workspace), events_tx).expect("spawn rust-analyzer");
    await_ready(&mut events).await;

    // Open the crate root: rust-analyzer publishes "unclosed delimiter" /
    // "expected R_PAREN" once the workspace has loaded. The first-ever run
    // scans the sysroot, hence the generous window.
    let broken = workspace.join("src").join("lib.rs");
    let broken_text = std::fs::read_to_string(&broken).expect("read back");
    client
        .did_open(&path_to_uri(&broken), "rust", &broken_text, 0)
        .await
        .expect("did_open");

    let saw_diagnostic = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match next_event(&mut events).await {
                ClientEvent::Diagnostics {
                    uri: event_uri,
                    diagnostics,
                } => {
                    if event_uri.ends_with("ra-probe/src/lib.rs")
                        && diagnostics.iter().any(|diagnostic| {
                            diagnostic.message.contains("unclosed")
                                || diagnostic.message.contains("R_PAREN")
                        })
                    {
                        return true;
                    }
                }
                // Indexing / fetching progress is expected on the way (this
                // is exactly the status-bar "rust-analyzer: N/M 42%" line).
                ClientEvent::Progress { .. } | ClientEvent::Status(_) => continue,
            }
        }
    })
    .await
    .is_ok();
    assert!(
        saw_diagnostic,
        "expected an 'unclosed delimiter' diagnostic for the broken crate root"
    );

    // Hover over `Vec` in good.rs (line 0, character 11 — ASCII, so UTF-16
    // equals code points): the typed API returns the type's signature.
    let good = workspace.join("src").join("good.rs");
    let good_text = std::fs::read_to_string(&good).expect("read back");
    client
        .did_open(&path_to_uri(&good), "rust", &good_text, 0)
        .await
        .expect("did_open");
    // Let rust-analyzer notice the file before asking.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let hover = tokio::time::timeout(
        Duration::from_secs(60),
        client.hover(&path_to_uri(&good), lsp_types::Position::new(0, 11)),
    )
    .await
    .expect("hover answered within 60s")
    .expect("hover ok")
    .expect("hover has contents");
    let contents = format!("{:?}", hover.contents);
    assert!(
        contents.contains("Vec") || contents.contains("alloc"),
        "hover mentions the type: {contents}"
    );

    // And a definition lookup: `new` (line 1, character 10) resolves to a
    // location in the sysroot sources.
    let definitions = tokio::time::timeout(
        Duration::from_secs(60),
        client.definition(&path_to_uri(&good), lsp_types::Position::new(1, 10)),
    )
    .await
    .expect("definition answered within 60s")
    .expect("definition ok");
    assert!(
        !definitions.is_empty(),
        "definition of Vec::new resolved to a location: {definitions:?}"
    );

    client.shutdown();
}
