# Session, todo, and state slice (Phase 9 groundwork)

Status: implemented and tested. This slice makes the agent dock's session,
todo, active-buffer, and event-delivery state backend-authoritative.

## Commands

| Command | Behavior |
|---|---|
| `new_session` | Creates a real empty session in `SessionEngine`, selects it, emits `bootstrap_state`. Refused with `session_busy` while any turn is live. |
| `switch_session { session_id }` | Validates the id (`unknown_session` otherwise), selects it, emits `bootstrap_state`. Refused with `session_busy` while any turn is live. |
| `retry_message { session_id, message_id }` | Validates the session/message (`unknown_session`, `unknown_message`), requires a user message (`not_a_user_message`) with a recorded dispatch path (`retry_unavailable`), rejects a live turn (`session_busy`), truncates that message and its tail, and re-dispatches the same content once through the original path (chat or agent, with the original model/mode/reasoning/temperature). Emits `bootstrap_state` so the frontend learns the new `active_request_id`. |
| `set_active_buffer { buffer_id }` | Records the user's actual tab. Rejects an unknown/closed buffer (`unknown_buffer`). Bootstrap prefers it over the first buffer in path order. |

Sessions own independent transcripts, message/request ids, parked approval
state, and todo lists. `chat_state.messages` stays flattened for protocol
compatibility; the frontend filters by `active_session_id`.

## Todos

`manage_todo` now writes a validated list into the owning session:

- `todos` is a JSON array (a stringified JSON array is also accepted) of
  `{ id, text, status }`; at most 50 items, id ≤ 64 chars and unique, text
  ≤ 500 chars, status one of `pending`, `in_progress`, `completed`.
- Malformed input returns a tool **error** and stores nothing.
- The success result reports the list read back from the store, not the
  model's raw arguments.
- A context without a session store (workflow subagents) returns a typed
  error rather than pretending to store.

The list is announced live as `session_todos_changed { session_id, todos }`,
carried per session in `SessionDto.todos`, and for the active session in
`ChatStateDto.todos` (both serde-defaulted).

There is no `todo_read` tool in this slice and no local todo editing: the plan
spine renders the stored list read-only. Add/remove/reopen/template actions
were removed because they mutated a local copy the backend never saw.

## Removed fabricated surfaces

- The seeded `INITIAL_SUBAGENTS` roster and the "Subagents Swarm" card (no
  subagent runtime exists; nothing invents agents).
- The local "Artifacts" drawer in the plan spine (workflow artifacts already
  have a real panel).
- Local session create/switch/delete/fork. Rail buttons call the commands
  above; a failed command leaves the backend's active session unchanged and
  shows the protocol message in a toast.
- The fabricated `main` branch, "Commit or push" row, add-step input, and
  starter-plan templates in the plan spine.

## Reply strategy (additive)

`dispatch_command` returns `CommandResultDto`:

- `Snapshot(Box<BootstrapState>)` for `get_bootstrap_state` and the folder-open
  commands — exactly the pre-existing JSON shape.
- `Ack(CommandAckDto)` for event-backed mutations: `protocol_version`,
  `core_instance_id`, `snapshot_sequence`, `acknowledged: true`, and the
  active `session_id`.

The union is untagged and `protocol_version` stays 1. Existing clients that
only read the event stream are unaffected; clients that parse the bootstrap
from commands that always returned one still get it. The small ack avoids a
tree walk, libgit2 status, and a full transcript copy on every keystroke-level
mutation.

## Event ordering

`EventLog` assigns the sequence and emits under one order guard, so two
concurrent producers cannot deliver n+1 before n. A no-emit `push` takes the
same guard, so it cannot interleave a sequence into the emit stream. Covered
by a multi-thread regression test (8 producers × 25 events).

## Known limitations

- Sessions and todos are in-memory for this slice; durable session JSONL
  persistence, resume, compaction, and a `todo_read` tool remain Phase 9d/9b.
- `retry_message` does not retain the original editor context (the agent
  system prompt's active-file note); it re-sends the user text and the
  recorded model/mode/instruction fields.
- `new_session`/`switch_session` are refused (not queued) during a live turn.
- The plan spine is read-only; there is no UI affordance to edit todos because
  the backend has no command for a user-authored todo write.