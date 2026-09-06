# CLAUDE.md

@AGENTS.md

<!--
Not a symlink, unlike the other backend filenames in this directory, and the
exception is deliberate.

`AGENTS.md` is the single source of truth. AGY.md, CODEX.md, COPILOT.md,
GROK.md, KIMI.md, OLLAMA.md and OPENAI.md are symlinks to it, so editing any of
them edits the same file.

CLAUDE.md cannot be, because the claude-mem plugin rewrites CLAUDE.md files in
every directory it observes, replacing a symlink with a regular copy. That copy
does not track later edits: this workspace spent a full rename cycle briefing
every session on `AGOR`, a product name abandoned over a trademark conflict,
because a stale copy sat where a symlink used to be and nothing said so.

A one-line import cannot go stale. Rewrite it however you like — the pointer is
still a pointer, and the instructions still come from AGENTS.md.
-->
