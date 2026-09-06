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

## Layout

```
crates/ostraka-core      domain types; no IO, no process spawning
crates/ostraka-adapter   vendor boundary: profiles, launch, event normalization
crates/ostraka-runtime   engine: worktrees, gate, review, run records
crates/ostraka-cli       the `ostraka` binary
adapters/                one TOML profile per vendor CLI — data, not code
scripts/install.sh       the curl one-liner; downloads a released binary
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

**The TUI is a view, and it earns two dependencies.** `ratatui` and `crossterm`
go in `ostraka-cli` only, with `default-features = false` — the defaults pull a
second backend and a colour stack that cost 67 crates including wasm bindings,
for nothing. Both are pure Rust with no runtime of their own, which is the
condition any dependency here has to meet: the distribution advantage is a
binary you can curl, and that survives a library but not a runtime. Measured:
1.61 MB to 1.97 MB.

Drawing is a pure function of state, so the screen is asserted against a
rendered buffer rather than looked at. The browser holds no logic: the listing
is `runtime::index`, the detail is `orchestrator::replay`, and promoting goes
through `promote::promote` like every other caller — pressing a key cannot
approve anything. It shows finished runs only; a live view of a run in progress
needs the orchestrator to stream while something else renders, which is the
parallel-execution problem and waits for the same answer.

**Async stays out until parallel execution earns it.** Everything today is
sequential and synchronous. When several adapters need to stream at once, tokio
goes in `ostraka-runtime` only — `ostraka-core` stays inert either way, which is
rule 4 above and is not negotiable.

## Release artifacts are a contract

`ostraka-<tag>-<target>.tar.gz` plus a `.sha256` sidecar. The install script and
the Homebrew formula both parse that name. Changing it breaks installs silently,
so it changes only deliberately — and `check-hygiene.sh` now fails when the
three files that construct or parse it stop agreeing, including when a platform
is added to the release matrix and not to the two things that install it.

**Publish the workspace, not the crates.** `cargo publish -p ostraka-adapter`
fails before the first release with `no matching package named ostraka-core
found`: the path dependencies carry versions, and cargo resolves them against
the registry. `cargo publish --workspace` orders them and verifies each against
the previous one locally. Dry-run clean for all four.

**The formula is bumped after the build, not before the tag.** Homebrew needs a
checksum per platform and those exist only once the artifacts do, so
`scripts/bump-formula.sh` runs in a second release job and commits the result to
the default branch. It refuses rather than guesses: a missing sidecar or a
surviving `0000…` placeholder fails the job instead of publishing a formula that
installs nothing.

**`scripts/install.sh` takes `OSTRAKA_BASE_URL`.** Without it the script could
only ever be tested by cutting a real release and watching what happened to
other people. With a `file://` directory of locally packaged artifacts it runs
end to end, checksum verification included.
