# How Ostraka works

Five diagrams of what the code on `main` does, checked against it rather than
against a plan. They are Mermaid, so GitHub renders them in place.

- [The big picture](#the-big-picture)
- [One run, step by step](#one-run-step-by-step)
- [From task to merged change](#from-task-to-merged-change)
- [Several runs at once](#several-runs-at-once)
- [The crates](#the-crates)

## The big picture

A workspace holds the configuration and everything Ostraka writes. The
repositories it works on are left the way their owners left them: nothing is
written into a repository, and each run works in a worktree of its own.

```mermaid
flowchart LR
    Human(["Operator"])

    subgraph Entry["Ways in"]
        direction TB
        CLI["ostraka run, drain, bench"]
        TUI["ostraka tui"]
        App["ostraka-app, a desktop window"]
        Queue["ostraka task add"]
    end

    Exec["run::execute, one path for every run"]
    Consult["consult, for ask and plan"]

    subgraph WS["Workspace"]
        direction TB
        Cfg[".ostraka/ostraka.toml and .ostraka/adapters"]
        Tasks[".ostraka/tasks"]
        WT["one worktree per run, under the worktree base"]
        Rec[".ostraka/runs, one record per run"]
        Cons[".ostraka/consulted"]
        Notes["notes"]
    end

    Repo[("repositories/NAME, whose own ostraka.toml gate wins")]

    subgraph Vendors["Agent CLIs, one TOML profile each"]
        direction TB
        V1["claude-code, codex, copilot-cli"]
        V2["grok, kimi-cli, kimi-code, agy"]
        V3["opencode-ollama, opencode-litellm, opencode-openrouter"]
    end

    Human --> CLI
    Human --> TUI
    Human --> App
    Human --> Queue
    Queue --> Tasks
    Tasks -- "run --next, drain" --> CLI
    CLI --> Exec
    TUI --> Exec
    CLI --> Consult
    TUI --> Consult
    Exec -- "reads" --> Cfg
    Exec -- "git worktree add" --> Repo
    Repo --> WT
    Exec -- "author profile, may write" --> Vendors
    Exec -- "reviewer profile, read-only" --> Vendors
    Consult -- "read-only invocation" --> Vendors
    Vendors -- "work in" --> WT
    Exec --> Rec
    Consult --> Cons
    App -- "reads runs, promotes" --> Rec
    Notes -. "linked into every worktree" .-> WT
```

`ostraka tui` starts a run through the same `run::execute` that `ostraka run`
calls, so a run begun from the browser is the run a shell would begin: the same
routing, gate and record. `ostraka-app` is a second view of the same records.
It lists them, shows a run's checks, events and diff, and promotes an approved
run through the same gate the command line uses. It never starts a run.

Worktrees go under `[worktree] base` in `ostraka.toml`. When that key is absent
it is `.ostraka/worktrees`. The config `ostraka init` writes sets it to
`worktrees`, which puts them in `worktrees/` at the workspace root.

## One run, step by step

The orchestrator drives every step and can approve none of them. Only the gate
can mint a `MergeToken`, and a run ends at a commit on its own branch. Nothing
is merged.

```mermaid
sequenceDiagram
    autonumber
    participant O as Orchestrator
    participant G as git
    participant A as Author profile
    participant C as Gate checks
    participant R as Reviewer profile
    participant K as Gate
    participant L as Run record

    Note over O,G: 1. Isolate
    O->>G: worktree add -b ostraka/RUN_ID
    Note over O,G: 2. Prepare the checkout
    O->>G: link what the config lists and the notes, run the setup command
    opt the checkout cannot be prepared
        O->>L: failed, before any agent runs
    end
    Note over O,A: 3. Execute
    O->>A: the task, in an environment built for it
    A-->>L: events, as they arrive
    opt the author exits non-zero, times out or is stopped
        O->>L: rejected, the half-written change is never reviewed
    end
    O->>G: what was touched, read from git and not from the agent
    opt nothing was touched, or a path is outside the policy
        O->>L: rejected, before any check runs
    end
    Note over O,C: 4. Gate checks
    O->>C: the project's own commands, really run
    C-->>L: output captured
    opt a required check fails
        O->>L: rejected, and the reviewer is never asked
    end
    Note over O,G: 5. Freeze
    O->>G: stage, name the tree, apply the policy again
    Note over O,R: 6. Review by a different profile
    O->>R: the frozen diff, through the read-only invocation
    R-->>O: APPROVE or REJECT, with a reason
    O->>G: stage again and compare the tree
    opt the tree changed during review
        O->>L: rejected, nothing is committed
    end
    Note over O,K: 7. The gate decides
    O->>K: checks passed, the reviewer is not the author, approved
    alt all three hold
        K-->>O: MergeToken
        O->>G: commit with Run, Authored-by and Reviewed-by trailers
        O->>G: remove the worktree, keep the branch
        O->>L: approved
    else any one fails
        K-->>O: Refusal
        O->>L: rejected, the worktree kept as evidence
    end
```

Each `opt` ends the run there. A refused run keeps its worktree, because that is
the evidence somebody needs; `ostraka prune` clears what is left over.

## From task to merged change

A run's record ends in one of three outcomes. `promote` asks the gate again
rather than trusting a stored answer, and stops at a branch: merging is always
a person's act.

```mermaid
stateDiagram-v2
    [*] --> Queued: ostraka task add
    [*] --> Running: ostraka run, or a task in the TUI
    Queued --> Running: run --next, or drain

    Running --> Approved: approved
    Running --> Rejected: refused
    Running --> Failed: setup failed

    Rejected --> Running: loop mode
    Approved --> Running: run --from ID

    Approved --> Promoted: ostraka promote ID
    Promoted --> Merged: a person merges or opens a pull request
    Merged --> [*]

    Rejected --> [*]
    Failed --> [*]

    note right of Promoted: a branch, and nothing merged
```

- **Approved**: the required checks passed and a profile other than the author
  approved the frozen change. It is a commit on the run's own branch.
- **Refused**, recorded as `rejected`: the author exited non-zero, timed out or
  was stopped; nothing changed; a path fell outside the policy; a required check
  failed; the reviewer said no; or the worktree changed during review. The
  record says which.
- **Setup failed**, recorded as `failed`: the worktree could not be prepared,
  so no agent ran.
- **Continuing** an approved run, with `run --from ID` or the next task in a TUI
  thread, starts from its branch. A refused run is never built on.
- **Promoting** asks the gate again against the current checks, then gives the
  run a branch. It merges nothing.

Loop mode retries a failed check, a rejected change or a run that changed
nothing. It never retries an author that could not run, a run that timed out or
was stopped, or a policy violation, because none of those is about the change.

Asking and planning are not in this picture. They consult one profile through
its read-only invocation, record the answer under `.ostraka/consulted`, and
produce no change, so there is nothing to approve, reject or promote.

## Several runs at once

`ostraka drain --workers N` takes the task list down N at a time. Each worker is
the same sequential run on a thread of its own, with its own worktree and its
own record. Routing picks an author and a reviewer for each task, so different
workers can use different profiles.

```mermaid
flowchart TB
    List[".ostraka/tasks/pending"] --> Drain["ostraka drain --workers 3"]
    Drain --> W1
    Drain --> W2
    Drain --> W3

    subgraph W1["worker 1"]
        c1["claim a task"] --> x1["run it: worktree, author, checks, review"]
    end
    subgraph W2["worker 2"]
        c2["claim a task"] --> x2["run it: worktree, author, checks, review"]
    end
    subgraph W3["worker 3"]
        c3["claim a task"] --> x3["run it: worktree, author, checks, review"]
    end

    x1 --> Runs[".ostraka/runs"]
    x2 --> Runs
    x3 --> Runs
    x1 --> Done[".ostraka/tasks/done"]
    x2 --> Done
    x3 --> Done
```

Claiming a task renames it from `pending` to `running`. The rename is atomic,
so two workers cannot take the same task, and no lock file is needed. One
Ctrl-C stops every worker; each run can also be stopped on its own.

`ostraka bench` uses the same machinery for comparison: every task in
`.ostraka/bench.toml` crossed with every candidate profile and model, reviewers
held constant, and the results tabulated by what the gate said.

## The crates

Arrows point from a crate to the crates it depends on.

```mermaid
flowchart BT
    core["ostraka-core: config, gate verdict, policy, record, task"]
    adapter["ostraka-adapter: profiles, processes, isolation, token usage"]
    runtime["ostraka-runtime: orchestrator, routing, worktrees, gate, review, promote"]
    app["ostraka-app: the desktop window, egui"]
    cli["ostraka: the command line, the TUI, bench and init"]

    adapter --> core
    runtime --> core
    runtime --> adapter
    app --> core
    app --> runtime
    cli --> core
    cli --> adapter
    cli --> runtime
```

`ostraka-core` does no IO and spawns no processes. No vendor name appears in
`ostraka-adapter` or `ostraka-runtime`: every agent CLI is a TOML profile in
`adapters/`, and adding one is a file, not a release.
