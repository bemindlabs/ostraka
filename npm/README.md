# ostraka

**Run agent fleets you can actually review.**

Ostraka runs coding agents from different vendors against one task, in isolated git worktrees,
and refuses to hand back a mergeable result until the project's own checks have run and an agent
that did not write the change has approved it.

```console
npm install -g ostraka
ostraka init       # write the workspace layout. Overwrites nothing
ostraka run "..."  # isolate, execute, gate, review, record
```

## This package carries no code

It names a version, downloads the release binary for your platform from GitHub, verifies the
checksum published beside it, and hands off. The binary is a single static Rust executable with
no runtime of its own.

That is deliberate. The distribution advantage this project has is a binary you can `curl`, and
anything this launcher did for itself would be the second runtime Ostraka refuses to have. npm
is a channel, not an exception.

Platforms: macOS and Linux on x86-64 and arm64, and Windows on x86-64. On anything else the
install fails with instructions rather than half-working.

## Why

When a run finishes, the only account of what happened was written by the same process that did
it. The agent says the tests pass; it says it touched three files. Both are output, not
evidence.

Ostraka takes that account away from the agent: the checks execute as real subprocesses in the
worktree, what changed is read from `git status` rather than from the agent's summary, review
goes to a different profile than the one that wrote the change, and every run leaves a record
that `ostraka replay` reads back a week later.

Nothing merges by itself. An approved run stops at a commit inside its worktree, and
`ostraka promote` gives it a branch.

## Other ways to install

```console
curl -fsSL https://ostraka.sh/install | sh     # a binary, no toolchain
cargo install ostraka                           # builds from source

brew tap bemindlabs/ostraka https://github.com/bemindlabs/ostraka
brew install ostraka
```

Every path resolves the same artifact.

## Documentation

[github.com/bemindlabs/ostraka](https://github.com/bemindlabs/ostraka)

MIT
