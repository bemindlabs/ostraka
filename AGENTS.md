# AGENTS.md

Instructions for any agent working in this repository. This file is the single
source of truth. `AGY.md`, `CODEX.md`, `COPILOT.md`, `GROK.md`, `KIMI.md`,
`OLLAMA.md` and `OPENAI.md` are symlinks to it. `CLAUDE.md` is a file containing
only `@AGENTS.md`, because a plugin rewrites CLAUDE.md in every directory it
observes and replaces a symlink with a copy that then goes stale — an import
line survives being rewritten. Put instructions here, never there.

`./scripts/check-hygiene.sh` enforces all of that, and CI runs it first.

Ostraka runs coding agents from different vendors as one reviewed fleet. It
favors no vendor — including in how this repository is laid out.

## Language

English only. No exceptions, no bilingual pairs. This covers code, comments,
docs, commit messages, CLI output, log lines and error strings.

The rule is about the working language, and the failure it prevents is a reader
who needs a translation before they can use the tool. So one thing it does not
forbid: a source word written once, in the sentence that defines it, glossed in
English on the spot — `Ostraka` is a Greek word and the README says so in Greek,
with the pronunciation beside it. Nothing downstream of that sentence may
require having read it. Everything else stays English, including any second
occurrence of the same word: a term that has to be introduced twice is a
glossary, which is the thing being refused.

## Layout

```
crates/ostraka-core      domain types; no IO, no process spawning
crates/ostraka-adapter   vendor boundary: profiles, launch, event normalization
crates/ostraka-runtime   engine: worktrees, gate, review, run records
crates/ostraka           the `ostraka` binary; the crate people install
crates/ostraka/templates the adapter profiles `init` writes into a new project
crates/ostraka-app       the `ostraka-app` desktop window; a second view
adapters/                one TOML profile per vendor CLI — data, not code
scripts/install.sh       the curl one-liner; downloads a released binary
npm/                     the npm shim; downloads the same binary, ships none
scripts/check-hygiene.sh the AGENTS.md rules a compiler cannot enforce
Formula/ostraka.rb       Homebrew tap formula, bumped by the release workflow
```

## The gate

Declared in `ostraka.toml`, executed by the runtime, and run in CI on every pull
request. The gate this project applies to others is the gate it passes itself:

