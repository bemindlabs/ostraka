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

# The release matrix, read once. `\S` is a GNU extension rather than portable
# ERE, so on a grep that does not take it this produced an empty list and every
# loop below quietly checked nothing — a silently-passing check being exactly
# what this script exists to prevent elsewhere. POSIX class, and the emptiness
# is an error rather than a pass.
targets=$(grep -oE '^ +- target: [^[:space:]]+' .github/workflows/release.yml | awk '{print $3}' | sort -u)
if [ -z "$targets" ]; then
    bad "no targets could be read out of .github/workflows/release.yml"
    note "every platform-coverage check below depends on this list"
else
    ok "the release matrix names $(printf '%s\n' "$targets" | wc -l | tr -d ' ') platforms"
fi

# 5. The release artifact name is a contract. Four files construct or parse it
#    — the workflow that publishes, the script that installs, the formula that
#    taps, and the `update` command that replaces a running binary with one —
#    and none of them can see the other three. Getting it wrong breaks installs
#    silently, for other people, after a tag has already been cut.
if ! grep -qF 'name="ostraka-${tag}-${{ matrix.target }}"' .github/workflows/release.yml; then
    bad "release.yml no longer packages ostraka-<tag>-<target>.tar.gz"
elif ! grep -qF 'name="ostraka-${VERSION}-${target}"' scripts/install.sh; then
    bad "install.sh no longer expects ostraka-<version>-<target>.tar.gz"
elif ! grep -qF 'ostraka-v#{version}-' Formula/ostraka.rb; then
    bad "Formula/ostraka.rb no longer expects ostraka-v<version>-<target>.tar.gz"
elif ! grep -qF 'format!("ostraka-{tag}-{target}")' crates/ostraka/src/update.rs; then
    bad "update.rs no longer expects ostraka-<tag>-<target>.tar.gz"
    note "\`ostraka update\` downloads by this name; a rename here strands"
    note "every already-installed copy, which is the half nobody can fix later"
else
    ok "install.sh, the formula and update parse the name release.yml builds"
fi

# The update command has to know every platform the release publishes, or it
# tells the people on the missing one that no release exists for them.
for target in $targets; do
    if ! grep -qF "$target" crates/ostraka/src/update.rs; then
        bad "release.yml builds $target and \`ostraka update\` cannot fetch it"
    fi
done
ok "every released platform is reachable from ostraka update"

# Every platform the release builds must be installable by both paths. Adding a
# target to the matrix and forgetting the installer is the quiet half of that
# contract; Windows is deliberately binary-only, with no formula.
for target in $targets; do
    case "$target" in *windows*) continue ;; esac
    if ! grep -qF "$target" scripts/install.sh; then
        bad "release.yml builds $target and install.sh cannot install it"
    fi
    if ! grep -qF "$target" Formula/ostraka.rb; then
        bad "release.yml builds $target and the formula does not name it"
    fi
done
ok "every released platform is reachable from install.sh and the formula"

# 6. The npm shim names a version and a platform table, and neither can be
#    allowed to drift from the release. A package published at the wrong version
#    downloads an artifact that does not exist; a platform the workflow builds
#    and the table omits is an install that fails only for the people on it.
npm_version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' npm/package.json | head -n1)
crate_version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)
if [ "$npm_version" != "$crate_version" ]; then
    bad "npm/package.json is $npm_version and Cargo.toml is $crate_version"
    note "the npm package downloads ostraka-v<its own version>-<target>.tar.gz"
else
    ok "the npm shim is the same version as the crates"
fi

for target in $targets; do
    if ! grep -qF "$target" npm/scripts/install.js; then
        bad "release.yml builds $target and the npm shim cannot install it"
    fi
done
ok "every released platform is reachable from the npm shim"

