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

## Release artifacts are a contract

`ostraka-<tag>-<target>.tar.gz` plus a `.sha256` sidecar. The install script and
the Homebrew formula both parse that name. Changing it breaks installs silently,
so it changes only deliberately.
