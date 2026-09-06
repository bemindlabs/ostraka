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

**Async stays out until parallel execution earns it.** Everything today is
sequential and synchronous. When several adapters need to stream at once, tokio
goes in `ostraka-runtime` only — `ostraka-core` stays inert either way, which is
rule 4 above and is not negotiable.

## Release artifacts are a contract

`ostraka-<tag>-<target>.tar.gz` plus a `.sha256` sidecar. The install script and
the Homebrew formula both parse that name. Changing it breaks installs silently,
so it changes only deliberately.
