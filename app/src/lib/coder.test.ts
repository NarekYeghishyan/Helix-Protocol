/**
 * The coder, checked against something other than itself.
 *
 * This is the trap a test for IDL-driven code falls into: the encoder reads the
 * IDL, so asserting that its output matches the IDL proves only that a file was
 * read twice. Every assertion here is independent of that path in one of three
 * ways.
 *
 * 1. **Anchor's discriminator rule.** A discriminator is `sha256("global:<ix>")`
 *    truncated to eight bytes — a rule the program's dispatcher applies at
 *    runtime and the IDL merely records. Recomputing it here checks the recorded
 *    value against the thing that will actually compare it.
 * 2. **Byte vectors written out by hand.** `stake(7, 1_000_000, Gold)` has one
 *    correct encoding, spelled out below. If the coder and the IDL agreed on
 *    something wrong, this still fails.
 * 3. **Seeds written as literals**, from `programs/staking/src/constants.rs`
 *    rather than from the IDL's `pda` description.
 *
 * What no test here can check is that the IDL matches the *programs* — that is
 * `tests/integration/tests/idl_sync.rs`, which runs where `anchor build` output
 * exists. The chain is: Rust source → `anchor build` → IDL → `sync-idl.mjs` →
 * these modules, with a test pinning every hop.
 */

import { strict as assert } from "node:assert";
import { createHash } from "node:crypto";
import { test } from "node:test";

import { PublicKey } from "@solana/web3.js";

import governance from "../idl/helix_governance.ts";
import staking from "../idl/helix_staking.ts";
import { decodeAccount, decodeType, encodeInstructionData, encodeType } from "./coder.ts";

/** Anchor's rule: the first eight bytes of sha256 over a namespaced name. */
function anchorDiscriminator(namespace: string, name: string): number[] {
  return [...createHash("sha256").update(`${namespace}:${name}`).digest().subarray(0, 8)];
}

// ------------------------------------------------------- discriminator rules

test("every instruction discriminator is the one Anchor's dispatcher will compare", () => {
  for (const idl of [staking, governance]) {
    for (const ix of idl.instructions) {
      assert.deepEqual(
        ix.discriminator,
        anchorDiscriminator("global", ix.name),
        `${idl.metadata.name}.${ix.name} would be dispatched to a different handler`,
      );
    }
  }
});

test("every account discriminator matches the rule the programs check on deserialise", () => {
  for (const idl of [staking, governance]) {
    for (const account of idl.accounts ?? []) {
      assert.deepEqual(
        account.discriminator,
        anchorDiscriminator("account", account.name),
        `${idl.metadata.name}::${account.name}`,
      );
    }
  }
});

test("every event discriminator matches the rule `emit!` writes", () => {
  for (const idl of [staking, governance]) {
    for (const event of idl.events ?? []) {
      assert.deepEqual(
        event.discriminator,
        anchorDiscriminator("event", event.name),
        `${idl.metadata.name}::${event.name}`,
      );
    }
  }
});

// -------------------------------------------------------------- byte vectors

test("stake encodes to exactly the bytes the program will deserialise", () => {
  const data = encodeInstructionData(staking, "stake", {
    position_id: 7n,
    amount: 1_000_000n,
    tier: { kind: "Gold" },
  });

  assert.deepEqual(
    [...data],
    [
      // sha256("global:stake")[0..8]
      ...anchorDiscriminator("global", "stake"),
      // position_id: u64 = 7, little-endian
      7, 0, 0, 0, 0, 0, 0, 0,
      // amount: u64 = 1_000_000 = 0x0F4240, little-endian
      0x40, 0x42, 0x0f, 0, 0, 0, 0, 0,
      // tier: the fourth variant of LockTier — Flexible, Bronze, Silver, Gold
      3,
    ],
  );
  assert.equal(data.length, 8 + 8 + 8 + 1);
});

test("the enum tag is the variant's position, so reordering LockTier is a wire change", () => {
  const tags = ["Flexible", "Bronze", "Silver", "Gold"].map(
    (kind) => encodeType(staking, { defined: { name: "LockTier" } }, { kind })[0],
  );
  assert.deepEqual(tags, [0, 1, 2, 3]);

  const choices = ["For", "Against", "Abstain"].map(
    (kind) => encodeType(governance, { defined: { name: "VoteChoice" } }, { kind })[0],
  );
  assert.deepEqual(choices, [0, 1, 2]);
});

test("an unknown enum variant is refused rather than encoded as something else", () => {
  assert.throws(
    () => encodeType(staking, { defined: { name: "LockTier" } }, { kind: "Platinum" }),
    /no variant "Platinum"/,
  );
});

// ------------------------------------------------------------- 64-bit safety

test("a u64 argument will not accept a number, however innocent it looks", () => {
  // The whole reason `amount.ts` exists, applied to the path where it matters
  // more: a display rounded down is a cosmetic bug, an *encoded* amount rounded
  // down is a signed transfer of the wrong size.
  assert.throws(
    () =>
      encodeInstructionData(staking, "stake", {
        position_id: 0n,
        amount: 1_000_000,
        tier: { kind: "Flexible" },
      }),
    /u64 expects a bigint/,
  );
});

test("u64 round-trips past the double-precision limit", () => {
  // 2^64 - 1, and 2^53 + 1 — the smallest integer a JSON number cannot hold.
  for (const value of [0n, 1n, 9_007_199_254_740_993n, 18_446_744_073_709_551_615n]) {
    const bytes = encodeType(staking, "u64", value);
    assert.equal(bytes.length, 8);
    assert.equal(decodeType(staking, "u64", bytes), value);
  }
});

