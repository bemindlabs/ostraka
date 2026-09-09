<pre align="center">
 ██████╗ ███████╗████████╗██████╗  █████╗ ██╗  ██╗ █████╗ 
██╔═══██╗██╔════╝╚══██╔══╝██╔══██╗██╔══██╗██║ ██╔╝██╔══██╗
██║   ██║███████╗   ██║   ██████╔╝███████║█████╔╝ ███████║
██║   ██║╚════██║   ██║   ██╔══██╗██╔══██║██╔═██╗ ██╔══██║
╚██████╔╝███████║   ██║   ██║  ██║██║  ██║██║  ██╗██║  ██║
 ╚═════╝ ╚══════╝   ╚═╝   ╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
</pre>

<h1 align="center">Ostraka</h1>

<p align="center"><strong>Run agent fleets you can actually review.</strong></p>

<p align="center">
  <a href="https://github.com/bemindlabs/ostraka/actions/workflows/ci.yml"><img alt="gate" src="https://github.com/bemindlabs/ostraka/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
  <img alt="rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
  <img alt="runtime" src="https://img.shields.io/badge/runtime-none-lightgrey">
</p>

**Contents** · [What it is](#what-it-is) · [The name](#the-name) ·
[Why](#why) · [Status](#status) · [A workspace](#a-workspace) ·
[Installing](#installing) · [A window](#a-window-if-you-prefer-one) ·
[How it is put together](#how-it-is-put-together) ·
[The one invariant](#the-one-invariant-worth-reading-the-code-for) ·
[Vendors](#vendors) · [Which agent for which job](#which-agent-for-which-job)

## What it is

A command-line tool that runs coding agents — Claude Code, Codex, Copilot CLI,
and others — and **will not hand you a mergeable result on their word alone.**

You give it a task. It makes a fresh git worktree, hands the task to one agent,
then does the two things the agent cannot be trusted to do for itself: it runs
your project's own checks as real subprocesses, and it asks a *different* agent
to review the diff. Only if both pass does the run produce a merge token — a
type with no public constructor, which is the compiled-in version of "the thing
that wrote the change does not get to approve it".

```console
$ ostraka run "add a Usage section to README.md" \
    --adapter claude-code --review-adapter codex
run t2237200-1-20260909T062815Z
  pass  check    25ms
approved — written by author, reviewed by reviewer
```

The interesting output is the other one. Both of these are real runs against the
same small project, and the second is the whole argument for the tool:

```console
$ ostraka run "add greet.sh, which prints exactly 'hello, world'" \
    --adapter claude-code --review-adapter codex
run t2225730-1-20260909T062649Z
  pass  check    25ms
refused — reviewer rejected: greet.sh lacks executable permission and
          therefore cannot be run directly as the requested shell script.
```

The project's own check passed. A second agent, from a different vendor,
looking only at the diff, caught what the check did not test for — and because
it did, no merge token was minted and there is nothing to merge.

Nothing merges by itself either way. An approved run stops at a commit in its
own worktree; `ostraka promote` gives it a branch; merging stays a person's act.
A week later `ostraka replay` reads the whole thing back — the streams, the
check output, the reviewer's actual words.

It is one binary with no runtime under it. One agent writing and one reviewing
is the common case; running several vendors at once is the same machinery with
more lines of work open.

## The name

**Ostraka** — *ὄστρακα*, Greek, plural of *ostrakon*. Say it **OSS-tra-ka**.

They were potsherds: broken bits of pottery, the cheapest writing surface an
ancient city had. Athens used them as ballots. When the assembly voted on
whether to expel a citizen, each voter scratched a name on a sherd and dropped
it in — which is where the English word *ostracism* comes from. The judgment
was written down by the people making it, on a medium nobody had to be rich to
use.

The same sherds are why we know what ordinary life cost. Papyrologists dig up
*ostraka* by the thousand carrying receipts, tax notes, school exercises and
shopping lists — the everyday record, surviving two thousand years for exactly
one reason: the medium was cheap, so people used it, and it was durable, so it
lasted.

Both halves are the product. **A decision is written down by the parties to it**
— an agent writes, the checks run, a different agent reviews, and the verdict is
recorded rather than asserted. **And the record is the cheap, durable part** —
a run leaves a record anyone can re-read, and the account that has to survive is
in the commit, not in a summary somebody has to take on trust.

The agents are named for civic offices for the same reason — an archon plans
and an ephor gates, because power that checks itself is the whole idea.
[`PHILOSOPHY.md`](PHILOSOPHY.md) has the rest of it.

## Why

You already have a coding agent, and it works. The gap is not capability. It is
that when a run finishes, the only account of what happened was written by the
same process that did it.

The agent says the tests pass. It says it touched three files. Both are output,
not evidence, and the way you find out otherwise is by reading the whole diff —
which is the work you were trying not to do. Ostraka's answer is to take that
account away from the agent:

**The checks run here, not in the summary.** Format, lint, test and build are
executed as real subprocesses in the worktree. A run that claims a passing suite
without one having run cannot produce a mergeable result. What changed is read
from `git status` in the worktree, never from the agent's description of its own
edits.

**A reviewer with the same blind spots is not a reviewer.** Review goes to a
different profile than the one that wrote the change, and routing prefers a
different *binary* — a different training lineage, a different system prompt, a
different set of things it does not think to check. The runtime will not approve
a change whose reviewer is its author, and that is a type with no public
constructor rather than a setting somebody can turn off.

**Your checkout is not the workspace.** Every task runs in its own git worktree.
The repository you cloned in is left the way its owner left it — no stashes, no
half-applied edits, no branch you did not ask for.

**A run does not quietly inherit you.** Left alone, an agent reads your personal
instruction files, your MCP servers and your settings, and behaves like you — so
the same task answers differently on someone else's machine. Each profile runs
against its own vendor home, and the environment a vendor gets is built rather
than inherited. Where a vendor offers no way to isolate,
[`adapters/README.md`](adapters/README.md) names it instead of implying
otherwise.

**Nothing merges by itself.** An approved run stops at a commit inside its
worktree. `ostraka promote` gives it a branch. Merging stays a person's act, and
a run that never obtained a merge token cannot be promoted at all.

**What happened stays readable.** Every run is a record — the streams, the check
output, the reviewer's actual words, the verdict. `ostraka replay` reads one back
a week later, which is when the question usually gets asked.

None of this needs a fleet. One agent writing and one reviewing is the common
case and the one Ostraka is quickest at. Configuring several vendors is the same
machinery with more than one line of work open — a capability, not a
prerequisite — and, today, still one run at a time: the browser refuses to start
a second while one is going, and parallel execution is an open question rather
than a shipped feature.

## Status

**1.0 — early, and it runs.** A task goes end to end: isolated worktree, an
agent writes, the project's checks actually execute, a *different* agent
reviews, and the result is a replayable record.

```console
ostraka init       # write the workspace layout. Overwrites nothing
ostraka check      # validate the workspace, its profiles and its repositories
ostraka check --fix  # and walk the steps out of what it found, asking first
ostraka adapters   # list adapter profiles and whether each can run here
ostraka task add "..."  # write work down for later
ostraka tasks           # what is waiting, going, and done
ostraka run --next      # take the oldest task and run it
ostraka run "..."  # isolate, execute, gate, review, record
ostraka run "..." --from ID   # continue an approved run instead of HEAD
ostraka replay ID  # read a finished run back
ostraka runs       # every run this workspace has recorded
ostraka tui        # write tasks, watch them run, read them back
ostraka prune      # remove worktrees finished runs left. Branches untouched
ostraka promote ID # give an approved run a branch. Merges nothing
ostraka completion bash  # a completion script, generated from this parser
```

## A workspace

Ostraka works from a workspace rather than from inside the repository it is
working on:

```text
<workspace>/
  .ostraka/
    ostraka.toml      how runs are made here
    adapters/*.toml   the vendor profiles this machine has
    runs/             what happened                    (not committed)
    worktrees/        where agents work                (not committed)
  repositories/<name> what you cloned in to be worked on
  notes/              what was worked out along the way
  skills/             what you expect of a run                (optional)
```

`notes/` and `skills/` are linked into every worktree and are the two
directions of the same idea. Notes are what runs worked out and wrote down;
skills are what you wrote down for runs to follow — procedures, conventions,
the way things are done here. Neither is part of the change a reviewer judges,
and an author is told about whichever of them is actually there.

Skills belong to the workspace rather than to a vendor deliberately. Every CLI
has some private notion of skills or plugins, kept in a home directory that
isolation relocates, so a run leaning on those would answer differently on a
different machine. One the workspace owns is one every vendor gets and every
machine reproduces. `init` does not create the directory: make it when you have
something to put in it.

Everything the runtime owns is in one directory, so **a repository cloned in is
left as its owner left it**: no config appears at its root, no worktrees are
made inside it, and deleting the workspace deletes every trace of Ostraka
having been used.

**A repository may bring its own `ostraka.toml`.** How a project is verified is
a property of that project — a workspace holding a Rust repository and a Node
one cannot have one gate between them — so a repository's own file wins
entirely where there is one, and the workspace's is the answer where there is
not. `ostraka check` says which answered for each.

**`notes/` is linked into every worktree.** An agent writing there writes into
the real directory, so what it worked out survives the run that worked it out —
including a refused one, which is the run whose notes are worth the most. The
link points out of the checkout, so none of it lands in the diff a reviewer
judges: notes are what was learned, the diff is what was changed.

`init` writes that layout and works out the gate from what it finds — cargo's
four checks for a Rust repository, `npm test` for a Node one. When it cannot
tell, it writes a check that **fails on purpose**: a gate declaring nothing
would approve whatever a reviewer waved through, and finding that out later is
the wrong way to learn it. Nothing already on disk is touched, so running it
twice is a no-op and running it in a half-finished workspace completes it.
`ostraka tui` offers the same thing on `i`, and says there what setting up will
not fix.

Where the workspace holds one repository, nothing has to name it. Where it
holds several, `--repository` does — and the browser has a picker on `w`.

**A workspace with nothing in it can start one.** Cloning needs a URL only you
have; starting needs a name, so `w` then `n` takes one and `git init`s a
repository there. It stops at `git init` on purpose: the commit a worktree
branches from is left to the guided fix, which asks before it commits anything
and reports git's own words when there is no author configured.

A real run — Claude Code wrote the change, Codex reviewed it, neither knew the
other was involved:

```console
$ ostraka run "print the current date after the greeting" \
    --author archon --reviewer ephor
run t907222-20260906T132706Z
  pass  executable 1ms
  pass  test     9ms
approved — written by archon, reviewed by ephor
```

The commit it leaves says who did what, because the run directory will not
outlive the repository:

```text
Author: archon <archon@ostraka.invalid>

    print the current date after the greeting

    Run: t907222-20260906T132706Z
    Authored-by: archon (claude-code)
    Reviewed-by: ephor (codex)
```

An approved run stops at a commit inside its worktree. `ostraka promote` gives
that commit a branch of its own and then stops too:

```console
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

```text
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
promote it. `?` lists every key, and `q` asks before it leaves —
quitting can discard the task in the box and stop a run that is going.

`ctrl-t` opens another line of work beside this one — its own thread, its own
repository, its own half-written task — and `ctrl-]` moves between them. They
are switched between rather than shown side by side: two transcripts on an
eighty-column terminal are two transcripts nobody can read. Runs still happen
one at a time across all of them, because the request to stop is a single flag;
a pane with something going is marked on the bar, and says so by name when it
finishes.

Typing `/` in the box offers the commands at the box, filtered as you type;
`/settings` shows what this thread and this project are set to. Opening a
repository that has runs behind it summarises the last few rather than claiming
nothing has been asked here. And where something is in the way of a run at all —
a directory git has never heard of, a repository with no commits — the browser
says what it is and walks through the steps out of it, one keypress at a time,
showing each command before it runs it.

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

```console
curl -fsSL https://ostraka.sh/install | sh     # a binary, no toolchain
npm install -g ostraka                          # downloads the same binary
cargo install ostraka                           # builds from source

brew tap bemindlabs/ostraka https://github.com/bemindlabs/ostraka
brew install ostraka
```

The tap takes a URL because the formula lives in this repository rather than in a
second one named `homebrew-ostraka`. Without the URL, `brew` appends that prefix
itself and looks somewhere that does not exist.

None of these are live yet — the names are chosen and unclaimed. Every path
above resolves the same artifact, `ostraka-<tag>-<target>.tar.gz`, and a check
in CI refuses to let the three that parse that name disagree about it.

## A window, if you prefer one

```console
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

<details>
<summary><b>Projects whose dependencies are gitignored</b></summary>


A worktree is a fresh checkout, so `node_modules/`, `.venv/` and `vendor/` are
not in it. Say what to bring:

```toml
[worktree]
base = ".ostraka/worktrees"   # under the directory ostraka owns
link = ["node_modules"]     # symlinked in, not installed per worktree
setup = "make deps"         # or a command, for what linking cannot express
```

Both run before the agent starts, so it can use the project's tools too, and a
failure is reported as a setup problem rather than as a rejected change.

</details>

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

Adapter profiles are data. Six ship, each run end to end before it was
committed:

| Profile | CLI |
|---|---|
| `claude-code` | `claude` |
| `codex` | `codex` |
| `copilot-cli` | `copilot` |
| `grok` | `grok` |
| `kimi-cli` | `kimi-cli` |
| `agy` | `agy` |

Adding a seventh is a TOML file in `adapters/`, not a release: point `command` at
a CLI on your PATH and `ostraka adapters` will find it. No vendor name appears
anywhere in the runtime. See [`adapters/README.md`](adapters/README.md).

Each profile declares two invocations. The author's may write; the reviewer's may
not — a reviewer that can edit the worktree can make a change it just rejected
pass on the next attempt.

## Which agent for which job

`ostraka bench` asks the same tasks of every candidate and tabulates what the
gate said. It can answer that question because a run already ends in an
objective verdict — the project's own checks, executed in a worktree, on the
change that was actually written. Nothing new judges anything; the measurement
is the pipeline, repeated.

Declared in `.ostraka/bench.toml`, beside the workspace rather than inside the
repository: a gate is what a repository agrees on, a benchmark is what an
operator is curious about this week.

```toml
# Held constant, so the numbers are about the candidate rather than the pair.
# The rest stand in where a candidate is the first one.
reviewers = ["claude-code", "codex"]

[[task]]
id = "greet"
prompt = "Add an executable greet.sh that prints exactly `hello, world`."

[[candidate]]
adapter = "codex"

[[candidate]]
adapter = "grok"
models = ["an-id-this-machine-can-run"]
```

```
$ ostraka bench --dry-run
2 cells: 1 task x 2 candidates. Each one is an authoring run and a review.

$ ostraka bench
greet
      codex:default    43s  wrote nothing
      grok:default     10s  checks failed: check

standings
    0%  grok:default    0 approved / 1 judged  gate 0  0 incomplete   10s   45.9k
    0%  codex:default   0 approved / 1 judged  gate 0  0 incomplete   43s   11.0k
```

That is a real run, and it is worth reading for what the columns keep apart.
`grok` wrote the script and left it mode 600, so `test -x` failed — a change
that was judged and found wanting. `codex` wrote nothing at all. Both are
zero, and only one of them is about the work.

Three things the table refuses to do:

- **A cell that never reached a verdict does not count against the candidate.**
  A rate limit, an expired credential, a ceiling or a stop is an outcome about
  the run. Averaging those into a pass rate would rank a vendor by how reliably
  its billing worked. A candidate with no judged cells shows `—`, not `0%`.
- **The gate is the score; the review is reported beside it.** A failing check
  is a fact about the change. A reviewer's verdict is an opinion about it, from
  a model with its own preferences. Both are printed and only the first is
  ranked on.
- **Every cell names the run it came from,** so `ostraka replay <id>` reaches
  the diff, the check output and the reviewer's words. A number nobody can get
  back to the evidence for is a number nobody should act on.

Every cell is a full run, so a matrix costs what its size says it does.
`--dry-run` prints that size without spending it.

## Working in this repository

[`AGENTS.md`](AGENTS.md) — the same file every backend reads, and the one place
the rules that a compiler cannot enforce are written down.

---

<p align="center">
  MIT · <a href="https://github.com/bemindlabs">bemindlabs</a> ·
  <a href="AGENTS.md">AGENTS.md</a> ·
  <a href="PHILOSOPHY.md">PHILOSOPHY.md</a> ·
  <a href="adapters/README.md">adapters</a>
</p>
