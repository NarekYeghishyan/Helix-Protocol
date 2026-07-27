#!/usr/bin/env node
/**
 * Generates Solana-format program keypairs for each program in the workspace.
 *
 * Writes `target/deploy/<name>-keypair.json` (the 64-byte secret||public array that
 * `solana` and `anchor` expect) and prints the base58 program IDs for Anchor.toml
 * and `declare_id!`.
 *
 * These files are gitignored. Regenerating them changes your program IDs — only run
 * this once, at project setup, or when intentionally rotating a dev deployment.
 *
 *   node scripts/gen-program-keys.mjs
 */
import { generateKeyPairSync } from "node:crypto";
import { mkdirSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// These must match each crate's `[lib] name`, because that is the filename
// `anchor build` looks for in target/deploy. A mismatch is silent: anchor simply
// generates its own keypair alongside yours and uses that one instead.
const PROGRAMS = [
  "helix_token_manager",
  "helix_staking",
  "helix_governance",
  "helix_treasury",
];

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(ROOT, "target", "deploy");

const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Base58 (Bitcoin alphabet) encoding of a byte array. */
function base58(bytes) {
  const digits = [0];
  for (const byte of bytes) {
    let carry = byte;
    for (let i = 0; i < digits.length; i++) {
      carry += digits[i] << 8;
      digits[i] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  // Each leading zero byte is one leading '1'.
  let out = "";
  for (const byte of bytes) {
    if (byte !== 0) break;
    out += "1";
  }
  for (let i = digits.length - 1; i >= 0; i--) out += B58[digits[i]];
  return out;
}

/** An ed25519 keypair as { seed: Uint8Array(32), publicKey: Uint8Array(32) }. */
function ed25519Keypair() {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const priv = privateKey.export({ format: "jwk" });
  const pub = publicKey.export({ format: "jwk" });
  return {
    seed: Buffer.from(priv.d, "base64url"),
    publicKey: Buffer.from(pub.x, "base64url"),
  };
}

mkdirSync(OUT_DIR, { recursive: true });

const results = [];
let skipped = 0;

for (const name of PROGRAMS) {
  const path = join(OUT_DIR, `${name}-keypair.json`);

  if (existsSync(path)) {
    // Never silently replace an existing key — that would orphan a deployment.
    const existing = JSON.parse(
      await import("node:fs/promises").then((fs) => fs.readFile(path, "utf8")),
    );
    results.push({ name, id: base58(Buffer.from(existing.slice(32, 64))), status: "kept" });
    skipped++;
    continue;
  }

  const { seed, publicKey } = ed25519Keypair();
  // Solana keypair file format: JSON array of 64 bytes = seed(32) || publicKey(32).
  const secretKey = Buffer.concat([seed, publicKey]);
  writeFileSync(path, JSON.stringify(Array.from(secretKey)));
  results.push({ name, id: base58(publicKey), status: "created" });
}

const pad = Math.max(...results.map((r) => r.name.length));
console.log(`\nProgram keypairs in target/deploy/${skipped ? `  (${skipped} kept)` : ""}\n`);
for (const { name, id, status } of results) {
  const mark = status === "created" ? "+" : "=";
  console.log(`  ${mark} ${name.padEnd(pad)}  ${id}`);
}

console.log(`\nAnchor.toml [programs.localnet]:\n`);
for (const { name, id } of results) console.log(`${name} = "${id}"`);
console.log();