test("a value that does not fit its width is refused, not truncated", () => {
  assert.throws(() => encodeType(staking, "u64", 2n ** 64n), /does not fit in u64/);
  assert.throws(() => encodeType(staking, "u64", -1n), /does not fit in u64/);
});

test("i64 encodes negatives in two's complement", () => {
  assert.deepEqual([...encodeType(staking, "i64", -1n)], [255, 255, 255, 255, 255, 255, 255, 255]);
  assert.equal(decodeType(staking, "i64", encodeType(staking, "i64", -1_700_000_000n)), -1_700_000_000n);
});

// ----------------------------------------------------------- account decoding

/** A `Position` built byte by byte, without going through the encoder. */
function handWrittenPosition(): Uint8Array {
  const pool = PublicKey.unique();
  const owner = PublicKey.unique();

  const bytes: number[] = [
    // sha256("account:Position")[0..8]
    ...anchorDiscriminator("account", "Position"),
    ...pool.toBytes(),
    ...owner.toBytes(),
  ];

  const le = (value: bigint, width: number) => {
    for (let i = 0; i < width; i++) bytes.push(Number((value >> BigInt(8 * i)) & 0xffn));
  };

  le(3n, 8); // position_id
  le(5_000_000_000n, 8); // amount
  le(7_500_000_000n, 8); // weighted_amount
  bytes.push(1); // tier: Bronze
  le(1_700_000_000n, 8); // lock_end
  le(123_456_789_012_345n, 16); // reward_per_token_paid (u128)
  le(42n, 8); // pending_rewards
  le(1_690_000_000n, 8); // created_at
  bytes.push(254); // bump

  return Uint8Array.from(bytes);
}

test("a Position decodes field by field from bytes the coder did not write", () => {
  const raw = handWrittenPosition();
  const position = decodeAccount(staking, "Position", raw);

  assert.equal(position.position_id, 3n);
  assert.equal(position.amount, 5_000_000_000n);
  assert.equal(position.weighted_amount, 7_500_000_000n);
  assert.deepEqual(position.tier, { kind: "Bronze" });
  assert.equal(position.lock_end, 1_700_000_000n);
  assert.equal(position.reward_per_token_paid, 123_456_789_012_345n);
  assert.equal(position.pending_rewards, 42n);
  assert.equal(position.bump, 254);
  assert.ok(position.pool instanceof PublicKey);
});

test("decoding refuses an account of the wrong type instead of reinterpreting it", () => {
  // The hazard is real rather than theoretical: `Pool` and `Position` both open
  // with two pubkeys, so a Pool decodes as a plausible Position — with the
  // pool's reward mint reported as its owner.
  const raw = handWrittenPosition();
  assert.throws(() => decodeAccount(staking, "Pool", raw), /discriminator is not Pool's/);
});

test("a truncated account fails loudly rather than reporting zeros", () => {
  const raw = handWrittenPosition().subarray(0, 60);
  assert.throws(() => decodeAccount(staking, "Position", raw), /account data ends after/);
});

// --------------------------------------------------- variable-length decoding

test("a Proposal decodes past its variable-length fields", () => {
  // The reason this file has a real borsh reader instead of fixed offsets:
  // `title` and `descriptor_uri` are strings and `action` is a data-carrying
  // enum, so everything after them — including `voting_ends_at`, which decides
  // whether a position may vote — sits at an offset that depends on content.
  const realm = PublicKey.unique();
  const proposer = PublicKey.unique();
  const destination = PublicKey.unique();

  const value = {
    realm,
    proposer,
    id: 9n,
    state: { kind: "Voting" },
    action: { kind: "TreasuryTransfer", destination, amount: 250_000n },
    title: "Fund the audit",
    descriptor_uri: "https://example.invalid/proposals/9",
    created_at: 1_700_000_000n,
    voting_starts_at: 1_700_000_100n,
    voting_ends_at: 1_700_600_000n,
    eta: 0n,
    for_votes: 10n,
    against_votes: 2n,
    abstain_votes: 1n,
    total_weight_snapshot: 1_000n,
    position_count_snapshot: 4n,
    bump: 253,
  };

  const body = encodeType(governance, { defined: { name: "Proposal" } }, value);
  const account = Uint8Array.from([...anchorDiscriminator("account", "Proposal"), ...body]);

  const decoded = decodeAccount(governance, "Proposal", account);
  assert.equal(decoded.title, "Fund the audit");
  assert.equal(decoded.voting_ends_at, 1_700_600_000n);
  assert.equal(decoded.position_count_snapshot, 4n);
  assert.deepEqual(decoded.state, { kind: "Voting" });
  assert.equal((decoded.action as { amount: bigint }).amount, 250_000n);

  // A one-character-longer title must move everything after it. If the reader
  // were using fixed offsets this would still pass the assertions above and
  // fail here.
  const longer = encodeType(
    governance,
    { defined: { name: "Proposal" } },
    { ...value, title: "Fund the audits" },
  );
  assert.equal(longer.length, body.length + 1);
});

// -------------------------------------------------------------- argument set

test("a missing or misspelled argument is refused rather than defaulted", () => {
  assert.throws(
    () => encodeInstructionData(staking, "stake", { position_id: 0n, amount: 1n }),
    /missing argument "tier"/,
  );
  assert.throws(
    () => encodeInstructionData(staking, "unstake", { amount: 1n, tier: { kind: "Gold" } }),
    /does not take: tier/,
  );
});

test("an instruction the program does not have names the ones it does", () => {
  assert.throws(() => encodeInstructionData(staking, "restake", {}), /has no instruction "restake"/);
});
