# Adapter profiles

One TOML file per vendor CLI. Profiles are data, not code: adding a vendor is a
file, not a release, and no vendor name appears in the runtime.

Every `*.toml` in this directory is loaded. `ostraka adapters` reports which of
them can actually run on this machine; `ostraka check` validates all of them.

## Shipped

| Profile | CLI | Verified headless |
|---|---|---|
| `claude-code` | `claude` | yes |
| `codex` | `codex` | yes |
| `copilot-cli` | `copilot` | yes |

Nothing is shipped that has not been run end to end. A profile for a CLI you have
is a five-line file — write it rather than waiting for one.

## The schema

```toml
id = "example"
command = "example-cli"

# How the agent is invoked when it writes a change. Must carry {{prompt}}.
args = ["--print", "{{prompt}}"]

# How it is invoked when it reviews someone else's change. Optional; absent
# means `args` is reused. Declare it whenever `args` grants write or tool
# permission: a reviewer that can edit the worktree can make a change it just
# rejected pass on the next attempt. The reviewer is handed the diff in its
# prompt and needs no tools at all.
review_args = ["--print", "--read-only", "{{prompt}}"]

# Appended only when a run names a model, so an optional flag never becomes
# `--model ""` — which is a different request from "use your default".
model_args = ["--model", "{{model}}"]

# How to ask whether this CLI is usable here. Defaults to ["--version"].
# Widen it when a CLI can be installed and answer --version while being
# unauthenticated: routing skips a profile that is not ready, and it can only
# skip what it can detect.
probe_args = ["--version"]

event_format = "none"   # or "jsonl"

[env]
EXAMPLE_NO_COLOR = "1"

[capabilities]
headless = true
streams_json = false
resumable = false
```

Placeholders substituted into every argument list: `{{prompt}}`, `{{model}}`,
`{{worktree}}`. Substitution is one pass and non-recursive, so task text cannot
expand into further placeholders.

## What a profile has to guarantee

The unit of truth is the git diff the run leaves in the worktree — not the
vendor's transcript. An adapter only has to run the prompt to completion in the
given directory, exit with a status, and leave its work on disk.

Two properties decide whether a CLI can be a *reviewer*, and they are worth
checking before writing the file:

1. **stdout carries the answer and little else.** The verdict is read from the
   first line of what the adapter says. A CLI that prints a banner or progress to
   stdout needs those routed to stderr, silenced by a flag, or `event_format =
   "jsonl"` with a shape the normalizer can read.
2. **It exits non-zero when it fails.** A reviewer that exits non-zero is treated
   as a rejection, which is the safe direction; one that exits zero having said
   nothing usable is also a rejection.

## Routing

With no `--adapter` given, the lowest-id profile that is *ready on this machine*
authors, and the next one reviews. Naming a profile explicitly overrides that and
is honoured even if the CLI is missing — a named adapter that cannot run should
fail by name rather than be quietly swapped for another.

A single profile is refused outright: one adapter has no independent reviewer,
and that should fail before any work is done rather than after.
