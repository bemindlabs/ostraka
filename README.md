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
ostraka runs       # every run this project has recorded
ostraka tui        # browse them in the terminal
ostraka promote ID # give an approved run a branch. Merges nothing
```

A real run — Claude Code wrote the change, Codex reviewed it, neither knew the
other was involved:

```
$ ostraka run "print the current date after the greeting" \
    --author archon --reviewer ephor
run t907222-20260906T132706Z
  pass  executable 1ms
  pass  test     9ms
approved — written by archon, reviewed by ephor
```

The commit it leaves says who did what, because the run directory will not
outlive the repository:

```
Author: archon <archon@ostraka.invalid>

    print the current date after the greeting

    Run: t907222-20260906T132706Z
    Authored-by: archon (claude-code)
    Reviewed-by: ephor (codex)
```

An approved run stops at a commit inside its worktree. `ostraka promote` gives
that commit a branch of its own and then stops too:

```
$ ostraka promote t907222-20260906T132706Z
promoted t907222-20260906T132706Z to promoted/t907222-20260906T132706Z (8db12c5f25b2)
  written by archon, reviewed by ephor

nothing has been merged. To take it further:
  git merge --no-ff promoted/t907222-20260906T132706Z
  gh pr create --head promoted/t907222-20260906T132706Z
```

It refuses a run the gate refused, and it refuses one whose record and whose
commit disagree — the record is a file beside the repository, the trailers are
inside history, and a promotion needs both to say the same thing.

`ostraka tui` is a browser over the same records — what ran, what it was for,
which check failed and what it printed, what the reviewer said, the diff itself,
and `p` to promote an approved one:

```
ostraka  .  ·  2 of 7 runs
+ 09-07 03:54  crates/ostraka-cli/src/runs.rs h…│ t3354339-20260907T035406Z
- 09-07 02:11  In crates/ostraka-core/src/ident…│ approved
                                                │ reviewer ephor — approve
                                                │
                                                │ diff
                                                │ @@ -57,3 +57,33 @@ fn first_line(prompt: &str)
                                                │ +#[cfg(test)]
                                                │ +mod tests {
tokens   claude-code 159.5k in / 4.3k out · codex 14.7k total · copilot-cli ~10.6k in / ~296 out
j/k move · tab diff · space/b scroll · / filter · p promote · r reload · q quit
```

The bottom line totals tokens per backend, from what each vendor said about
itself — a combined figure shown as a total, a rounded one marked with a tilde,
and a backend that reports nothing left out rather than shown as zero.

Tab cycles checks, events and diff. `/` filters on the task text, the run id or
the outcome word. A run that produced no commit says so rather than showing the
commit its branch happens to point at, which is a thing it used to do.

It holds no logic of its own, so `p` cannot approve anything: promotion goes
through the same gate as the command, and a record claiming an approval its
commit does not corroborate is refused there as it is anywhere else.

Refusals are the interesting half. A failing check never reaches the reviewer,
a reviewer that says nothing is a rejection, and the run record keeps the check
output either way — a failed run is the one someone needs to read.

It runs on itself. Cloned fresh, with nothing above it, `ostraka run` takes a
task through this repository's own four cargo checks and an independent review —
the gate this project applies to others is the gate it passes itself, executed by
the same code.

Not done: merging itself — promotion names a branch and leaves the merge to a
person — richer routing, and a live view of a run in progress.

## Installing

```
curl -fsSL https://ostraka.sh/install | sh     # a binary, no toolchain
brew install bemindlabs/ostraka/ostraka
npm install -g ostraka                          # downloads the same binary
cargo install ostraka                           # builds from source
```

None of these are live yet — the names are chosen and unclaimed. Every path
above resolves the same artifact, `ostraka-<tag>-<target>.tar.gz`, and a check
in CI refuses to let the three that parse that name disagree about it.

## A window, if you prefer one

```
ostraka-app            # the same records, in a desktop window
```

The run list, the checks with their output, the events, the diff, and the token
totals per backend — with the panes resizable and the text selectable, which is
most of what a window buys over the terminal view. Promote is a button, and it
goes through the same gate: it cannot approve what `ostraka promote` would
refuse.

`egui` in a window `eframe` opens — pure Rust, no web runtime, one binary per
platform. macOS, Linux and Windows on x86-64, plus Apple silicon; the command
line additionally ships for aarch64 Linux, where a GUI would need a cross
sysroot of window libraries.

## How it is put together

| Crate | Owns |
|---|---|
| `ostraka-core` | Domain types: tasks, gate specs, verdicts, run records, identity. No IO, no process spawning. |
| `ostraka-adapter` | The vendor boundary: profiles, process launch, event normalization. |
| `ostraka-runtime` | The engine: worktrees, gate execution, independent review, run logs. |
| `ostraka` | The `ostraka` binary — the crate you install. |

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

## Vendors

Adapter profiles are data. Three ship, each run end to end before it was
committed:

| Profile | CLI |
|---|---|
| `claude-code` | `claude` |
| `codex` | `codex` |
| `copilot-cli` | `copilot` |

Adding a fourth is a TOML file in `adapters/`, not a release: point `command` at
a CLI on your PATH and `ostraka adapters` will find it. No vendor name appears
anywhere in the runtime. See [`adapters/README.md`](adapters/README.md).

Each profile declares two invocations. The author's may write; the reviewer's may
not — a reviewer that can edit the worktree can make a change it just rejected
pass on the next attempt.

## Working in this repository

[`AGENTS.md`](AGENTS.md) — the same file every backend reads.

---

MIT · [bemindlabs](https://github.com/bemindlabs)
