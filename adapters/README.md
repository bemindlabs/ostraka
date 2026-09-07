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

# Variables this vendor needs from the launching environment. A vendor process
# is started with a built environment, not an inherited one: an operating-system
# baseline, these names, whatever `[env]` sets, and whatever isolation sets.
# Anything else is dropped. A CLI that authenticates by environment variable
# names that variable here, because the runtime cannot know which one is whose.
inherit_env = ["EXAMPLE_API_KEY"]

# Appended only when a run names a model, so an optional flag never becomes
# `--model ""` — which is a different request from "use your default".
model_args = ["--model", "{{model}}"]

# How to ask whether this CLI is usable here. Defaults to ["--version"].
# Widen it when a CLI can be installed and answer --version while being
# unauthenticated: routing skips a profile that is not ready, and it can only
# skip what it can detect.
probe_args = ["--version"]

event_format = "none"   # "none", "jsonl", or "json" (one document)
event_text = "/result"  # required with "json": where the reply is in it

# Where this vendor reports what a run cost, if it reports it at all. Nothing
# here is estimated: a vendor that says nothing produces no row, which is what
# lets a status line show a dash instead of a zero.
#
#   stream      "stdout" | "stderr"
#   shape       "text" (find a marker) | "json" (RFC 6901 pointers)
#   number      "after" | "before" — which side of the marker the count is on
#   approximate this vendor rounds before reporting, so treat it as an estimate
#
# A field may name several counters, which are summed: a vendor that splits
# input into fresh, cache-creating and cache-read tokens is reporting three
# parts of one number, and taking the first alone understated a real run by
# four orders of magnitude.
[usage]
stream = "stdout"
shape = "json"
input = ["/usage/input_tokens", "/usage/cache_read_input_tokens"]
output = "/usage/output_tokens"

[env]
EXAMPLE_NO_COLOR = "1"

# Only for the part of isolation no flag can reach: a CLI whose entire per-user
# directory is named by an environment variable. See below.
[isolation]
home_env = "EXAMPLE_HOME"
home_source = ".example"
credentials = ["auth.json"]

[capabilities]
headless = true
streams_json = false
resumable = false
```

Placeholders substituted into every argument list: `{{prompt}}`, `{{model}}`,
`{{worktree}}`. Substitution is one pass and non-recursive, so task text cannot
expand into further placeholders.

## One vendor, two models

Independence is expressed between profiles, so a reviewer that is the same CLI
on a different model is a second file. Pin the model in `args` rather than in
`model_args`, which is only appended when a *run* names one:

```toml
# adapters/claude-code-sonnet.toml
id = "claude-code-sonnet"
command = "claude"
args = ["-p", "{{prompt}}", "--model", "sonnet", "--permission-mode", "plan",
        "--setting-sources", "project", "--strict-mcp-config"]
```

```
$ ostraka run "..." --adapter claude-code-opus --review-adapter claude-code-sonnet
approved — written by author, reviewed by reviewer

    Authored-by: author (claude-code-opus)
    Reviewed-by: reviewer (claude-code-sonnet)
