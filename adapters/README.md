# Adapter profiles

One TOML file per vendor CLI. Profiles are data, not code: adding a vendor is a
file, not a release, and no vendor name appears in the runtime.

A profile describes how to run a coding agent headlessly in a directory:

```toml
id = "example"
command = "example-cli"
args = ["--print", "{{prompt}}"]
event_format = "none"   # or "jsonl"

[env]
EXAMPLE_NO_COLOR = "1"

[capabilities]
headless = true
streams_json = false
resumable = false
```

Placeholders substituted into `args`: `{{prompt}}`, `{{model}}`, `{{worktree}}`.

The unit of truth is the git diff the run leaves in the worktree — not the
vendor's transcript. An adapter only has to run the prompt to completion, exit
with a status, and leave its work on disk.
