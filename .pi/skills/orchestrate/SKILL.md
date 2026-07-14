---
name: orchestrate
description: Coordinate ekko from its first open PLAN item through exclusive serial gates and explicit dependency-ready waves with exact base/path locks and focused evidence.
---

# ekko PLAN orchestrator

Act as sole coordinator/integrator. Never delegate orchestration itself.
Optimize for a green integration branch and safe dependency order.

Modes:

- `run` or omitted: recover active batch, otherwise select, launch, await,
  review, and integrate the frontier batch;
- `status`: observe/report only;
- `plan`: print next valid batch + locks only.

Additional text may narrow scope but not skip the PLAN frontier.
`${PI_ORCHESTRATE_MAX_WORKERS:-4}` is the hard worker limit. Serial gates use
one worker.

## 1. Recover before creating work

1. Read `PLAN.md`, `DESIGN.md`, `.pi/skills/parallel-plan/SKILL.md`,
   `.pi/skills/next/SKILL.md`, and applicable doctrines in full.
2. Run `git status --short --branch`, `git rev-parse HEAD`,
   `git log --oneline -20`, and `git worktree list --porcelain`.
3. Inspect `${PI_PARALLEL_STATE_ROOT:-$HOME/.local/state/ekko-parallel}` and
   `parallel/*` branches. If a batch exists, run
   `../parallel-plan/monitor.sh --once`; recover it, never duplicate it.
4. For exited workers inspect logs, branches, worktrees, commits, exit codes.
5. Integration must be clean, on a non-`parallel/*` branch, and at the ref
   workers receive as `integration`. Preserve unrelated dirt and stop.
6. Record `ref/zellij` revision when present. Use only for PLAN-named focused
   mechanism/interaction evidence, never whole-product parity.

## 2. Compute the exact PLAN frontier

1. Verify checked items against merged history and acceptance evidence; reopen
   or repair false closure first.
2. Scan top-to-bottom. First unchecked item = **F**. Nothing after F is eligible
   except siblings with F's explicit wave marker/readiness gate.
3. Resolve dependencies, `serial after`, and prose gates against checked items
   and merged commits. Missing prerequisite blocks the frontier.
4. If F is serial, owns a shared contract/hot file, or has no wave marker,
   choose a **serial gate**.
5. If F has explicit **Wave X** and its gate is satisfied, candidates are F plus
   subsequent dependency-ready Wave X siblings until the first serial or
   different-wave boundary. Never invent waves from apparently disjoint code.
6. A dependent serial item waits until the full named wave is integrated,
   checked, green. If worker limits split a wave, it remains the frontier.

## 3. Design one locked batch

A batch is exactly one:

- **serial gate:** one worker for F or one acceptance-bearing slice;
- **parallel wave:** 2–4 workers for coherent explicit-wave deliverables.

A wave assignment is valid only when dependencies/interfaces are in the common
base, paths are disjoint, evidence runs in isolation, and no sibling's unmerged
semantic decision is required.

Hot/shared paths—root manifests/lockfiles, `flake.nix`, `PLAN.md`, `DESIGN.md`,
wire enums, central registries, `main.rs`, and actor loops—have one writer or
remain integrator-owned.

Record per worker:

- `slug`, `item`, `assignment`;
- exact `base` + `integration`;
- merged `dependencies` or `none`;
- exclusive `paths` + named shared-file exceptions;
- evidence class.

All workers in a batch receive one base. Persist `kind=serial|parallel`.
`plan` prints frontier reasoning, worker table, locks, evidence, and gate closure
criteria, then stops.

## 4. Assign evidence

Use one primary class:

- mechanism invariant / deterministic geometry / resource cleanup;
- public extension + Lua reachability/replacement;
- named focused Zellij behavior;
- wire round-trip / daemon seam;
- measured release performance.

Only the Zellij class reads `ref/zellij`. It licenses no adjacent parity.
Normal checks use local fixtures and never ambient repositories.

## 5. Launch without moving the base

Choose batch ID `YYYYMMDD-HHMM-topic`. Recheck clean integration and that both
`HEAD` and `integration` equal base. Launch each worker once:

```sh
../parallel-plan/spawn-worker.sh \
  --wave "$batch" \
  --kind "$kind" \
  --slug "$slug" \
  --item "$item" \
  --base "$base" \
  --integration "$integration" \
  --dependencies "$dependencies" \
  --paths "$paths" \
  --assignment "$assignment"
```

Launcher creates isolated branch/worktree, copies skills, links local references,
sets a worktree-local Cargo target, records metadata, disables ambient resources,
and loads only `parallel-plan`.

Parallel launch failure may leave independent launched workers running with the
failure recorded. Serial launch failure stops with gate open.

## 6. Await without interference

```sh
../parallel-plan/monitor.sh --wave "$batch" --wait --interval 30
```

Use `--once` for status/diagnosis. Until all exit: do not move integration,
edit worker trees/state, cross-merge workers, launch another batch, or treat
process exit as readiness. Preserve blocked/cancelled worktrees.

## 7. Review and classify

For each worker inspect metadata, full log/handoff, exit code, clean status,
`git log --reverse --format=fuller <base>..<branch>`, commit bodies,
`git diff --stat <base>...<branch>`, paths vs ownership, evidence, and design fit.

Classify:

- **ready:** clean, committed, scoped, replayable, credible evidence;
- **blocked:** missing prerequisite/evidence/acceptance;
- **stale:** base/API semantics moved;
- **invalid:** path/scope breach, speculative behavior, contaminated evidence,
  unrelated changes, or wrong reference use.

Exit 0 is necessary, not sufficient.

## 8. Integrate and satisfy the gate

Only orchestrator edits integration or `PLAN.md`.

1. Order ready commits by actual dependency, least shared first.
2. Cherry-pick single-purpose commits. Resolve only expected integrator-owned
   reconciliation; abort/reassign semantic conflicts from the new base.
3. Run focused checks after each accepted slice.
4. Reconcile manifests/indexes/docs in a dedicated integrator commit.
5. After batch run formatting, affected suites, `cargo test --workspace`,
   `nix build`, and `nix flake check` when implementation changed. Worker checks
   do not replace integrated checks.
6. Update `PLAN.md` truthfully; update `DESIGN.md` for contract/role changes;
   leave integration clean.

A serial gate closes only when all acceptance is integrated, checked, documented,
and clean. A wave closes only when every required sibling is integrated and the
combined tree is green. Partial work leaves the gate/wave open.

## 9. Continue or report

In `run`, continue only if the previous gate/wave is fully green and enough
context remains for another complete cycle. Stop on active workers, unresolved
gates, failed checks, user decisions, or blocked/stale work.

Report compactly:

- integration base → final tip;
- batch kind/ID + worker classifications;
- gate/wave completion;
- commits integrated/rejected/blocked/rescheduled;
- exact integrated checks/outcomes;
- retained processes/worktrees;
- next first open dependency-ready gate/wave.