```
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Run all four before proposing a change, plus `./scripts/check-hygiene.sh`.

The hygiene check is deliberately *not* one of the four. Gate checks run inside
a worktree while an agent is working in it, and an editor plugin writing a file
there would refuse correct work for a reason that has nothing to do with the
change. It belongs in CI, on a clean checkout.

## Rules that are structural, not stylistic

1. **No vendor names in the runtime.** Not in `ostraka-runtime`, not in
   `ostraka-adapter`, not in prompts. A supported CLI is a profile in
   `adapters/`. If you find yourself writing a product name in Rust, the design
   is wrong.
2. **No model identifiers in code.** They arrive through configuration.
3. **`MergeToken` stays unforgeable.** Do not add a public constructor, a
   `From` impl, or a test helper that builds one outside `runtime::gate`. Do not
   move the gate into its own crate.
4. **`ostraka-core` stays inert.** No process spawning, no network, no async
   runtime. Its dependencies are serde, serde_json, toml and thiserror; adding
   another needs a reason written down.
5. **Fail safe.** An unparseable verdict, a check that could not start, a
   reviewer that crashed — all of these are rejections, never passes.
6. **Read what happened from git, not from the agent.** Files touched come from
   `git status` in the worktree, never from a vendor's account of its own edits.

## Decisions that are settled

These were open questions across two implementation notes. They are answered;
reopening one needs a reason, not a preference.

**Ostraka works from a workspace, not from inside what it is working on.**
`.ostraka/` holds the configuration, the adapter profiles, the run records and
the worktrees; `repositories/<name>` holds what is being worked on;  `notes/`
holds what was worked out along the way. A repository cloned in is left as its
owner left it — no config at its root, no worktrees inside it, and deleting the
workspace deletes every trace of Ostraka having been used. `orchestrator::Places`
is that topology as a parameter: a run happens *in* a repository, *beside* a set
of worktrees, and is *recorded* somewhere that may be neither, and deriving the
second two from the first was the assumption that a workspace held exactly one
repository.

**A workspace can start a repository, and stops at `git init`.** "Nothing to
work on yet" has two halves and only one of them needs the operator: cloning
needs a URL nobody here knows, and starting needs a name. `w` then `n` takes
one. It does not make the first commit — that is the guided fix's step, which
already asks before it commits whatever is lying in a directory and already
reports git's own words when there is no author configured, and duplicating it
would be a second place for that to go wrong.

A name is one path segment. A slash, a leading dot or a `..` is refused rather
than obeyed: a browser that made a repository two levels up because somebody
typed a slash is a browser nobody should leave open.

**What is in the way is said on the work screen, not only on the setup one.**
The setup screen is gone the moment setting up is done, which used to leave a
workspace that was configured, had nowhere to run anything, and said so only if
you pressed enter and found out.

**How a project is verified is that project's business.** A workspace holding a
Rust repository and a Node one cannot have one gate between them, so a
repository's own `ostraka.toml` wins entirely where it has one and the
workspace's answers where it does not. `check` says which answered for each.
The gate is still the repository's agreement, wherever it is read from — the
browser shows it and does not edit it.

**`notes/` is linked into every worktree, and is not configuration.** An agent
writing there writes into the real directory, so what it worked out survives
the run — including a refused one, which is the run whose notes are worth the
most. The link points out of the checkout, so none of it reaches the diff a
reviewer judges: notes are what was learned and the diff is what was changed,
and confusing the two would put an agent's scratchpad in front of a reviewer as
if it were part of the change.

**A workspace resolves where it is.** The command line defaults to `.`, and a
relative worktree path built from it means two different directories: `git
worktree add` resolves it against the repository it is run in, and everything
else resolves it against the workspace. The agent then works in a checkout git
has never heard of and the first thing to notice is `git status` failing two
minutes later. Found by running it, in a real workspace, rather than by
reasoning about it.

**The repository is self-contained.** The fleet in the sibling workspace —
`agents/archon`, `agents/ephor` and the rest — is the team that *builds*
Ostraka. It is not an input to the runtime, which reads `ostraka.toml` and
`adapters/` and nothing else. The offices survive the move into code as types
and identities: `MergeToken` is what "the orchestrator cannot approve itself"
compiles to. Verified by cloning this repository somewhere with no workspace
above it and running `ostraka run` on itself, gate and review included.

**No second runtime.** No Bun, no Node, no TypeScript. Adapters are subprocesses
speaking stdio; `std::process` covers that completely. A runtime dependency
would destroy the one distribution advantage this project has — a curl one-liner
that then requires you to install a runtime is not a one-liner. Revisit only for
a genuinely TypeScript-native surface, such as an editor extension.

**SemVer, not CalVer.** crates.io, npm and Homebrew all reason about SemVer
ranges; a date-shaped version fights all three. Crate versions and git tags are
the same number.

**The first release is `v1.0.0`.** It was `v3.0.0` for most of this project's
life, and the reason it is not any more is the cost that decision named for
itself: to anyone meeting this on crates.io, a first published version of 3.0.0
implies two earlier majors they can neither find nor install. BWOC 1.x and 2.x
did precede it and `PHILOSOPHY.md` traces the line, but that lineage is a story
this repository tells, not a range a resolver can reason about — and a registry
is read by resolvers. The number should describe what is published rather than
what came before it.

Nothing about the lineage is disowned by the change. The philosophy document
still says what preceded this and why; what stops is the version number making a
claim about registries that the registries cannot show.

SemVer's promise holds from the tag onward either way: 1.0.0 is a stable API
under SemVer, and that is the only thing a version number is obliged to mean.

Every version site says `1.0.0`: the workspace, the four crate manifests that
inherit it, and `npm/package.json`, which `check-hygiene.sh` keeps in step.
`Formula/ostraka.rb` says `0.0.0` on purpose — it is a placeholder the release
job rewrites once artifacts exist, and leaving it obviously wrong is how it
stays obvious that nothing has been released.

**Run records are not committed.** `.ostraka/runs/` is ignored. The audit trail
that has to survive lives in the commit the run produces: it is authored by the
agent that wrote it and carries the run id and both adapters as trailers. A run
directory is working evidence, readable with `ostraka replay` while it is there;
a commit is the permanent record, and it is in history whether or not anyone
kept the directory.

**A run is isolated from the operator, not from the repository.** A repository's
own instruction files travel with the task and reproduce anywhere; the person
who installed the CLI does not. So the shipped profiles suppress user-level
instruction files, settings, hooks, plugins and MCP servers, and keep the
repository's own `AGENTS.md` and `CLAUDE.md`. Where a vendor offers no flag —
its whole per-user directory named by one environment variable — the profile
declares `[isolation]` and the runtime relocates that directory to
`.ostraka/vendor-home/<profile-id>/`, linking in only the entries the profile
names as credentials. Where a vendor offers no way at all, the limit is written
down in `adapters/README.md` rather than papered over. Measured per vendor, both
directions, before shipping.

**Independence is between profiles, not between models.** A profile is already
the unit of "how this agent is invoked" — binary, arguments, permission posture,
isolation, capabilities — and a different model is a different invocation, so a
same-vendor pair is a second five-line file and needs no new concept. Expressing
it at the model level would need the runtime to compare model identifiers, which
rule 2 forbids, and "a different string in a `--model` flag" is a far weaker
property than "a different command line with its own read-only posture". The
commit trailers name adapters; two profile ids keep them true.

What is really being bought is not different weights. It is a different training
lineage, a different system prompt and a different set of blind spots — and two
models from one vendor share all three. So a same-vendor pair is legitimate and
is not equivalent: automatic routing prefers a profile invoking a *different
binary*, and only falls back to a same-binary pair when that is all there is.
Naming one explicitly is still honoured, because naming is a decision.

**Promotion re-asks the gate; it never stores its answer.** A run mints its
token in memory and the token dies with the process, so `ostraka promote` calls
`gate::reaffirm`, which builds a token from the run record against the
project's *current* checks — a check added since the run was made has never
passed and blocks promotion. Serializing a token would make one anybody could
write, which is rule 3 defeated by a file format. Promotion additionally
requires the commit's own trailers to agree with the record, because the record
sits beside the repository and the trailers sit inside history: forging one is
not enough. And it stops at a branch. Merging, pushing and opening a pull
request are acts a person takes.

**The binary crate is `ostraka`.** It was `ostraka-cli`, which would have made
the install command `cargo install ostraka-cli` and left the bare name unclaimed
and squattable — a worse outcome than the tidier directory. The package is the
one people type. `ostraka-cli` should still be claimed on claim day, pointing
nowhere, for the same reason.

**A vendor's environment is built, not inherited.** `env_clear`, then an
operating-system baseline, then the names a profile lists in `inherit_env`, then
its `[env]`, then isolation — later winning, isolation last because it is the
structural guarantee. Inheriting was not a small leak: an adapter profile that
ran `env` showed an authoring agent holding its launcher's session id, IPC
socket and messaging token, which is a channel out of a process whose whole job
is to write to a worktree. No vendor variable appears in `ostraka-adapter`; a
CLI that authenticates by environment variable names that variable in its own
profile, which is rule 1 applied to the environment. Gate checks are *not*
scrubbed: they are the project's commands run on the operator's behalf and need
the operator's toolchain.

**Token counts are quoted, never computed.** Every figure in the TUI's bottom
line is a vendor's own accounting, extracted by a `[usage]` block in that
vendor's profile — no vendor variable, marker or JSON pointer appears in
`ostraka-adapter`, for the same reason no product name does. A vendor that
reports nothing produces no row and is absent from the display, because "does
not say" and "spent nothing" are different claims. A vendor that reports one
combined figure is shown as a total rather than as a split with a fabricated
zero, and one that rounds is marked as an estimate. A field may sum several
counters: reading Claude Code's `input_tokens` alone, without the two cache
counters beside it, reported 16 for a run that actually sent 159,460.

**`init` never generates a gate that passes everything.** It infers checks from
what it finds, and where it cannot tell, it writes one that fails with an
instruction. An empty `[gate]` is refused by `Config::validate`, but a *generated*
one that quietly passed would be worse than a refusal: someone would learn what
their gate did from a change that should not have been approved. Nothing on
disk is overwritten either — a setup command is not how anybody should lose a
file they wrote.

The profiles it writes are embedded, because a binary installed by curl has no
repository beside it. They are copies of `adapters/*.toml`, and
`check-hygiene.sh` fails if the two drift — the same treatment as the gate
written out in both `ci.yml` and `ostraka.toml`.

**Onboarding says what setting up will not fix.** Reviewed by opening the
browser in an empty directory and reading what it promised. It offered to set
the place up and said nothing about the two things that would still be wrong
afterwards, both of which were then discovered from a failed run.

The first is git. Everything downstream stands on `git worktree add`, so a
directory git has never heard of produces a perfectly valid project that cannot
run anything — and said so for the first time in git's own words, from inside a
run, after a vendor had been paid. `worktree::is_repository` is asked once, by
`ostraka check` and by the screen that makes the offer.

The second is the gate. Where `init` cannot tell how a project is verified it
writes a check that fails on purpose, which is the right thing to write and was
invisible: the command printed a note afterwards and the browser printed
nothing at all. Both warnings now sit under the offer, before the key that
takes it, and they stay afterwards because taking the offer does not make them
untrue.

**A problem the browser recognises comes with the steps out of it.** "did not
finish — git worktree add failed: fatal: not a git repository" is a true
sentence and a dead end: it is git's account from two layers down, it arrives
after a run has been started and a vendor paid, and it leaves the operator to
work out both that the answer is `git init` and that `git init` alone is not
enough either, because a worktree needs a commit to branch from.
`tui::remedy` is that knowledge as steps. It is diagnosed from the state of the
directory rather than from the text of an error — matching English out of
another program is how a fix stops working when that program rewords itself —
and it does nothing on its own: one step per keypress, each one shown with the
commands it will run before it runs them, because the second commits whatever
is lying in the operator's directory. A step that fails keeps git's own words,
which for a missing author identity are better than any sentence written here.
A run in a directory that cannot host one is refused before it starts, with the
steps offered and the task still in the box.

**What was asked here last is the first thing you see.** "Nothing has been asked
here yet" is true of the thread and false of the directory, and in a repository
with fifty runs behind it that read as a browser that had lost them. The
opening screen summarises the five most recent — enough to recognise where the
work got to, and not a second list.

**Panes are switched between, not shown side by side.** A pane is a line of
work — a thread, the repository it runs in, and the task being written into it
— and everything else belongs to the workspace and stays where it is. Two
transcripts on an eighty-column terminal are two transcripts nobody can read,
and the width is what this screen spends on the thing being read; which pane you
are in is a row at the top, and it only appears once there is more than one.

**One run at a time across every pane, and that is not a property of panes.**
The request to stop is a single flag because a signal is single, so two runs
going at once would both stop when either was asked to. Running several is the
parallel-execution question and panes do not answer it: what they buy is keeping
several lines of work *open*, not running them together. Every pane is drained
each tick, though — a run keeps going in a pane somebody has switched away from,
and a transcript that stopped updating because nobody was looking at it would be
a transcript that lied. A run that ends in a pane you are not in names that pane
when it says so, and the bar marks it while it goes.

A pane is not closed out from under a run, and the last one is not closed at
all: the first would abandon the thread writing into a worktree, and the second
would leave a browser with nothing to type into.

**A slash in the box offers the commands at the box.** Typing `/settings` used
to start a run whose task was the word "settings". The menu is anchored to the
input rather than centred, because that is where the eye already is; the
centred palette is for going and finding a command, this is for the one you
were halfway through naming. One word behind a slash and nothing else: a task
is a task however it starts.

**Settings show what is this thread's and what is the repository's, and only
change the first.** Author identity, reviewer identity and the model hint belong
to a thread and are typed in place. The gate, the two ceilings and the worktree
base are read from `ostraka.toml` and shown rather than edited — a gate is what
a repository agrees on, and a screen that quietly rewrote it would change what
everybody else's runs are judged by. Read when the settings are opened rather
than held, because that file is one somebody edits.

**A half-finished chord outranks an open dialog.** The leader is armed after a
dialog opens, so the dialog was swallowing the letter meant to complete it and
the leader stayed armed for ever. Chords, then the chord in progress, then what
is open over the screen, then whoever has the keyboard.

**Leaving is asked, not assumed.** Quitting can discard a task that was being
written and can stop a run that is going, and one key should not do both
silently. The dialog says which of the two applies. Already on the way out, it
does not ask again — asking twice would be asking about the answer.

**`init` asks whether anything is left to write; the browser asks whether this
runs.** Two questions, and they are not the same one. `Plan::complete` is the
first and `Plan::runnable` — a config, and at least one profile beside it — is
the second. The browser asked the first for as long as it existed, which put
"This directory is not an Ostraka project yet" across a screen with three
recorded runs behind it, because this repository's own `.gitignore` names four
paths under `.ostraka/` rather than the directory and so `init` correctly still
had a line to add. A missing ignore line is untidy; it is not a directory
nobody has set up. `Planned` carries a `Role` so that distinction is a field
rather than a filename compared against a literal.

The second half of that lesson cost more. `runnable` first asked whether the
plan's *own* profile entries were present — and a plan lists the profiles
`init` would write, so a project that brought its own under other names had
every one of them missing while running perfectly well. The opening screen went
up over a working project, a stray keystroke on it took the offer, and three
vendor profiles nobody had asked for landed in a directory that already had
two. Whether a project has a profile is a question about the directory, so it
is asked of the directory. Found by driving the browser against a scratch
project rather than by reasoning about it.

**The desktop application is a second view, not a second product.** `ostraka-app`
draws with `egui` in a window `eframe` opens: pure Rust, no web runtime, no
second toolchain — which is the same condition that let `ratatui` in, and the
reason Tauri is not the answer here. Everything it shows comes from
`ostraka-runtime`, and its Promote button calls `promote::promote` exactly as
the command line does, so a click cannot approve what the gate refuses. State
lives in `state.rs` with no `egui` in it and is tested without a display; the
drawing reads that state.

It costs: a 9.7 MB binary against the command line's 2.0 MB, and the workspace
gate now compiles a window stack. Both were measured before being accepted. The
release builds it for four targets rather than five — a GUI cross-compiled to
aarch64 Linux needs the arm64 window and GL libraries in a sysroot, which is not
a flag, and the command line still ships there.

**The TUI is a view, and it earns two dependencies.** `ratatui` and `crossterm`
go in the `ostraka` crate only, with `default-features = false` — the defaults pull a
second backend and a colour stack that cost 67 crates including wasm bindings,
for nothing. Both are pure Rust with no runtime of their own, which is the
condition any dependency here has to meet: the distribution advantage is a
binary you can curl, and that survives a library but not a runtime. Measured:
1.61 MB to 1.97 MB.

Drawing is a pure function of state, so the screen is asserted against a
rendered buffer rather than looked at. The browser holds no logic: the listing
is `runtime::index`, the detail is `orchestrator::replay`, and promoting goes
through `promote::promote` like every other caller — pressing a key cannot
approve anything.

**It shows a run in progress, and that did not need async.** This reverses a
decision recorded here — "it shows finished runs only; a live view needs the
orchestrator to stream while something else renders, which is the
parallel-execution problem and waits for the same answer" — and the reason it
gave turned out to be wrong. Rendering *one* run while it happens needs a
thread and two channels, both `std`. Several runs at once still needs the
answer that paragraph was waiting for, and is still not offered: the command
list refuses a second run while one is going, by every route including the bare
key.

The mechanism is `runtime::progress`. A `Watcher` is handed a `Step` and
returns nothing, which is the whole guarantee: watching cannot become steering,
and a screen that has gone away cannot stall a run — the channel watcher drops
what it cannot deliver. `RunLog` carries it, because every `append` was already
the sentence "something happened" and a second parameter threaded through six
functions to say it twice is how the two drift apart. `gate::run_checks`
reports each check as it finishes, because the gate is the longest part of a run
— twelve seconds on this repository — and a caller that learns the outcome only
at the end has nothing to show for that time.

`run::execute` is the one path. `ostraka run` calls it and so does the browser,
so a run started by typing a task into a box is the run a shell starts: same
routing, same gate, same record. Two paths would be two pipelines inside one
release. The browser starting a run does not let it approve one — the gate is
where that is decided, and it is on the far side of `execute` either way.

Quitting waits. A browser that exited while a vendor was still writing into a
worktree would undo the thing Ctrl-C was taught to prevent, so `q` during a run
asks it to stop and stays up until it has.

**The flows are tested as journeys, not as frames.** `tui::flows` drives the
real key handler and the real drawing from the same `open` the command uses,
and the run flows drive the real orchestrator with shell scripts for vendors —
the same point `milestone_one` makes one crate down. A task is typed, a run
happens, and the assertion is what ends up on the screen and in
`.ostraka/runs/`. They caught two things no single-purpose test would have: a
second run started in the same second as the first overwrote its record,
because the task id had been moved to a clock that counts in seconds; and a
stop asked for in the same breath as a run was wiped by the run clearing the
interrupt flag behind it, which is why the flag is now cleared before the
thread starts rather than inside it.

What they stop at is the terminal. A pty is not something `std` can open and
`script(1)` takes different arguments on every platform this releases for, so
the boundary is `handle` and `draw`. The one thing on the far side — that a
browser with no terminal says so rather than panicking inside a dependency — is
asserted directly, because a test's stdout is not a terminal and that is the
real path.

**A thread is the unit of work; a run is the unit of review.** One run off
`HEAD` is right for judging a change and wrong for doing a piece of work,
because the second task in any real piece of work starts by reading what the
first one wrote and off `HEAD` it cannot see it. `tui::thread::Thread` chains
them: each run branches from the run before it, which is one argument —
`base_ref` — and nothing else. Every run in the chain still passes the gate and
an independent review on its own; the chain buys continuity, not leniency.

**The chain advances only through the gate.** After a refusal the next task
starts where the refused one did. Continuing from a change the gate would not
take is a way of taking it, and it would take it silently, which is worse than
taking it.

**The box has the keys, and the list is a dialog.** The screen someone opens
this for is the one where they say what they want, so typing types and every
command is a chord, a leader letter or a palette entry. A permanent list of
runs costs half the width of the terminal to show something read once in a
while; it is `Dialog::Runs`, and opening a run gives it a screen of its own.
`Command::offered` is the authority for the bare keys as well as for the two
routes that read it to draw themselves — without that, `n` during a run opened
a box the palette had already refused, and enter would have replaced the live
session and walked away from the thread behind it.

**Regions are divided by rules, and colour means something.** Space alone was
not enough: four regions separated by gaps read as one region with holes in it,
and the boundary has to be re-derived every time the eye lands. Colour is
semantic rather than decorative — the runtime muted, the author the accent, the
gate the colour of something under test, the reviewer its own — so a transcript
can be scanned for "what did the reviewer say" without being read. Still
sixteen ANSI colours and no palette of our own.

**Choosing the pair is offered; being allowed to is still the gate's.**
`Dialog::Agents` sets the author and reviewer profiles for a thread, because
routing is usually right and occasionally not, and naming one is a decision.
The runtime is unchanged: it still refuses a reviewer that is the author, and
`--adapter` and `--review-adapter` were already the arguments. What the browser
adds is somewhere to see which profiles are on this machine and which of them
answer.

**One box, sixteen colours, and one list of commands.** The screen is separated
by space and a one-column gutter rather than by borders — a browser that boxes
every region spends a quarter of an eighty-column terminal drawing lines around
nothing, and once the boxes are gone the one that remains is unmistakably where
typing goes. Colour is the sixteen ANSI base colours and nothing else: a
true-colour palette would look identical on every machine, which sounds like the
point and is not, because it would look identical *and* wrong beside every other
window on that desktop, and unreadable over ssh to an eight-colour terminal. The
two outcome ticks are the one knowing exception — they are East Asian Ambiguous,
so a terminal configured for CJK width shifts that column by a cell, accepted
because they are what every other tool in the same terminal already uses.

The three panes are named in a row above the pane rather than by a footer naming
the next one: the row costs the same width and says what there is instead of what
to press to find out. Below seventy-two columns the detail moves behind Enter,
because two columns neither of which can be read is worse than one that can.

`tui::command::Command` is the list, and the bare key, the `ctrl-x` leader and
the `ctrl-k` palette all dispatch through one `perform` — three ways to reach
seven actions, not three implementations that can drift into three slightly
different versions of promote. A command added to that list appears in all three
and in the keys dialog, which is the only way a screen with several routes to
the same action stays honest about what it can do. Keys are read in the order a
keystroke has to be read — an open dialog first, then what is being typed into,
then a half-finished chord, then the browser — because any other order lets `q`
close the browser out from under someone reading the help.

**Nothing waits forever.** `policy.timeout_secs` spent this project's life
declared, documented as a wall-clock ceiling, and read by nothing — which is
worse than absent, because someone sets it and believes their fleet is bounded.
It is enforced now, and `[gate] timeout_secs` bounds the project's own checks
separately: a test suite is allowed to take longer than an agent is. Unset still
means wait, so no existing project changes behaviour; `init` writes both so the
default is visible rather than buried.

A stopped vendor is its own outcome, never a verdict on the change. `TimedOut`
and `Interrupted` are distinct from `AuthorFailed`, because "the clock ran out"
and "the operator changed their mind" are not "the agent could not do it".

**An author that did not exit cleanly did not finish, and half a change is not
reviewed.** This was a hole, and it was found by testing what a token limit
actually looks like rather than by reasoning about it. A vendor whose context
window fills mid-task exits non-zero *with part of the change already on disk* —
so the worktree was not empty, the `AuthorFailed` branch did not apply, and the
half-written change went to the gate, passed it, was reviewed and was approved.
A merge token for half of what was asked.

The principle was already written down one paragraph up, for the killed author:
what is on disk is half of whatever it was doing, and half a change is not a
change anybody should be asked to review. A context window running out is a
stop like any other; only who stopped it differs. A non-zero exit is now a
refusal whether or not the worktree is empty.

The cost is named rather than glossed: a vendor that exits non-zero for a
harmless reason now has finished work refused rather than reviewed. That is the
safe direction and it is recoverable — a refused run keeps its worktree, the
record carries the vendor's own words, and running it again is one command. The
unsafe direction was not recoverable, because nobody was told.

Five shapes are pinned down in `milestone_one`, and one journey through the
browser in `tui::flows`: an author out of context halfway, a reviewer out of
context, a reviewer cut off mid-sentence before its verdict line, a reviewer
that answered twice because it retried after a limit, and the count a run spent
being recorded on the way out — which is the run somebody most wants the number
for. The browser shows the vendor's sentence on the rule that closes the turn;
it used to drop that label for being too long, which put a bare rule exactly
where the reason should have been.

Two things only a real terminal revealed. A shell killed at the ceiling leaves
its children alive, and they hold the pipe open — so waiting for EOF after a
kill waits on precisely the process the ceiling gave up on; output is collected
line by line and abandoned after a short grace instead. And Ctrl-C reaches the
whole foreground process group, so the child usually dies of the same signal
before the poll notices, which made an interrupted run report as an agent
failure until `finish` learned to ask whether a stop had been requested at all.

**A ceiling that leaves the expensive half running has not bounded anything.**
Abandoning the pipe kept the run moving, but the children went on running: a
vendor's helper, a test runner's workers, a build's dev server, still holding
the CPU, the ports and the GPU of a run that was over. Vendors and checks are
launched into a process group of their own now, and the stop signals the group
rather than the leader. That also detaches them from the terminal's foreground
group, which is the right way round — the stop is deliberate and takes
everything, instead of being whatever Ctrl-C happened to reach.

The signal goes through `libc::killpg`, the one dependency `ostraka-adapter`
and `ostraka-runtime` add beyond serde and thiserror, unix-only and bindings
with no runtime of their own. It earns the line by being the version that
works. Shelling out to `kill -TERM -1234` was written first and is silently
wrong: procps reads `-1234` as the pid 1234, signals the leader alone, and
exits 0 — so the descendants stay up and the launcher is told it worked. The
two `kill` binaries in circulation disagree about the leading `-`, and a
teardown path is the worst place to depend on which one is installed. Both
crates keep a regression test that lets a shell fork something outliving it and
fails if the survivor writes its marker.

**A worktree is prepared before the agent, not just before the gate.** A fresh
checkout has none of what git ignores — `node_modules/`, `.venv/`, `vendor/` —
so every check that shells out to the toolchain fails for a reason that has
nothing to do with the change, and the agent cannot run those tools either.
`[worktree] link` symlinks them in from the project, absolute so the link does
not depend on how deep `base` puts the checkout; `[worktree] setup` runs a
command for what linking cannot express. Linked rather than installed per
worktree: `npm ci` in each one costs hundreds of megabytes, and a run should not
be why a disk fills.

A failure here is `SetupFailed`, never a failed check. "The environment was not
ready" and "the change was rejected" are different answers, and reporting the
first as the second is what made a missing `node_modules` read as a refused
change — reported from a real Next.js onboarding, issue #1.

`init` writes `link = ["node_modules"]` for a Node project, because detecting
the ecosystem and then handing over a gate that cannot run in the environment
Ostraka itself builds is worse than not detecting it.

**Worktrees are released on success only.** The commit is on the run's branch,
and the diff pane, replay and promotion all read it from there, so the checkout
is redundant once a run is approved — and a directory per run is how a busy
repository fills a disk with copies of itself. A refused run keeps its worktree:
that is the evidence someone needs, and deleting it would take it away exactly
when it matters. `ostraka prune` clears what is left, reports before it acts,
and never touches a branch.

**Parallel execution arrived, and async still has not earned it.** `ostraka
drain --workers N` takes the task list down N at a time, and each worker is the
sequential run that already existed on a thread of its own. This is the second
time the answer to "that waits for async" turned out to be a thread — the live
transcript was the first. `tokio` buys nothing for work that is subprocesses
rather than sockets and that numbers what somebody asked for rather than
thousands. If it ever goes in it goes in `ostraka-runtime` only; `ostraka-core`
stays inert either way, which is rule 4 above and is not negotiable.

**What the queue made possible was the claim, not the concurrency.** A run has
always been able to wait on a subprocess while another thread watched it. What
did not exist was somewhere to take the second task from, and a claim two
workers cannot both win — `tasks::claim` renames `pending/<id>` to
`running/<id>`, which is atomic everywhere this ships and tells the loser
`NotFound`. No lock file, no crate.

**One Ctrl-C stops everything, and stopping one run of several is still open.**
The interrupt flag is global on purpose, and for a drain that is the right
answer. The browser's `s` key is the case that needs a per-run handle, and it is
still the reason the browser refuses a second run while one is going.

## Release artifacts are a contract

`ostraka-<tag>-<target>.tar.gz` plus a `.sha256` sidecar. The install script,
the Homebrew formula and the npm shim all parse that name. Changing it breaks installs silently,
so it changes only deliberately — and `check-hygiene.sh` now fails when the
three files that construct or parse it stop agreeing, including when a platform
is added to the release matrix and not to the two things that install it.

**A fifth crate is a fifth name.** `ostraka-app` joins `ostraka`, `ostraka-core`,
`ostraka-adapter` and `ostraka-runtime` on claim day. Verified free on the
registry the day it was added, along with `ostraka-desktop`, which was the
alternative.

**Publish the workspace, not the crates.** `cargo publish -p ostraka-adapter`
fails before the first release with `no matching package named ostraka-core
found`: the path dependencies carry versions, and cargo resolves them against
the registry. `cargo publish --workspace` orders them and verifies each against
the previous one locally. Dry-run clean for all four.

**Dry-run twice at the same version and the second one lies.** Verification
compiles each packaged crate against the previously packaged ones, and those are
cached by name and version. Change the source without changing the version — the
normal state while preparing a release — and cargo happily reuses the copy it
built last time. It cost most of an afternoon presenting as three compiler
errors about a field that does exist, which reads as a code fault and is not
one. Run it with a throwaway target directory:

```
CARGO_TARGET_DIR=$(mktemp -d) cargo publish --dry-run --workspace
```

A real release bumps the version and never sees this. Every rehearsal does, and
the dangerous direction is not the failure — it is a dry-run that *passes*
against stale artifacts.

**The formula is bumped after the build, and lands by review.** Homebrew needs a
checksum per platform and those exist only once the artifacts do, so
`scripts/bump-formula.sh` runs in a second release job — which opens a pull
request rather than pushing. A workflow committing to the default branch is an
unreviewed merge, and principle 2 does not exempt a release for being automated.
The tag is published either way; the formula lands when a person merges it, and
`brew` serves the previous version until then. The script refuses rather than
guesses: a missing sidecar or a surviving `0000…` placeholder fails the job
instead of publishing a formula that installs nothing.

**The npm package carries no binary.** `npm/` names a version, downloads the
release artifact for the running platform, verifies the checksum published
beside it, and hands off. The launcher is deliberately thin — anything it did
would be the second runtime this project refuses. Publishing it is `npm publish`
from `npm/`, by hand on claim day: there is no publish job, because one would
need a token nobody has created and would fire on a tag before anyone had
decided to release. `check-hygiene.sh` keeps its version equal to the crates'
and its platform table equal to the release matrix; both drifts were confirmed
to fail it.

**`scripts/install.sh` takes `OSTRAKA_BASE_URL`.** Without it the script could
only ever be tested by cutting a real release and watching what happened to
other people. With a `file://` directory of locally packaged artifacts it runs
end to end, checksum verification included.
