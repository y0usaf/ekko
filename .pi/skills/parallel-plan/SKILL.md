---
name: parallel-plan
description: Implement one coordinator-issued, dependency-ready, path-owned PLAN.md slice from an exact ekko integration base in an isolated worktree.
---

# Parallel PLAN worker

You are one worker in a coordinated serial gate or explicit PLAN wave. Optimize
for safe replay into integration, not worker count.

The invoking message must provide:

- `item=<PLAN.md item>`
- `assignment=<one coherent deliverable>`
- `base=<exact integration commit>`
- `integration=<integration branch/ref>`
- `dependencies=<required merged commits, or none>`
- `paths=<exclusive paths/globs; shared-file exceptions explicit>`

Reject absent/ambiguous fields. Do not choose your own item, prerequisites,
paths, or adjacent work.

## 1. Verify isolation and locks

1. Run `git status --short --branch`, `git rev-parse --show-toplevel`,
   `git rev-parse HEAD`, `git worktree list --porcelain`, and
   `git log --oneline -15`.
2. Branch must start `parallel/`, tree must be clean, and `base` must be an
   ancestor of `HEAD`. Fresh worker: `HEAD == base`; resumed worker: only this
   assignment's commits may be ahead.
3. At startup, `integration` must resolve exactly to `base`. Movement makes the
   assignment stale. Never merge, rebase, cherry-pick, or reinterpret it.
4. Every dependency must be an ancestor of `HEAD`; otherwise stop blocked.
5. Use this worktree's `CARGO_TARGET_DIR` for every Cargo-backed command.
6. Never modify another worktree or coordination state. Do not edit `PLAN.md`
   unless path ownership explicitly grants it; the integrator normally owns it.

## 2. Recover the scoped contract

Read in full: `PLAN.md`, `DESIGN.md`, `.pi/skills/next/SKILL.md`, and applicable
`~/Dev/design/doctrines/` files. Inspect the assigned acceptance block, merged
diffs, implementation, and existing evidence before designing changes.

Evidence selection:

- generic mechanism → local invariants, bounds, deterministic geometry, cleanup;
- public extension capability → ordinary builtin + Lua bridge/replacement proof;
- named Zellij behavior → smallest relevant source/test in pinned `ref/zellij`;
- PTY/wire behavior → deterministic protocol tests + daemon seam;
- performance → checked release harness and explicit limits.

Only a named Zellij behavior requires the reference. Verify its revision with
`git -C ref/zellij rev-parse HEAD`; never substitute an ambient checkout.

## 3. Enforce frontier, dependencies, and path ownership

- Trust only interfaces/dependencies merged into `base`. If another worker's
  unmerged decision is required, stop: the assignment is not parallel-safe.
- Implement exactly the assignment. A serial slice remains serial; a wave marker
  permits only assigned siblings.
- Treat `paths` as a hard boundary. Shared manifests, lockfiles, protocol enums,
  central registries, generated indexes, `PLAN.md`, and hot actor loops need one
  named writer or integrator ownership.
- Avoid duplicate adapters/registries, broad renames, formatting churn, and
  unrelated cleanup.
- Preserve architecture: daemon-owned durable pane/PTY state; thin client;
  versioned wire separate from event vocabulary; extension-first policy;
  immutable snapshots + returned actions; bounded dispatch/queues; one
  declaration path; useful no-builtins boot.

## 4. Build acceptance-bearing evidence

- Keep each permanent test responsible for one contract; do not weaken tests.
- Reference-derived observations become compact checked fixtures only when the
  assignment explicitly requires them; normal checks remain offline.
- Public capability must be exercised through the public surface. A new
  `UiAction` reachable from builtins must also be reachable from Lua when action
  marshaling is in scope.
- PTY lifecycle tests must prove process/fd cleanup, stale-generation rejection,
  and bounded queues where relevant.
- Wire changes bump `WIRE_VERSION` and test framing/round trips.

## 5. Shape commits for replay

- Keep commits single-purpose and dependency-ordered.
- Before committing, compare `git diff --name-only base...HEAD` and working-tree
  paths against `paths`; run `git diff --check`.
- Use the established component-prefixed subject. Commit bodies record PLAN
  item, assignment, original base, behavior/evidence, dependencies, remaining
  integration work, and exact checks.

## 6. Verify in isolation

Run focused checks and `cargo fmt --check` while iterating. Before claiming an
implementation slice complete, run the PLAN-required workspace/Nix checks.
Docs-only work uses its explicit structural checks. Record exact commands and
outcomes; another worker's run is not evidence.

## 7. Recheck drift and hand off

Resolve `integration` again. Do not rebase or merge. If it moved from `base`,
mark the result stale and identify likely path/API conflicts.

Finish with:

- branch, original base, current integration tip;
- commits in application order;
- changed paths + shared-file exceptions;
- exact checks/outcomes + isolated target path;
- expected conflicts and merged dependencies relied on;
- exact remainder;
- `status=ready|blocked|stale`.

Do not mark the PLAN item complete.
