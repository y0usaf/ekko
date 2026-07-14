---
name: next
description: Continue ekko from the first open dependency-ready PLAN.md item, respecting serial gates, explicit waves, focused reference use, extension-first design, and Nix acceptance.
---

# Next PLAN item

Use this skill for one non-orchestrated continuation session. `PLAN.md`,
`DESIGN.md`, and Git history are the durable handoff; do not infer a different
roadmap from the current implementation.

## 1. Recover the frontier

1. Run `git status --short --branch`, `git rev-parse HEAD`, and
   `git log --oneline -15`. Preserve unrelated user changes; do not begin from
   unexplained dirt.
2. Read `PLAN.md` and `DESIGN.md` in full, then the applicable doctrine files
   under `~/Dev/design/doctrines/`.
3. Inspect implementation, evidence, and recent diffs for the last closed item.
   A checked box without landed acceptance evidence is plan drift: repair or
   truthfully reopen it before advancing.
4. Scan unchecked PLAN items top-to-bottom. The first open item is the frontier.
   It is dependency-ready only when every explicit dependency, serial gate, and
   preceding wave gate is checked and present in merged history. Do not skip a
   blocked/stale first item for a later heading.
5. A **serial** item owns the repository frontier until its acceptance criteria
   are integrated. For an explicit **Wave**, `/next` may implement only the
   first open wave item and must not launch or claim siblings.

If the item is too large, land one acceptance-bearing coherent slice of that
same item and record the exact remainder in `PLAN.md`.

## 2. Recover only the required contract

- Follow `DESIGN.md`: core Rust = generic mechanism; stock policy = ordinary
  `ekko-builtins` consumers of the public API; user replacements use that same
  API through Rust or Lua; snapshots in/actions out; zero-builtins boot remains
  real.
- `ref/zellij` is a focused reference for pane/session mechanisms and named
  interaction behavior only when the PLAN item calls for it. Inspect the
  smallest relevant source/test; it is not a whole-product parity mandate.
- `ref/phi` and `ref/pi-harness` may supply focused composition/rendering clues,
  never requirements.
- Generic mechanisms need invariant, bounds, and cleanup evidence. Public
  extension capability needs a real builtin plus Lua reachability where the
  bridge exposes that capability. PTY behavior needs a daemon seam test.
- Never make normal checks depend on ambient sibling checkouts or update a
  reference-derived fixture from ekko output.

## 3. Implement and verify

- Implement only the frontier item/slice. Avoid broad formatting, generated
  churn, compatibility scaffolding, and unrelated cleanup.
- Preserve source neutrality: builtins get no privileged capability, lifecycle,
  priority, or declaration path.
- Keep dispatch bounded, queues/backlogs capped, PTY resources reclaimable, and
  wire changes explicit via `WIRE_VERSION`.
- Use a worktree-local `CARGO_TARGET_DIR` for direct Cargo iteration. Final
  build/verification claims are Nix claims.
- Before closing implementation: run `cargo fmt --check`, focused checks,
  `cargo test --workspace`, `nix build`, and `nix flake check` unless the PLAN
  item explicitly defines a narrower docs-only acceptance path. Record exact
  commands/outcomes; do not claim checks not run.

## 4. Close the loop

1. Re-read the full acceptance block and inspect `git diff --check`.
2. Update `PLAN.md` in the same change: check the item only when every criterion
   passed; otherwise record the landed slice and exact remainder.
3. Update `DESIGN.md` when contracts, crate roles, wire shape, or doctrine
   conformance changed.
4. Commit in the established component-prefixed style. Explain mechanism/policy
   placement, evidence, and checks.
5. End with: commits, changed paths, checks/outcomes, exact remainder, and the
   resulting first open dependency-ready item.
