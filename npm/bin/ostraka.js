#!/usr/bin/env node
// Hands off to the downloaded binary.
//
// Thin on purpose: everything this project does happens in the Rust binary, and
// a launcher that did any of it would be the second runtime AGENTS.md forbids.

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const here = dirname(fileURLToPath(import.meta.url));
const binary = join(here, process.platform === "win32" ? "ostraka.exe" : "ostraka");

if (!existsSync(binary)) {
  // The likely cause by a wide margin, and worth naming: an install that
  // skipped scripts leaves this package with no binary and no explanation.
  console.error(
    "ostraka: no binary here. It is downloaded by a postinstall script, so an " +
      "install run with --ignore-scripts leaves nothing to run.\n" +
      "         Re-run `npm rebuild ostraka`, or install without npm: " +
      "curl -fsSL https://ostraka.sh/install | sh",
  );
  process.exit(1);
}

const { status, signal } = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
// A signalled child exits 128+n by convention; passing 0 would report a killed
// run as a successful one.
process.exit(signal ? 128 : (status ?? 1));