# 7. `ostraka init` writes adapter profiles into a new project, and it carries
#    them as embedded copies because a binary installed by curl has no
#    repository beside it. Copies drift; these two must not.
for profile in adapters/*.toml; do
    template="crates/ostraka/templates/$(basename "$profile")"
    if [ ! -f "$template" ]; then
        bad "$profile has no template in crates/ostraka/templates/"
        note "cp $profile $template"
    elif ! cmp -s "$profile" "$template"; then
        bad "$template has drifted from $profile"
        note "the profile init writes would differ from the one this repo ships"
        note "cp $profile $template"
    fi
done
for template in crates/ostraka/templates/*.toml; do
    [ -f "adapters/$(basename "$template")" ] || bad "$template has no profile in adapters/"
done
ok "the profiles init writes are the ones this repository ships"

# 7b. `streams_json` describes the invocation a profile declares, which makes it
#     exactly `event_format != "none"` — two spellings of one fact, in one file,
#     that a compiler cannot hold together. They have already come apart twice:
#     `codex` and `kimi-cli` both claimed to emit events while running their CLI
#     in a prose mode, on the strength of what the binary can do elsewhere.
#     The field goes at the next major; until then this is what stops it lying.
for profile in adapters/*.toml crates/ostraka/templates/*.toml; do
    format=$(sed -n 's/^event_format *= *"\(.*\)".*/\1/p' "$profile" | head -1)
    [ -n "$format" ] || format="none"
    declared=$(sed -n 's/^streams_json *= *\(.*\)/\1/p' "$profile" | head -1)
    [ -n "$declared" ] || declared="false"
    if [ "$format" = "none" ]; then expected="false"; else expected="true"; fi
    if [ "$declared" != "$expected" ]; then
        bad "$profile says streams_json = $declared with event_format = \"$format\""
        note "a profile describes one command line: if it emits no parseable"
        note "events it does not stream json, whatever the CLI can do elsewhere"
        note "expected streams_json = $expected"
    fi
done
ok "every profile's streams_json agrees with its event_format"

# 8. A relative link in a file this repository ships must resolve inside this
#    repository. The README footer pointed at PHILOSOPHY.md for a release while
#    the file lived one directory up in the workspace — fine on the machine that
#    wrote it, a 404 for every visitor, and invisible to every other check here.
#    Clone-and-read is the only way that surfaces, so it is checked instead.
missing=0
for doc in README.md AGENTS.md PHILOSOPHY.md adapters/README.md; do
    [ -f "$doc" ] || continue
    dir=$(dirname "$doc")
    # Markdown links and bare hrefs, minus anything with a scheme or an anchor.
    for link in $(grep -oE '\]\([^)]+\)|href="[^"]+"' "$doc" \
                  | sed -E 's/^\]\(//; s/\)$//; s/^href="//; s/"$//' \
                  | grep -vE '^(https?:|mailto:|#)' \
                  | sed -E 's/#.*$//' | sort -u); do
        [ -n "$link" ] || continue
        if [ ! -e "$dir/$link" ]; then
            bad "$doc links to $link, which does not exist in this repository"
            missing=1
        fi
    done
done
[ "$missing" -eq 0 ] && ok "every relative link in the shipped docs resolves"

# 9. The two things about distribution that nothing else here can see.
#
#    A tap in a repository not named `homebrew-<something>` is only reachable
#    when `brew tap` is given a URL — brew appends the prefix itself otherwise
#    and looks somewhere that does not exist. The formula shipped for a release
#    while the README offered the bare two-argument form, which resolves to a
#    repository nobody has created. Check 6 compares the artifact *name* across
#    the three installers; it has nothing to say about where a formula is
#    served from.
if [ -f Formula/ostraka.rb ]; then
    if grep -qE 'brew tap +bemindlabs/ostraka +https://github.com/bemindlabs/ostraka' README.md; then
        ok "the brew instructions tap this repository by URL"
    else
        bad "README.md must tap by URL, or brew looks for bemindlabs/homebrew-ostraka"
        note "brew tap bemindlabs/ostraka https://github.com/bemindlabs/ostraka"
    fi
fi

#    And every file the npm package promises to ship has to exist. `npm publish`
#    does not fail on a missing one — it publishes a package whose page is
#    blank, which is only ever noticed on the day the name is claimed.
missing_npm=0
for file in $(sed -n '/"files"/,/]/p' npm/package.json | grep -oE '"[^"]+\.[a-z]+"' | tr -d '"'); do
    [ -f "npm/$file" ] || { bad "npm/package.json ships $file, which does not exist"; missing_npm=1; }
done
[ "$missing_npm" -eq 0 ] && ok "every file the npm package ships exists"

exit $fail
