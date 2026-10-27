use selora_protocol::{EventEnvelope, FrontendEvent};

/// Replay helper retained for the event-stream slice; the empty shell has no producer yet.
#[allow(dead_code)]
pub fn replay_after(
    events: &[EventEnvelope<FrontendEvent>],
    sequence: u64,
) -> Vec<EventEnvelope<FrontendEvent>> {
    events
        .iter()
        .filter(|event| event.sequence > sequence)
        .cloned()
        .collect()
}
