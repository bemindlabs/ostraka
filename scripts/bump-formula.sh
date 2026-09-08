#!/usr/bin/env bash
#
# Point the Homebrew formula at a release.
#
#   scripts/bump-formula.sh v1.0.0 dist/
#
# The second argument is a directory holding the release's `.sha256` sidecars,
# named exactly as the release workflow packages them. Homebrew needs a checksum
# per platform and those exist only after the artifacts are built, so this runs
# after the build, not before the tag.
#
# It refuses rather than guesses: a missing sidecar, or a placeholder checksum
# left behind, fails here instead of producing a formula that installs nothing.

set -euo pipefail
cd "$(dirname "$0")/.."

tag="${1:-}"
dir="${2:-}"
[ -n "$tag" ] && [ -n "$dir" ] || { echo "usage: $0 <tag> <sidecar-dir>" >&2; exit 2; }

formula="Formula/ostraka.rb"
version="${tag#v}"

# The platforms the formula names. Windows ships a binary but not a formula.
targets="aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu"

map=$(mktemp)
trap 'rm -f "$map"' EXIT
for target in $targets; do
    sidecar="$dir/ostraka-${tag}-${target}.tar.gz.sha256"
    [ -f "$sidecar" ] || { echo "no checksum for $target at $sidecar" >&2; exit 1; }
    printf '%s %s\n' "$target" "$(cut -d' ' -f1 < "$sidecar")" >> "$map"
done

awk -v version="$version" '
    NR == FNR { sha[$1] = $2; next }
    /^  version "/ { print "  version \"" version "\""; next }
    {
        if ($0 ~ /url "/) { for (t in sha) if (index($0, t) > 0) current = t }
        if ($0 ~ /^ *sha256 "/ && current != "") {
            sub(/"[0-9a-f]*"/, "\"" sha[current] "\"")
            current = ""
        }
        print
    }
' "$map" "$formula" > "$formula.new"
mv "$formula.new" "$formula"

if grep -q 'sha256 "0\{64\}"' "$formula"; then
    echo "a placeholder checksum survived the rewrite; the formula would install nothing" >&2
    exit 1
fi
grep -q "version \"$version\"" "$formula" || { echo "the version was not rewritten" >&2; exit 1; }

echo "Formula/ostraka.rb now points at $tag"
