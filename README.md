# Ostraka

**Run agent fleets you can actually review.**

Ostraka runs coding agents from different vendors against one task, in isolated
git worktrees, and refuses to hand back a mergeable result until the project's
own checks have run and an agent that did not write the change has approved it.

> *ostraka* — the potsherds an assembly wrote its judgments on, and the everyday
> receipts that survived because the medium was cheap and durable. The record is
> the point.

## Status

**3.0 — early, and it runs.** A task goes end to end: isolated worktree, an
agent writes, the project's checks actually execute, a *different* agent
reviews, and the result is a replayable record.

```
ostraka check      # validate the project config and every adapter profile
ostraka adapters   # list adapter profiles and whether each can run here
ostraka run "..."  # isolate, execute, gate, review, record
ostraka replay ID  # read a finished run back
```

A run in full:

```
$ ostraka run "add a greeting note" --adapter writer --review-adapter reviewer \
    --author archon --reviewer ephor
run t3289514-20260905T151159Z
  pass  not-empty 0ms
  pass  greeting 2ms
approved — written by archon, reviewed by ephor
```

Refusals are the interesting half. A failing check never reaches the reviewer,
a reviewer that says nothing is a rejection, and the run record keeps the check
output either way — a failed run is the one someone needs to read.

Not done: merging (a run commits inside its worktree and stops there), richer
routing, and a TUI.

## How it is put together

| Crate | Owns |
|---|---|
| `ostraka-core` | Domain types: tasks, gate specs, verdicts, run records, identity. No IO, no process spawning. |
| `ostraka-adapter` | The vendor boundary: profiles, process launch, event normalization. |
| `ostraka-runtime` | The engine: worktrees, gate execution, independent review, run logs. |
| `ostraka-cli` | The `ostraka` binary. |

Four crates, not one per agent role. The roles are a protocol, not a deployment
unit, and splitting them would only create a dependency cycle.

## The one invariant worth reading the code for

`MergeToken` in `ostraka-runtime::gate` has private fields and no public
constructor. The only way to obtain one is `gate::evaluate`, which requires
both proof that every required check actually ran and passed, and an approval
whose reviewer differs from the change's author.

An orchestrator holding every other type in the crate still cannot produce one.
That is why the gate is not a separate crate: across a crate boundary it would
have to be injectable, and an injectable gate is a bypassable gate.

## Adding a vendor

Adapter profiles are data. Drop a TOML file in `adapters/`, point `command` at a
CLI on your PATH, and `ostraka adapters` will find it. No vendor name appears
anywhere in the runtime, and adding one does not need a release. See
[`adapters/README.md`](adapters/README.md).

## Working in this repository

[`AGENTS.md`](AGENTS.md) — the same file every backend reads.

---

MIT · [bemindlabs](https://github.com/bemindlabs)
