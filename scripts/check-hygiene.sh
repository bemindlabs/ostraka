#!/usr/bin/env bash
#
# Repository hygiene: the rules in AGENTS.md that a compiler cannot enforce.
#
# Runs in CI on a clean checkout, and is worth running by hand before proposing
# a change. It is deliberately not one of the gate checks in ostraka.toml: an
# agent's editor plugins write into the worktree while a run is in progress, and
# a gate that fails on that would refuse correct work for someone else's reason.

set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
note() { printf '  %s\n' "$1"; }
bad() { printf 'FAIL  %s\n' "$1"; fail=1; }
ok() { printf 'ok    %s\n' "$1"; }

# 1. One source of truth. Every backend filename resolves to AGENTS.md, so that
#    reading any of them reads the same instructions. A backend file that has
#    become an independent copy will drift, and the copy is what agents then
#    read: this repository's sibling workspace spent a rename cycle briefing
#    every session on a product name abandoned over a trademark conflict,
#    because a stale copy sat where a symlink used to be.
for name in AGY CODEX COPILOT GROK KIMI OLLAMA OPENAI; do
    file="$name.md"
    [ -e "$file" ] || continue
    if [ ! -L "$file" ]; then
        bad "$file is a regular file; it must be a symlink to AGENTS.md"
        note "restore with: rm $file && ln -s AGENTS.md $file"
    elif [ "$(readlink "$file")" != "AGENTS.md" ]; then
        bad "$file points at $(readlink "$file"), not AGENTS.md"
    else
        ok "$file -> AGENTS.md"
    fi
done

# CLAUDE.md is the one that may not be a symlink. The claude-mem plugin rewrites
# CLAUDE.md in every directory it observes and replaces a symlink with a regular
# copy; an import line survives being rewritten, and a copy does not. What
# matters is that it carries no instructions of its own.
if [ ! -e CLAUDE.md ]; then
    bad "CLAUDE.md is missing; it must import AGENTS.md"
elif [ -L CLAUDE.md ]; then
    bad "CLAUDE.md is a symlink; make it a file whose only directive is @AGENTS.md"
    note "a plugin will replace the symlink with a stale copy — see AGENTS.md"
elif ! grep -qx '@AGENTS.md' CLAUDE.md; then
    bad "CLAUDE.md does not import AGENTS.md on a line of its own"
else
    # Everything outside a comment or a generated block must be a heading, the
    # import, or blank. Anything else is a second set of instructions that only
    # one backend reads — which is the whole failure this file exists to avoid.
    #
    # The claude-mem block is exempt by name because that plugin is the reason
    # this is a file rather than a symlink: its rewrite has to keep passing, or
    # the check fails on every machine where the plugin is installed and gets
    # switched off for the wrong reason.
    own=$(awk '
        /<!--/ || /<claude-mem-context>/  { inc = 1 }
        !inc && !/^#/ && !/^@AGENTS\.md$/ && NF { print }
        /-->/ || /<\/claude-mem-context>/ { inc = 0 }
    ' CLAUDE.md)
    if [ -n "$own" ]; then
        bad "CLAUDE.md carries instructions of its own; they belong in AGENTS.md:"
        printf '%s\n' "$own" | sed 's/^/      /'
    else
        ok "CLAUDE.md imports AGENTS.md and adds nothing"
    fi
fi

# 2. No backend-specific instruction files scattered through the tree. The only
#    legitimate CLAUDE.md is the root symlink above; the rest is plugin
#    scribble, and it has reached a commit twice.
strays=$(git ls-files '*CLAUDE.md' | grep -v '^CLAUDE.md$' || true)
if [ -n "$strays" ]; then
    bad "backend-specific files tracked outside the root symlink:"
    printf '%s\n' "$strays" | sed 's/^/      /'
else
    ok "no stray backend instruction files"
fi

# 3. English only — no exceptions and no bilingual pairs. Universality was the
#    deciding criterion for this generation's whole vocabulary; a reader who
#    needs a translation before they can use the tool is the failure mode.
if git grep -lP '[\x{0E00}-\x{0E7F}]' -- . >/dev/null 2>&1; then
    bad "Thai text found in:"
    git grep -lP '[\x{0E00}-\x{0E7F}]' -- . | sed 's/^/      /'
else
    ok "no Thai text"
fi

translations=$(git ls-files | grep -E '\.th\.md$|^docs/th/' || true)
if [ -n "$translations" ]; then
    bad "translated files found:"
    printf '%s\n' "$translations" | sed 's/^/      /'
else
    ok "no translated files"
fi

# 4. The gate CI runs must be the gate ostraka.toml declares. They are written
#    out in both places because CI cannot read the config before building the
#    binary that reads it, so they can silently diverge.
for check in "cargo fmt --all -- --check" "cargo clippy --all-targets" \
             "cargo test --workspace" "cargo build --workspace"; do
    if ! grep -qF "$check" ostraka.toml; then
        bad "ostraka.toml no longer declares: $check"
    fi
    if ! grep -qF "$check" .github/workflows/ci.yml; then
        bad "ci.yml no longer runs: $check"
    fi
done
ok "the gate in ci.yml matches the gate in ostraka.toml"

exit $fail
