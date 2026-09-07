#!/usr/bin/env node
// Fetches the released binary that matches this package's version.
//
// The npm package carries no binary of its own: it names a version, downloads
// the artifact the release workflow published for this platform, and verifies
// the checksum published beside it. `ostraka-<tag>-<target>.tar.gz` is the same
// contract scripts/install.sh and Formula/ostraka.rb parse — three readers of
// one name, which is why check-hygiene.sh refuses to let them drift.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, mkdirSync, copyFileSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const tag = `v${version}`;

// Kept in step with the release matrix by check-hygiene.sh: a platform the
// workflow builds and this table does not know is a silent install failure.
const TARGETS = {
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
};

function die(message) {
  console.error(`ostraka: ${message}`);
  process.exit(1);
}

const key = `${process.platform}-${process.arch}`;
const target = TARGETS[key];
if (!target) {
  die(
    `no released binary for ${key}. Supported: ${Object.keys(TARGETS).join(", ")}. ` +
      `Build from source instead: cargo install --git https://github.com/bemindlabs/ostraka ostraka-cli`,
  );
}

const name = `ostraka-${tag}-${target}`;
const base =
  process.env.OSTRAKA_BASE_URL ??
  `https://github.com/bemindlabs/ostraka/releases/download/${tag}`;
const binaryName = process.platform === "win32" ? "ostraka.exe" : "ostraka";

/** Reads a release file, from a URL or from a local directory. */
async function read(file) {
  // `file://` is supported so this script can be exercised against locally
  // built artifacts. Node's fetch does not implement it, hence the branch.
  if (base.startsWith("file://")) {
    return readFileSync(join(fileURLToPath(base), file));
  }
  const response = await fetch(`${base}/${file}`);
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${base}/${file}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

const scratch = mkdtempSync(join(tmpdir(), "ostraka-install-"));
try {
  const archive = join(scratch, `${name}.tar.gz`);
  writeFileSync(archive, await read(`${name}.tar.gz`).catch((e) => die(`download failed — ${e.message}`)));

  // A checksum that is published and not checked is decoration. A checksum that
  // is absent is reported rather than assumed to be fine.
  try {
    const sidecar = (await read(`${name}.tar.gz.sha256`)).toString("utf8");
    const expected = sidecar.trim().split(/\s+/)[0];
    const actual = createHash("sha256").update(readFileSync(archive)).digest("hex");
    if (expected !== actual) {
      die(`checksum mismatch for ${name}.tar.gz`);
    }
  } catch (e) {
    if (e instanceof Error && e.message.startsWith("checksum")) throw e;
    console.error(`ostraka: no published checksum for ${tag}; skipping verification`);
  }

  // tar rather than a bundled decompressor: it is on every supported platform,
  // Windows 10 included, and a dependency-free installer is worth a subprocess.
  const untar = spawnSync("tar", ["-xzf", archive, "-C", scratch], { stdio: "inherit" });
  if (untar.status !== 0) {
    die("could not extract the archive; is `tar` on PATH?");
  }

  mkdirSync(join(root, "bin"), { recursive: true });
  const installed = join(root, "bin", binaryName);
  copyFileSync(join(scratch, name, binaryName), installed);
  if (process.platform !== "win32") {
    chmodSync(installed, 0o755);
  }
  console.error(`ostraka: installed ${tag} for ${target}`);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