```

It is a legitimate pair and it is not equivalent to a cross-vendor one: two
models from one vendor share a training lineage, a system prompt and a set of
blind spots. So a run that picks for you prefers a profile invoking a different
binary, and falls back to a same-binary pair only when that is all there is.
Naming one is still honoured — naming is a decision.

## What a run cost

`ostraka tui` totals tokens per backend along the bottom, from what each vendor
reported about itself. The three shipped CLIs report in three shapes, on two
streams, and one rounds — which is why extraction is profile data rather than
code, and why the display says which is which:

```
tokens   claude-code 159.5k in / 4.3k out · codex 14.7k total · copilot-cli ~10.6k in / ~296 out
```

Three claims, each made the way its vendor made it. `codex` reports one combined
figure, so it is shown as a total rather than as a split with a zero nobody
claimed. `copilot-cli` rounds before it reports, so its figures carry a tilde. A
backend that reports nothing is absent rather than shown as zero — "does not
say" and "spent nothing" are different, and only one of them is true.

## Isolation — keeping the operator out of the run

A coding CLI reads a great deal from whoever installed it: instruction files,
MCP servers, hooks, plugins, a default model. None of that travels with the
repository, so a fleet runner that inherits it produces results that depend on
whose machine it is on. One run record in this project shows an authoring agent
replying in Thai inside an English-only repository, because it had adopted a
persona from the operator's personal instruction file.

The line worth drawing is not "read nothing". It is **the repository travels
with the task, the operator does not.** A repository's own `AGENTS.md` is part
of the input and reproduces anywhere; `~/.claude/CLAUDE.md` is not and does not.

Most of it is flags, and flags belong in `args` and `review_args` where they are
visible in the command line the run record shows. Only one shape needs the
`[isolation]` table: a CLI that offers no flag at all and keeps everything
per-user in a single directory named by an environment variable. Ostraka creates
`.ostraka/vendor-home/<profile-id>/` and points the variable at it.

Relocating that directory relocates the credentials inside it, which is why
`credentials` exists: those entries — and nothing else — are linked into the new
directory, so the CLI can still authenticate. Linked, never copied: isolation
must not leave a second copy of a secret on disk in order to achieve itself.
The directory is per profile rather than per run, because a vendor that
refreshes its own token needs somewhere to keep it.

`[isolation]` is a promise, so a profile that declares it will not launch at all
if the runtime offered nowhere to put the directory.

### What was measured

A scratch repository containing `AGENTS.md` with a codeword and `CLAUDE.md`
importing it, run outside any workspace, with a canary present in the operator's
user-level files and nowhere else. Claude Code was asked with `--tools ""` so
that the answer could only come from injected context rather than from the model
reading a file.

| | operator config reaches the model | repository instructions reach it | what changes that |
|---|---|---|---|
| `claude` | yes | yes | `--setting-sources project` |
| `codex` | yes — `$CODEX_HOME/AGENTS.md` is spliced into the same block as the project doc | yes | `CODEX_HOME` only |
| `copilot` | no user-level instructions path exists | yes | `--no-custom-instructions`, which also drops the repository's |

Instruction files turned out to be the smaller half. A baseline `claude -p` in an
empty scratch repository had 108 tools, 81 of them MCP servers belonging to the
operator — mail, calendar, drive, an automation server. An authoring agent could
have sent mail from the operator's account. `--strict-mcp-config`, with no
`--mcp-config` to go with it, leaves none. `--setting-sources project` alone does
not: the two flags are orthogonal and both are needed.

### Two routes, and why the flags are shipped

Claude Code can be isolated either way, and both were measured. The flags above
leave the operator's own installation untouched. Relocating `CLAUDE_CONFIG_DIR`
with `.credentials.json` carried is *stronger* — it drops every MCP server with
no flag at all, because the servers are configured in the directory that moved:

```toml
[isolation]
home_env = "CLAUDE_CONFIG_DIR"
home_source = ".claude"
credentials = [".credentials.json"]
```

The flags are shipped anyway. An isolation that needs no credential handling is
worth more than a marginally stronger one that does: linking a credential into a
directory is a thing that can go wrong, and it only has to be done for a vendor
that leaves no alternative. Use the table above when you want one mechanism
across every vendor.

### Limits, stated rather than implied

- **Copilot CLI cannot separate the two.** `--no-custom-instructions` drops the
  operator's instruction files and the repository's `AGENTS.md` together, so it
  is not shipped: losing the repository's own rules costs more than it buys on a
  machine where no user-level instructions file exists at all. Its config home
  (`XDG_CONFIG_HOME`) can be relocated, but `config.json` is where its login
  lives as well as its default model, so relocating it logs the CLI out — the
  way back in is `GH_TOKEN` in the environment, which is the operator's call and
  not something a profile should make for them.
- **Claude Code's wider hammers cost more than they save.** `--safe-mode` also
  drops the repository's `CLAUDE.md`; `--bare` additionally forces
  `ANTHROPIC_API_KEY` and never reads OAuth, so a subscription login stops
  working. Both were measured, neither is shipped.
- **`--strict-mcp-config` also drops a repository's own `.mcp.json`.** That is
  repository content and arguably reproducible, but there is no way today to
  pass it through, and an MCP server reaches outside the worktree. No servers is
  the safe end of that trade.
- **The environment is built, not inherited — and the baseline is a judgement.**
  A vendor gets `HOME`, `PATH`, `SHELL`, `TMPDIR`, the user and terminal names,
  the locale and timezone, the proxy variables, and the handful Windows cannot
  start a process without. Everything else is dropped unless a profile names it
  in `inherit_env`. That baseline is generic on purpose and it is still a
  choice: proxy settings differ between machines, so a run reaches the network
  by a path that is not reproducible even though its behaviour is.
- **The gate's checks are not scrubbed.** `cargo`, `pytest` and the rest are the
  *project's* commands run on the operator's behalf, not an agent's, and they
  need the toolchain environment the operator has. Only adapter launches get a
  built environment.
- **The default model is operator state.** Two machines with different vendor
  configuration run the same task on different models. Isolation removes the
  vendor's *configured* default — a relocated home falls back to the CLI's
  built-in one, which changes between releases. A run that has to be reproducible
  names its model.
- **Codex cannot author where its own sandbox cannot start.** On a machine
  without the user-namespace permissions bubblewrap needs — a container, a
  locked-down CI runner — `--sandbox workspace-write` fails before any file
  operation with `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`,
  and codex **exits zero having changed nothing**. A run therefore refuses with
  "the author ran cleanly and changed nothing", which is the truthful reading of
  exit zero and an empty worktree; codex's own explanation is in the run record.
  Reproduced with the operator's untouched configuration, so it is not something
  isolation does. Reviewing is unaffected — `--sandbox read-only` runs, and
  codex is verified in that role under a relocated home.
- **A relocated home accumulates.** Sessions, caches and a vendor's own memory
  store live there across runs. It is isolation from the operator, not a fresh
  sandbox each time.
