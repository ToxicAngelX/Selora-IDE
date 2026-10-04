//! Manual smoke test against the real Selora gateway (ROADMAP.md Phase 1
//! acceptance). NOT run in CI — it needs a real key.
//!
//! Usage:
//!   SELORA_KEY=sk-gw-... cargo run -p selora-ai --example smoke [base_url]
//!
//! Exercises exactly what the spike window does: `list_models`, then a
//! streaming "say hi" chat, printing deltas as they arrive and the receipt
//! at the end.

use selora_ai::{ChatMessage, ChatRequest, SeloraClient};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let key = std::env::var("SELORA_KEY").expect("SELORA_KEY must be set");
    let base_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://api.selora.lol".to_owned());

    let client = SeloraClient::new(&base_url, &key);

    // --- list_models ---
    println!("base_url: {base_url}");
    let models = match client.list_models().await {
        Ok(models) => {
            println!("connected ({} models):", models.len());
            for model in &models {
                println!("  - {} ({})", model.id, model.display_name);
            }
            models
        }
        Err(error) => {
            println!("connect FAILED: {error}");
            std::process::exit(1);
        }
    };

    // --- stream_chat: "say hi" ---
    // The spike takes the first model the gateway listed; SELORA_MODEL overrides.
    let model_id = std::env::var("SELORA_MODEL")
        .ok()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| {
            models
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_else(|| "default".to_owned())
        });

    let request =
        ChatRequest::new(model_id.clone(), vec![ChatMessage::user("say hi")]).with_stream(true);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let stream = tokio::spawn(async move { client.stream_chat(request, tx).await });

    let mut streamed = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            selora_ai::ChatEvent::Delta(delta) => {
                streamed.push_str(&delta);
                print!("{delta}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            selora_ai::ChatEvent::Done(receipt) => {
                println!();
                println!(
                    "--- receipt: model={}, tokens: {} in / {} out / cost: ${:.4} (estimated: {})",
                    receipt.model,
                    receipt.prompt_tokens,
                    receipt.completion_tokens,
                    receipt.cost_usd,
                    receipt.estimated
                );
            }
            selora_ai::ChatEvent::Error(error) => {
                println!();
                println!("--- stream error: {error}");
            }
        }
    }

    match stream.await {
        Ok(Ok(_)) => {
            println!(
                "--- first 200 chars: {}",
                &streamed.chars().take(200).collect::<String>()
            );
            println!("SMOKE TEST PASSED");
        }
        Ok(Err(error)) => {
            println!("stream FAILED: {error}");
            std::process::exit(1);
        }
        Err(join) => {
            println!("worker died: {join}");
            std::process::exit(1);
        }
    }
}
