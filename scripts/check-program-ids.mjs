#!/usr/bin/env node
/**
 * Asserts that every program's address is declared identically everywhere it is
 * written down: `declare_id!` in the crate, and each `[programs.<cluster>]`
 * table in Anchor.toml.
 *
 *   node scripts/check-program-ids.mjs
 *
 * # Why this exists, and why it is not `anchor keys verify`
 *
 * CI used to run `anchor keys verify` immediately after generating fresh
 * keypairs. That check compares `declare_id!` against the *private keys* in
 * `target/deploy/`, which are gitignored and generated per developer — so on a
 * clean runner it compared a committed address against a keypair invented three
 * seconds earlier. It never passed, and could not: it was a check whose
 * preconditions the job itself destroyed. Because it sat ahead of the build, the
 * entire Rust suite was skipped on every run the repository has ever had.
 *
 * The keypair is a deployment secret. The *address* is source, written in three
 * places that must agree, and disagreement is the real failure this was reaching
 * for: `declare_id!` is compiled into the bytecode, while Anchor.toml is what
 * `anchor deploy` and the TypeScript clients read. When they drift, the program
 * is deployed to one address and addressed at another, and the symptom is
 * `DeclaredProgramIdMismatch` at runtime rather than anything at build time.
 *
 * That comparison needs no keypairs, is identical on every machine, and can
 * actually fail — which is the only reason to run it.
 */
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** `[lib] name` for each crate under programs/, keyed by directory. */
function programCrates() {
  const dir = join(ROOT, "programs");
  const crates = [];

  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const manifest = join(dir, entry.name, "Cargo.toml");
    if (!existsSync(manifest)) continue;

    // The `[lib]` name, not `[package]` name — they differ by a hyphen, and the
    // lib name is what Anchor.toml keys on and what target/deploy is named for.
    const toml = readFileSync(manifest, "utf8");
    const lib = section(toml, "lib").name;
    if (!lib) fail(`${manifest} has no [lib] name`);

    crates.push({ dir: entry.name, lib, manifest });
  }

  if (crates.length === 0) fail(`no program crates under ${dir}`);
  return crates.sort((a, b) => a.lib.localeCompare(b.lib));
}

/**
 * The `key = "value"` pairs of one TOML table, by exact header.
 *
 * Deliberately not a TOML parser. It handles the shape these two files actually
 * have — flat tables of quoted strings — and anything it cannot read becomes a
 * missing key, which every caller below reports rather than skips.
 */
function section(toml, header) {
  const out = {};
  let inside = false;

  for (const raw of toml.split("\n")) {
    const line = raw.trim();
    if (line.startsWith("#") || line === "") continue;

    if (line.startsWith("[")) {
      inside = line === `[${header}]`;
      continue;
    }
    if (!inside) continue;

    const eq = line.indexOf("=");
    if (eq === -1) continue;
    const key = line.slice(0, eq).trim();
    const value = line
      .slice(eq + 1)
      .trim()
      .replace(/^"(.*)"$/, "$1");
    out[key] = value;
  }
  return out;
}

/** Every `[programs.<cluster>]` table in Anchor.toml. */
function anchorClusters(toml) {
  const clusters = {};
  for (const line of toml.split("\n")) {
    const match = line.trim().match(/^\[programs\.([A-Za-z0-9_-]+)\]$/);
    if (match) clusters[match[1]] = section(toml, `programs.${match[1]}`);
  }
  return clusters;
}

function declaredId(crate) {
  const path = join(ROOT, "programs", crate.dir, "src", "lib.rs");
  const source = readFileSync(path, "utf8");
  const match = source.match(/declare_id!\s*\(\s*"([1-9A-HJ-NP-Za-km-z]+)"\s*\)/);
  if (!match) fail(`${path} has no declare_id!`);
  return match[1];
}

const problems = [];
function fail(message) {
  problems.push(message);
}

// ---------------------------------------------------------------------------

const crates = programCrates();
const anchorToml = readFileSync(join(ROOT, "Anchor.toml"), "utf8");
const clusters = anchorClusters(anchorToml);

if (Object.keys(clusters).length === 0) {
  fail("Anchor.toml declares no [programs.<cluster>] table");
}

const rows = [];

for (const crate of crates) {
  const declared = declaredId(crate);
  const row = { lib: crate.lib, declared, clusters: {} };

  for (const [cluster, entries] of Object.entries(clusters)) {
    const listed = entries[crate.lib];
    row.clusters[cluster] = listed ?? "—";

    if (listed === undefined) {
      fail(`Anchor.toml [programs.${cluster}] does not list ${crate.lib}`);
    } else if (listed !== declared) {
      fail(
        `${crate.lib}: declare_id! says ${declared}, ` +
          `Anchor.toml [programs.${cluster}] says ${listed}`,
      );
    }
  }
  rows.push(row);
}

// An address listed for a program that no longer exists is drift in the other
// direction — harmless to the build, and a trap for anyone reading Anchor.toml
// to find out what this workspace deploys.
const libs = new Set(crates.map((c) => c.lib));
for (const [cluster, entries] of Object.entries(clusters)) {
  for (const key of Object.keys(entries)) {
    if (!libs.has(key)) {
      fail(`Anchor.toml [programs.${cluster}] lists ${key}, which is not a crate under programs/`);
    }
  }
}

// ---------------------------------------------------------------------------

const clusterNames = Object.keys(clusters);
const pad = Math.max(...rows.map((r) => r.lib.length), 7);

console.log("\nProgram addresses\n");
console.log(`  ${"program".padEnd(pad)}  ${"declare_id!".padEnd(44)}  ${clusterNames.join("  ")}`);
for (const row of rows) {
  const agree = clusterNames.map((c) => {
    if (row.clusters[c] === row.declared) return "ok";
    // Absent and wrong are different repairs — one is an addition, the other a
    // correction — so the column says which.
    return row.clusters[c] === "—" ? "missing" : "MISMATCH";
  });
  console.log(`  ${row.lib.padEnd(pad)}  ${row.declared.padEnd(44)}  ${agree.join("  ")}`);
}

if (problems.length > 0) {
  console.error(`\n${problems.length} problem(s):\n`);
  for (const p of problems) console.error(`  - ${p}`);
  console.error(
    "\nFix by making the addresses agree. If you rotated keypairs on purpose, run\n" +
      "`anchor keys sync` to rewrite declare_id! and Anchor.toml together, then\n" +
      "regenerate the dashboard IDLs with `node app/scripts/sync-idl.mjs`.\n",
  );
  process.exit(1);
}

console.log(`\nAll ${rows.length} programs agree across declare_id! and ${clusterNames.length} cluster table(s).\n`);
