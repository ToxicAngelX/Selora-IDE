# Agent Mode

Agent Mode turns the chat into an agent turn: the model can read, create,
edit, and delete workspace files, run shell commands, and search/fetch the
web — with **every side-effecting action gated behind an approval card**.

Toggle it with the `agent` pill in the chat header. The state persists per
workspace.

## The tools

| Tool | Approval |
|---|---|
| `read_file`, `list_dir`, `fuzzy_find`, `search_files`, `web_search`, `web_fetch` | never — read-only |
| `write_file`, `edit_file`, `delete_file` | always — diff/delete card |
| `bash` | card, unless the command matches your allowlist |

Reads stream their results into collapsed cards. Writes show a unified
diff with +/- coloring; nothing touches disk until you **Approve**.
**Approve all (this turn)** pre-approves the remaining write/bash cards of
the current turn only — it is never persisted. **Deny** tells the model
"the user denied this" and the turn continues without the change. **Stop**
ends the turn within a second (a running command's child process is
killed).

## Guards (always on)

- Paths are rooted at the workspace; escapes are rejected.
- `web_fetch` passes an SSRF guard (no localhost, private, or link-local
  addresses), caps pages at 256 KB, and follows no redirects.
- `bash` runs with a cleared environment (a fixed allowlist of variables —
  your Selora key is structurally absent), no profile scripts, and a
  timeout. Only commands matching the `agent.bash_allowlist` setting run
  without a card.
- The loop caps at 25 tool iterations, 10 minutes, and $1.00 of estimated
  cost per turn (all three configurable under `agent.*` in Settings); a
  turn that hits a cap ends with its partial answer.

## Turn summary

Every turn ends with a footer: iterations, tools used, approved/denied
counts, tokens, and cost. The cost meter in the status bar includes agent
turns.
