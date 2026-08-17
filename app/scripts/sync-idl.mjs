/**
 * Copies the IDLs `anchor build` generates into the app as TypeScript modules.
 *
 * The dashboard builds its transactions *from the IDL* rather than from
 * hand-written discriminators and account orders. That removes the drift the
 * indexer already learned about the hard way — see `event_coverage.rs`, where a
 * hand-maintained event list silently stopped decoding the two events that
 * record governance becoming self-governing. An instruction encoder has the same
 * shape of hazard and a worse failure: a stale account order does not fail to
 * decode, it signs the wrong transaction.
 *
 * Three things about the way this is done are deliberate:
 *
 * - **A copy, not a path into `target/`.** `target/` is gitignored and the CI
 *   dashboard job has no Anchor toolchain, so an import reaching into it would
 *   make the app unbuildable in exactly the environment that builds it.
 * - **`.ts`, not `.json`.** A JSON import needs `with { type: "json" }` under
 *   Node's ESM loader and needs it *absent* for some bundlers. A TypeScript
 *   module is unambiguous to both, and it type-checks against `Idl` — so an IDL
 *   shape this app cannot handle is a compile error rather than a runtime one.
 * - **The copy is checked by a test that can see both.** `idl_sync.rs` runs in
 *   the Rust job, which does have `anchor build` output, and fails if these
 *   files are not exactly what the programs now generate. A copy nobody compares
 *   is a fork.
 *
 * Run after `anchor build`:  node scripts/sync-idl.mjs
 */

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const generated = resolve(here, "../../target/idl");
const destination = resolve(here, "../src/idl");

/** Only what the dashboard actually builds transactions against. */
const PROGRAMS = ["helix_staking", "helix_governance"];

const BANNER = (name) => `/**
 * Generated from \`target/idl/${name}.json\` by \`node scripts/sync-idl.mjs\`.
 *
 * Do not edit. \`tests/integration/tests/idl_sync.rs\` fails if this file is not
 * byte-for-byte what \`anchor build\` currently produces.
 */
`;

mkdirSync(destination, { recursive: true });

for (const name of PROGRAMS) {
  const source = join(generated, `${name}.json`);
  let raw;
  try {
    raw = readFileSync(source, "utf8");
  } catch (cause) {
    console.error(`cannot read ${source} — run \`anchor build\` first (${cause.message})`);
    process.exit(1);
  }

  // Re-serialise rather than pasting the file through: this is the same
  // normalisation `idl_sync.rs` applies before comparing, so the two agree on
  // formatting and disagree only about content.
  const idl = JSON.parse(raw);
  const body = JSON.stringify(idl, null, 2);

  const module = `${BANNER(name)}
import type { Idl } from "../lib/idl.ts";

const idl: Idl = ${body};

export default idl;
`;

  const out = join(destination, `${name}.ts`);
  writeFileSync(out, module);
  console.log(`${name}: ${body.length} bytes -> ${out}`);
}
