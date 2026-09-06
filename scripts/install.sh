#!/bin/sh
# Install ostraka.
#
#   curl -fsSL https://ostraka.sh/install | sh
#
# Downloads a released binary. It deliberately does not use `cargo install`:
# the point is that installing requires no toolchain.

set -eu

REPO="bemindlabs/ostraka"
INSTALL_DIR="${OSTRAKA_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${OSTRAKA_VERSION:-latest}"
# Where release artifacts are fetched from. Overridable so that this script can
# be exercised end to end against a directory of locally built artifacts —
# `file://` works with both curl and wget — rather than only ever being tested
# by cutting a real release and watching what happens to other people.
BASE_URL="${OSTRAKA_BASE_URL:-}"

say() { printf '%s\n' "$*" >&2; }
die() { say "install: $*"; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }
need uname
need tar

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1"; }
  fetch_to() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO- "$1"; }
  fetch_to() { wget -qO "$2" "$1"; }
else
  die "need curl or wget"
fi

os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
  Linux-x86_64)   target="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64|Linux-arm64) target="aarch64-unknown-linux-gnu" ;;
  Darwin-x86_64)  target="x86_64-apple-darwin" ;;
  Darwin-arm64)   target="aarch64-apple-darwin" ;;
  *) die "unsupported platform: $os $arch" ;;
esac

if [ "$VERSION" = "latest" ]; then
  VERSION=$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n1)
  [ -n "$VERSION" ] || die "could not determine the latest version"
fi

name="ostraka-${VERSION}-${target}"
if [ -n "$BASE_URL" ]; then
  url="${BASE_URL}/${name}.tar.gz"
else
  url="https://github.com/$REPO/releases/download/${VERSION}/${name}.tar.gz"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "downloading $name"
fetch_to "$url" "$tmp/$name.tar.gz" || die "download failed: $url"

# Verify the checksum when a sidecar is published and a checksum tool exists.
if fetch_to "$url.sha256" "$tmp/$name.tar.gz.sha256" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    expected=$(cut -d' ' -f1 < "$tmp/$name.tar.gz.sha256")
    actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
    [ "$expected" = "$actual" ] || die "checksum mismatch for $name.tar.gz"
    say "checksum ok"
  elif command -v shasum >/dev/null 2>&1; then
    expected=$(cut -d' ' -f1 < "$tmp/$name.tar.gz.sha256")
    actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
    [ "$expected" = "$actual" ] || die "checksum mismatch for $name.tar.gz"
    say "checksum ok"
  else
    say "warning: no sha256 tool found; skipping verification"
  fi
else
  say "warning: no published checksum for this release; skipping verification"
fi

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$INSTALL_DIR"
mv "$tmp/$name/ostraka" "$INSTALL_DIR/ostraka"
chmod +x "$INSTALL_DIR/ostraka"

say "installed ostraka $VERSION to $INSTALL_DIR/ostraka"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH" ;;
esac
