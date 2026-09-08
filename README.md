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
ostraka init       # write the files a project needs. Overwrites nothing
ostraka check      # validate the project config and every adapter profile
ostraka adapters   # list adapter profiles and whether each can run here
ostraka run "..."  # isolate, execute, gate, review, record
ostraka replay ID  # read a finished run back
ostraka runs       # every run this project has recorded
ostraka tui        # write tasks, watch them run, read them back
ostraka prune      # remove worktrees finished runs left. Branches untouched
ostraka promote ID # give an approved run a branch. Merges nothing
```

`init` writes `ostraka.toml`, the adapter profiles, and two `.gitignore` lines,
and works out the gate from what it finds: cargo's four checks for a Rust
project, `npm test` for a Node one. When it cannot tell, it writes a check that
**fails on purpose** — a gate declaring nothing would approve whatever a
reviewer waved through, and finding that out later is the wrong way to learn it.
Nothing already on disk is touched, so running it twice is a no-op and running
it in a half-configured project completes it. `ostraka tui` offers the same
thing on `i` when you open it somewhere that is not a project yet.

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

`ostraka tui` is where the work happens. You write a task, watch it run, and
write the next one; each is isolated, gated and reviewed by a different agent
than the one that wrote it, and each starts where the last one finished.

```
 ~/src/ostraka                                     on ostraka/t4821-1-20260908T0301Z
 ───────────────────────────────────────────────────────────────────────────────────
 ▌ add a wall-clock ceiling to the gate
 isolate
 prepare
 author
 │ said reading crates/ostraka-runtime/src/gate.rs
 │ done exit 0, 2 file(s)
 gate
 │ ✓ pass  format      126ms
 │ ✓ pass  test       4830ms
 review
 │ said VERDICT-bb6aac54: APPROVE
 ────────────────────────────────────────────── approved — nothing merged ──
 ▌ now add a test for it
 author  ⠹

 ╭ › ───────────────────────────────────────────────────────────────────────────────╮
 │                                                                                   │
 ╰───────────────────────────────────────────────────────────────────────────────────╯
 running  author  ·  8s                   tokens  claude-code 159.5k in / 4.3k out
```

**A thread is a chain of runs, and nothing about the gate is relaxed to get
one.** One run off `HEAD` is the right unit for reviewing a change and the wrong
unit for doing a piece of work: the second task starts by looking at what the
first one wrote, and off `HEAD` it cannot see it. So each run branches from the
run before it, and the breadcrumb says which. What changes is one argument.

A refused run is not built on. The chain advances only where the gate approved,
because continuing from a change it would not take is a way of taking it. `f`
starts a fresh thread from `HEAD`.

**The box has the keys.** What you type is the task; `enter` runs it, `alt-enter`
takes another line, and the up arrow offers back what you have asked here
before. `esc` hands the keys back for a moment — `n` takes the box again. Every
command is also on the `ctrl-x` leader and in the `ctrl-k` palette, and the list
of commands is the authority for all three, so a key cannot do what the palette
has decided not to offer.

**`a` chooses who writes and who reviews.** Routing picks a pair on its own and
is usually right — it prefers a reviewer that is a *different binary* from the
author, which is the property that makes a review worth having. Naming one is a
decision, and a decision is honoured; the gate still refuses a reviewer that is
the author.

**`l` looks up a run**, in a dialog rather than a column: a permanent list costs
half the width of the screen to show something you read once in a while. Opening
one gives it a screen of its own — what it was for, which check failed and what
it printed, what the reviewer said, the diff read from the commit, and `p` to
promote it. `?` lists every key.

Regions are divided by rules rather than by space alone, and colour carries
meaning rather than decoration: the runtime is muted, the author is the accent,
the gate is the colour of something being tested, the reviewer is its own. All
sixteen are ANSI base colours, so the hues are the ones already configured in
that terminal.

The bottom line totals tokens per backend, from what each vendor said about
itself — a combined figure shown as a total, a rounded one marked with a tilde,
and a backend that reports nothing left out rather than shown as zero. A message
about what just happened takes the line while it is worth reading.

It holds no logic of its own, so `p` cannot approve anything and neither can
`enter`: a task typed into the box goes through `run::execute`, which is the
function `ostraka run` calls. Quitting during a run asks it to stop and waits,
rather than leaving a vendor writing into a worktree.

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

### Projects whose dependencies are gitignored

A worktree is a fresh checkout, so `node_modules/`, `.venv/` and `vendor/` are
not in it. Say what to bring:

```toml
[worktree]
base = "worktrees"
link = ["node_modules"]     # symlinked in, not installed per worktree
setup = "make deps"         # or a command, for what linking cannot express
```

Both run before the agent starts, so it can use the project's tools too, and a
failure is reported as a setup problem rather than as a rejected change.

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
