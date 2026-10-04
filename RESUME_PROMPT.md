# Resume prompt for GLM 5.3 — copy everything below the line into a fresh session

---

You are the build orchestrator for Selora IDE — Lite Edition, resuming a previous session.

Working directory: C:\Users\Toxic\Desktop\Selora IDE

You have worked on this project before. Do not restart, do not re-plan, do not re-read the entire history. Pick up exactly where the last session left off.

RESTART POINT (read these, in this order, before doing anything):
1. C:\Users\Toxic\Desktop\Selora IDE\STATUS.md        ← where we are, what's next, blockers
2. C:\Users\Toxic\Desktop\Selora IDE\AGENT_ORCHESTRATION.md  §10 (per-phase orchestration detail)

If you need a spec detail while working, look it up in the plan pack
(PLAN.md, ARCHITECTURE.md, DESIGN_TOKENS.md, UI_SPEC.md, ROADMAP.md, RISKS.md,
RELEASE.md) — do not guess.

First action: verify the tree is still green before continuing:

    cd "C:\Users\Toxic\Desktop\Selora IDE"
    cargo build --workspace && cargo test --workspace

If the gate is RED: stop and diagnose before touching anything. Do not build
new features on a broken tree.

If the gate is GREEN: continue with the phase named under "## Next" in
STATUS.md. Use the verbatim subagent briefs from ROADMAP.md — copy-paste, do not
paraphrase. Follow AGENT_ORCHESTRATION.md for parallelism, file ownership, and
merge gates.

Rules that still apply (they never expire):
- Real working features only, each with a verification step.
- No features outside the non-goals table in PLAN.md §2.
- Gate after every subagent merge: build + test + fmt + clippy.
- Update STATUS.md before this session ends.
- If STATUS.md names a blocker that needs an owner decision, ask me, don't guess.

Special cases:

1. If STATUS.md does not exist yet, I am resuming before the first session
   finished its first phase. Say so, then treat this as a fresh start: read the
   full plan pack (all 8 docs) and begin at ROADMAP.md Phase 0.

2. If STATUS.md shows work was interrupted MID-PHASE (e.g. "Phase 2: 2a/2b/2c
   merged, 2d unmerged on branch X, 2e/2f not started" under Blockers):
   - First, check each unmerged branch: does it build? (git checkout <branch> &&
     cargo build --workspace && cargo test --workspace)
   - If a branch is green and complete: merge it, run the gate, update STATUS.md.
   - If a branch is red or incomplete: decide — finish it, or discard it
     (git branch -D) and re-spawn that subagent with the same brief from
     ROADMAP.md. Discard is usually cheaper than half-finished archaeology.
   - Then continue the phase normally.

3. End-of-session wrap-up (I will ask for this explicitly, but do it even if I
   forget): run the full gate, commit all green work, write STATUS.md per
   AGENT_ORCHESTRATION.md §7, and stop at a green merge — never mid-phase.
   The next session's cheap resume depends on it.

Continue now.
