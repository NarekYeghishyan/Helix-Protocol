/**
 * The account shapes the UI reads, pinned against the programs.
 *
 * `chain.ts` casts the coder's output to typed interfaces, and a cast is not a
 * check: rename `total_staked` on chain and the TypeScript still compiles while
 * every figure derived from it becomes `undefined`. Nothing in the type system
 * closes that, so the field lists are written out here from the `#[account]`
 * structs in `state.rs` and compared against what the programs now declare.
 *
 * The list being in *order* matters as much as its contents. These are borsh
 * layouts, so two swapped `u64`s decode without complaint and report each
 * other's values.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import governance from "../idl/helix_governance.ts";
import staking from "../idl/helix_staking.ts";
import { tokenAmountFrom } from "./chain.ts";
import type { Idl } from "./idl.ts";
import { typeDef } from "./idl.ts";

/** The declared field names of an account, in layout order. */
function fields(idl: Idl, name: string): string[] {
  const def = typeDef(idl, name);
  if (def.type.kind !== "struct") throw new Error(`${name} is not a struct`);
  return (def.type.fields ?? []).map((f) => f.name);
}

test("Pool is the account chain.ts thinks it is", () => {
  assert.deepEqual(fields(staking, "Pool"), [
    "authority",
    "pending_authority",
    "stake_mint",
    "reward_mint",
    "stake_vault",
    "reward_vault",
    "total_staked",
    "total_weighted",
    "reward_rate",
    "reward_period_end",
    "reward_per_token",
    "last_update_ts",
    "total_rewards_funded",
    "total_rewards_accrued",
    "total_rewards_paid",
    "position_count",
    "paused",
    "bump",
    "vault_authority_bump",
  ]);
});

test("Position is the account chain.ts thinks it is", () => {
  assert.deepEqual(fields(staking, "Position"), [
    "pool",
    "owner",
    "position_id",
    "amount",
    "weighted_amount",
    "tier",
    "lock_end",
    "reward_per_token_paid",
    "pending_rewards",
    "created_at",
    "bump",
  ]);
});

test("Proposal is the account the vote gates read", () => {
  assert.deepEqual(fields(governance, "Proposal"), [
    "realm",
    "proposer",
    "id",
    "state",
    "action",
    "title",
    "descriptor_uri",
    "created_at",
    "voting_starts_at",
    "voting_ends_at",
    "eta",
    "for_votes",
    "against_votes",
    "abstain_votes",
    "total_weight_snapshot",
    "position_count_snapshot",
    "bump",
  ]);
});

test("Realm is the account cast_vote is seeded from", () => {
  assert.deepEqual(fields(governance, "Realm"), [
    "authority",
    "guardian",
    "staking_pool",
    "quorum_bps",
    "approval_bps",
    "voting_period",
    "timelock_delay",
    "min_weight_to_propose",
    "proposal_count",
    "bump",
    "executor_bump",
  ]);
});

test("the position filter offsets follow from the field order", () => {
  // `fetchPositions` filters on `pool` at 8 and `owner` at 40. Those numbers are
  // only right while `pool` and `owner` are the first two fields — an inserted
  // field would leave the filter matching a slice of something else and quietly
  // returning nothing, which reads as "you have no positions".
  const [first, second] = fields(staking, "Position");
  assert.equal(first, "pool");
  assert.equal(second, "owner");
});

test("ProposalState carries the variant the UI gates voting on", () => {
  const def = typeDef(governance, "ProposalState");
  assert.equal(def.type.kind, "enum");
  const variants = def.type.kind === "enum" ? def.type.variants.map((v) => v.name) : [];
  assert.deepEqual(variants, [
    "Draft",
    "Voting",
    "Succeeded",
    "Defeated",
    "Queued",
    "Executed",
    "Cancelled",
  ]);
});

test("a token account's amount is read at offset 64", () => {
  // mint (32) + owner (32) + amount (8) — the same in both token programs,
  // because Token-2022 appends its extensions after the 165-byte base account.
  const data = new Uint8Array(165);
  new DataView(data.buffer).setBigUint64(64, 18_446_744_073_709_551_615n, true);
  assert.equal(tokenAmountFrom(data), 18_446_744_073_709_551_615n);

  // Token-2022 with extensions: longer, same offset.
  const extended = new Uint8Array(400);
  new DataView(extended.buffer).setBigUint64(64, 42n, true);
  assert.equal(tokenAmountFrom(extended), 42n);

  assert.throws(() => tokenAmountFrom(new Uint8Array(32)), /too short to be a token account/);
});
