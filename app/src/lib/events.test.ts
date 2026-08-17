/**
 * Event decoding, over log lines shaped the way the runtime writes them.
 *
 * This is the mechanism the claim preview depends on: it reports the amount the
 * simulated `claim` actually transferred, taken from the `RewardsClaimed` the
 * program emitted, rather than recomputing `Position::earned` in TypeScript. So
 * the thing worth testing is not that decoding works in general but that the
 * specific field the UI reads survives a round trip through base64.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { PublicKey } from "@solana/web3.js";

import { encodeType } from "./coder.ts";
import { amountField, decodeEvents, firstEvent } from "./events.ts";
import governance from "../idl/helix_governance.ts";
import staking from "../idl/helix_staking.ts";

/** The line `emit!` produces: `Program data: ` and base64 of disc + body. */
function emitted(idl: typeof staking, name: string, fields: Record<string, unknown>): string {
  const declared = idl.events?.find((e) => e.name === name);
  if (!declared) throw new Error(`no event ${name}`);

  const body = encodeType(idl, { defined: { name } }, fields);
  const payload = Uint8Array.from([...declared.discriminator, ...body]);
  return `Program data: ${Buffer.from(payload).toString("base64")}`;
}

const POOL = PublicKey.unique();
const POSITION = PublicKey.unique();
const OWNER = PublicKey.unique();

test("the claimed amount comes out of the event, exact past 2^53", () => {
  const logs = [
    `Program ${staking.address} invoke [1]`,
    "Program log: Instruction: Claim",
    emitted(staking, "RewardsClaimed", {
      pool: POOL,
      position: POSITION,
      owner: OWNER,
      // Past the double-precision limit on purpose: this is the number the UI
      // puts in front of a user, and a rounded one would be wrong in the
      // direction of promising more than the transfer delivers.
      amount: 9_007_199_254_740_993n,
      timestamp: 1_700_000_000n,
    }),
    `Program ${staking.address} success`,
  ];

  const events = decodeEvents([staking, governance], logs);
  assert.equal(events.length, 1);
  assert.equal(events[0].name, "RewardsClaimed");
  assert.equal(events[0].program, "helix_staking");

  assert.equal(amountField(firstEvent(events, "RewardsClaimed"), "amount"), "9007199254740993");
});

test("a stake reports what the vault received, not what was sent", () => {
  // The distinction the whole protocol turns on for a fee-bearing Token-2022
  // mint: `amount_sent` leaves the wallet, `amount_credited` arrives. A preview
  // that showed the first would overstate the position by the fee.
  const logs = [
    emitted(staking, "Staked", {
      pool: POOL,
      position: POSITION,
      owner: OWNER,
      position_id: 4n,
      amount_sent: 1_000_000n,
      amount_credited: 970_000n,
      weighted_amount: 1_940_000n,
      tier: { kind: "Gold" },
      lock_end: 1_800_000_000n,
      timestamp: 1_700_000_000n,
    }),
  ];

  const staked = firstEvent(decodeEvents([staking], logs), "Staked");
  assert.equal(amountField(staked, "amount_sent"), "1000000");
  assert.equal(amountField(staked, "amount_credited"), "970000");
  assert.deepEqual(staked?.fields.tier, { kind: "Gold" });
});

test("events from both programs decode in the order they were logged", () => {
  const logs = [
    emitted(staking, "RewardsClaimed", {
      pool: POOL,
      position: POSITION,
      owner: OWNER,
      amount: 1n,
      timestamp: 1n,
    }),
    emitted(governance, "VoteCast", {
      proposal: PublicKey.unique(),
      position: POSITION,
      voter: OWNER,
      choice: { kind: "Against" },
      weight: 625_000n,
      for_votes: 0n,
      against_votes: 625_000n,
      abstain_votes: 0n,
      timestamp: 2n,
    }),
  ];

  const events = decodeEvents([staking, governance], logs);
  assert.deepEqual(
    events.map((e) => e.name),
    ["RewardsClaimed", "VoteCast"],
  );
  assert.deepEqual(events[1].fields.choice, { kind: "Against" });
  assert.equal(amountField(events[1], "weight"), "625000");
});

test("data lines from other programs are ignored rather than mistaken for events", () => {
  // Any real transaction's logs carry `Program data:` lines this app knows
  // nothing about. Treating one as a failure would make every successful claim
  // look broken.
  const logs = [
    "Program data: AAAAAAAAAAA=",
    "Program data: not-valid-base64!!!",
    "Program log: Instruction: TransferChecked",
    emitted(staking, "PositionClosed", {
      pool: POOL,
      position: POSITION,
      owner: OWNER,
      position_id: 2n,
      timestamp: 3n,
    }),
  ];

  const events = decodeEvents([staking, governance], logs);
  assert.deepEqual(
    events.map((e) => e.name),
    ["PositionClosed"],
  );
});

test("no logs is no events, not a crash", () => {
  assert.deepEqual(decodeEvents([staking], null), []);
  assert.deepEqual(decodeEvents([staking], []), []);
  assert.equal(firstEvent([], "RewardsClaimed"), undefined);
  assert.equal(amountField(undefined, "amount"), undefined);
});
