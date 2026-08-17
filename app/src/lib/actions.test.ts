/**
 * The five wallet flows, checked against the programs rather than against the
 * IDL they were built from.
 *
 * The account lists and the seed literals below are transcribed from the
 * `#[derive(Accounts)]` structs under each program's `src/instructions`
 * directory, by hand and on purpose. That is what makes them worth running: the
 * builder
 * reads the IDL, so comparing it to the IDL would prove nothing, while comparing
 * it to a second reading of the source catches both a builder bug and an IDL
 * that stopped describing the program.
 *
 * When one of these fails after a program change, the fix is to re-read the
 * `Accounts` struct and update the list here — not to copy whatever the builder
 * now produces, which would silently ratify the change.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { PublicKey } from "@solana/web3.js";

import {
  LOCK_TIERS,
  buildClaim,
  buildClosePosition,
  buildStake,
  buildUnstake,
  buildVote,
  poolAddress,
  positionAddress,
  whyCannotVote,
} from "./actions.ts";
import type { Pool, Position, Proposal, Realm } from "./chain.ts";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  GOVERNANCE_PROGRAM_ID,
  STAKING_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  associatedTokenAddress,
  buildInstruction,
  STAKING,
} from "./programs.ts";

// ------------------------------------------------------------------ fixtures

const STAKE_MINT = PublicKey.unique();
const REWARD_MINT = PublicKey.unique();
const OWNER = PublicKey.unique();

/** PDA derivation written out with literal seeds, as `constants.rs` declares them. */
function pda(seeds: (Uint8Array | Buffer)[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

function u64le(value: bigint): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, value, true);
  return out;
}

const POOL = pda(
  [Buffer.from("pool"), STAKE_MINT.toBytes(), REWARD_MINT.toBytes()],
  STAKING_PROGRAM_ID,
);

function pool(overrides: Partial<Pool> = {}): Pool {
  return {
    authority: PublicKey.unique(),
    pending_authority: null,
    stake_mint: STAKE_MINT,
    reward_mint: REWARD_MINT,
    stake_vault: pda([Buffer.from("stake_vault"), POOL.toBytes()], STAKING_PROGRAM_ID),
    reward_vault: pda([Buffer.from("reward_vault"), POOL.toBytes()], STAKING_PROGRAM_ID),
    total_staked: 1_000_000n,
    total_weighted: 1_250_000n,
    reward_rate: 100n,
    reward_period_end: 2_000_000_000n,
    reward_per_token: 0n,
    last_update_ts: 1_700_000_000n,
    total_rewards_funded: 0n,
    total_rewards_accrued: 0n,
    total_rewards_paid: 0n,
    position_count: 4n,
    paused: false,
    bump: 255,
    vault_authority_bump: 254,
    ...overrides,
  };
}

function position(overrides: Partial<Position> = {}): Position {
  return {
    pool: POOL,
    owner: OWNER,
    position_id: 2n,
    amount: 500_000n,
    weighted_amount: 625_000n,
    tier: { kind: "Bronze" },
    lock_end: 1_800_000_000n,
    reward_per_token_paid: 0n,
    pending_rewards: 0n,
    created_at: 1_700_000_000n,
    bump: 253,
    ...overrides,
  };
}

/** Account names, in order, with `w` for writable and `s` for signer. */
function shape(keys: { pubkey: PublicKey; isSigner: boolean; isWritable: boolean }[]): string[] {
  return keys.map(
    (k) => `${k.isWritable ? "w" : "-"}${k.isSigner ? "s" : "-"} ${k.pubkey.toBase58()}`,
  );
}

// --------------------------------------------------------------------- stake

test("stake matches the Stake accounts struct, account for account", () => {
  const p = pool();
  const prepared = buildStake({
    poolAddress: POOL,
    pool: p,
    owner: OWNER,
    amount: 1_000_000n,
    tier: "Gold",
    tokenProgram: TOKEN_PROGRAM_ID,
  });

  assert.equal(prepared.instructions.length, 1);
  const ix = prepared.instructions[0];
  assert.ok(ix.programId.equals(STAKING_PROGRAM_ID));

  // programs/staking/src/instructions/stake.rs — pool(mut), owner(mut, Signer),
  // position(init), stake_mint, owner_token_account(mut), stake_vault(mut),
  // token_program, system_program.
  assert.deepEqual(shape(ix.keys), [
    `w- ${POOL.toBase58()}`,
    `ws ${OWNER.toBase58()}`,
    `w- ${pda(
      [Buffer.from("position"), POOL.toBytes(), OWNER.toBytes(), u64le(4n)],
      STAKING_PROGRAM_ID,
    ).toBase58()}`,
    `-- ${STAKE_MINT.toBase58()}`,
    `w- ${associatedTokenAddress(OWNER, STAKE_MINT, TOKEN_PROGRAM_ID).toBase58()}`,
    `w- ${pda([Buffer.from("stake_vault"), POOL.toBytes()], STAKING_PROGRAM_ID).toBase58()}`,
    `-- ${TOKEN_PROGRAM_ID.toBase58()}`,
    `-- ${SYSTEM_PROGRAM_ID.toBase58()}`,
  ]);
});

test("the position id is the pool's current count, which is what the program requires", () => {
  // `stake.rs` requires `position_id == pool.position_count` and returns
  // `UnexpectedPositionId` otherwise. Taking it from anywhere else — a local
  // counter, the number of positions the indexer knows about — is the race that
  // error was renamed to describe.
  for (const count of [0n, 4n, 2n ** 53n + 1n]) {
    const prepared = buildStake({
      poolAddress: POOL,
      pool: pool({ position_count: count }),
      owner: OWNER,
      amount: 1_000n,
      tier: "Flexible",
      tokenProgram: TOKEN_PROGRAM_ID,
    });

    const encodedId = prepared.instructions[0].data.subarray(8, 16);
    assert.deepEqual([...encodedId], [...u64le(count)]);

    // And the position PDA has to be seeded on the same id, or the program
    // creates an account at an address the client is not watching.
    const expected = pda(
      [Buffer.from("position"), POOL.toBytes(), OWNER.toBytes(), u64le(count)],
      STAKING_PROGRAM_ID,
    );
    assert.ok(prepared.instructions[0].keys[2].pubkey.equals(expected));
  }
});

test("a Token-2022 mint gets a different owner token account than a classic one", () => {
  // The whole point of `Interface<TokenInterface>`, and the failure that a
  // hardcoded token program produces at the last step of an otherwise correct
  // transaction.
  const classic = buildStake({
    poolAddress: POOL,
    pool: pool(),
    owner: OWNER,
    amount: 1_000n,
    tier: "Flexible",
    tokenProgram: TOKEN_PROGRAM_ID,
  }).instructions[0];

  const extended = buildStake({
    poolAddress: POOL,
    pool: pool(),
    owner: OWNER,
    amount: 1_000n,
    tier: "Flexible",
    tokenProgram: TOKEN_2022_PROGRAM_ID,
  }).instructions[0];

  assert.ok(!classic.keys[4].pubkey.equals(extended.keys[4].pubkey));
  assert.ok(classic.keys[6].pubkey.equals(TOKEN_PROGRAM_ID));
  assert.ok(extended.keys[6].pubkey.equals(TOKEN_2022_PROGRAM_ID));
});

test("the associated token address is the ATA program's own derivation", () => {
  assert.ok(
    associatedTokenAddress(OWNER, STAKE_MINT, TOKEN_PROGRAM_ID).equals(
      pda(
        [OWNER.toBytes(), TOKEN_PROGRAM_ID.toBytes(), STAKE_MINT.toBytes()],
        ASSOCIATED_TOKEN_PROGRAM_ID,
      ),
    ),
  );
});

// ------------------------------------------------------------------- unstake

test("unstake matches the Unstake accounts struct", () => {
  const p = pool();
  const ix = buildUnstake({
    poolAddress: POOL,
    pool: p,
    position: position(),
    owner: OWNER,
    amount: 250_000n,
    tokenProgram: TOKEN_PROGRAM_ID,
  }).instructions[0];

  // unstake.rs — note the order differs from `stake`: the vault comes *before*
  // the owner's account here and after it there. Positional encoding means a
  // client that assumed one order for both would swap source and destination.
  assert.deepEqual(shape(ix.keys), [
    `w- ${POOL.toBase58()}`,
    `ws ${OWNER.toBase58()}`,
    `w- ${positionAddress(p, OWNER, 2n).toBase58()}`,
    `-- ${STAKE_MINT.toBase58()}`,
    `w- ${pda([Buffer.from("stake_vault"), POOL.toBytes()], STAKING_PROGRAM_ID).toBase58()}`,
    `w- ${associatedTokenAddress(OWNER, STAKE_MINT, TOKEN_PROGRAM_ID).toBase58()}`,
    `-- ${pda([Buffer.from("vault_authority"), POOL.toBytes()], STAKING_PROGRAM_ID).toBase58()}`,
    `-- ${TOKEN_PROGRAM_ID.toBase58()}`,
  ]);

  assert.deepEqual([...ix.data.subarray(8)], [...u64le(250_000n)]);
});

// --------------------------------------------------------------------- claim

test("claim creates the reward account first, then matches the Claim struct", () => {
  const p = pool();
  const prepared = buildClaim({
    poolAddress: POOL,
    pool: p,
    position: position(),
    owner: OWNER,
    tokenProgram: TOKEN_PROGRAM_ID,
  });

  assert.equal(prepared.instructions.length, 2);

  const [create, claim] = prepared.instructions;
  assert.ok(create.programId.equals(ASSOCIATED_TOKEN_PROGRAM_ID));
  // `CreateIdempotent`, so a second claim in the same block does not fail on an
  // account the first one made.
  assert.deepEqual([...create.data], [1]);

  assert.deepEqual(shape(claim.keys), [
    `w- ${POOL.toBase58()}`,
    // Not writable here: `claim` takes the owner as a plain `Signer`, since the
    // rent is already paid and nothing debits them.
    `-s ${OWNER.toBase58()}`,
    `w- ${positionAddress(p, OWNER, 2n).toBase58()}`,
    `-- ${REWARD_MINT.toBase58()}`,
    `w- ${pda([Buffer.from("reward_vault"), POOL.toBytes()], STAKING_PROGRAM_ID).toBase58()}`,
    `w- ${associatedTokenAddress(OWNER, REWARD_MINT, TOKEN_PROGRAM_ID).toBase58()}`,
    `-- ${pda([Buffer.from("vault_authority"), POOL.toBytes()], STAKING_PROGRAM_ID).toBase58()}`,
    `-- ${TOKEN_PROGRAM_ID.toBase58()}`,
  ]);

  // `claim` takes no arguments — the amount is whatever the program computes.
  assert.equal(claim.data.length, 8);
});

// ------------------------------------------------------------------ close

test("close_position leaves the pool read-only, as the program declares", () => {
  const p = pool();
  const ix = buildClosePosition({
    poolAddress: POOL,
    pool: p,
    position: position(),
    owner: OWNER,
  }).instructions[0];

  // close_position.rs is explicit that `pool` is read-only: a closable position
  // holds no principal, no weight and no unpaid rewards, so there is nothing in
  // the pool's books to adjust. A writable pool here would mean that argument
  // had stopped holding.
  assert.deepEqual(shape(ix.keys), [
    `-- ${POOL.toBase58()}`,
    `ws ${OWNER.toBase58()}`,
    `w- ${positionAddress(p, OWNER, 2n).toBase58()}`,
  ]);
});

// ---------------------------------------------------------------------- vote

const REALM = pda([Buffer.from("realm"), POOL.toBytes()], GOVERNANCE_PROGRAM_ID);

function realm(): Realm {
  return {
    authority: PublicKey.unique(),
    guardian: PublicKey.unique(),
    staking_pool: POOL,
    quorum_bps: 1_000,
    approval_bps: 6_000,
    voting_period: 259_200n,
    timelock_delay: 86_400n,
    min_weight_to_propose: 1_000n,
    proposal_count: 3n,
    bump: 255,
    executor_bump: 254,
  };
}

function proposal(overrides: Partial<Proposal> = {}): Proposal {
  return {
    realm: REALM,
    proposer: PublicKey.unique(),
    id: 1n,
    state: { kind: "Voting" },
    action: { kind: "Signal" },
    title: "Fund the audit",
    descriptor_uri: "https://example.invalid/1",
    created_at: 1_700_000_000n,
    voting_starts_at: 1_700_000_000n,
    voting_ends_at: 1_700_259_200n,
    eta: 0n,
    for_votes: 0n,
    against_votes: 0n,
    abstain_votes: 0n,
    total_weight_snapshot: 1_000_000n,
    position_count_snapshot: 4n,
    bump: 252,
    ...overrides,
  };
}

test("cast_vote matches the CastVote accounts struct", () => {
  const positionPk = positionAddress(pool(), OWNER, 2n);
  const ix = buildVote({
    realm: realm(),
    proposal: proposal(),
    position: position(),
    positionAddress: positionPk,
    voter: OWNER,
    choice: "For",
  }).instructions[0];

  assert.ok(ix.programId.equals(GOVERNANCE_PROGRAM_ID));

  // vote.rs — realm, proposal(mut), voter(mut, Signer), position, vote_record(init),
  // system_program.
  assert.deepEqual(shape(ix.keys), [
    `-- ${REALM.toBase58()}`,
    `w- ${pda(
      [Buffer.from("proposal"), REALM.toBytes(), u64le(1n)],
      GOVERNANCE_PROGRAM_ID,
    ).toBase58()}`,
    `ws ${OWNER.toBase58()}`,
    `-- ${positionPk.toBase58()}`,
    `w- ${pda(
      [
        Buffer.from("vote"),
        pda([Buffer.from("proposal"), REALM.toBytes(), u64le(1n)], GOVERNANCE_PROGRAM_ID).toBytes(),
        positionPk.toBytes(),
      ],
      GOVERNANCE_PROGRAM_ID,
    ).toBase58()}`,
    `-- ${SYSTEM_PROGRAM_ID.toBase58()}`,
  ]);

  // The choice is the last byte: For = 0.
  assert.deepEqual([...ix.data.subarray(8)], [0]);
});

test("the vote record is seeded per (proposal, position), which is what blocks double voting", () => {
  const first = positionAddress(pool(), OWNER, 1n);
  const second = positionAddress(pool(), OWNER, 2n);

  const record = (positionPk: PublicKey) =>
    buildVote({
      realm: realm(),
      proposal: proposal(),
      position: position(),
      positionAddress: positionPk,
      voter: OWNER,
      choice: "For",
    }).instructions[0].keys[4].pubkey;

  // Two positions vote independently; the same position twice collides at the
  // same address, which is why `init` refuses it in the runtime.
  assert.ok(!record(first).equals(record(second)));
  assert.ok(record(first).equals(record(first)));
});

// ----------------------------------------------------------- the vote gates

test("the vote gates refuse exactly what the program refuses", () => {
  const now = 1_700_100_000n;
  const open = proposal();

  // A position locked past the close and inside the snapshot may vote.
  assert.equal(whyCannotVote(open, position({ lock_end: 1_800_000_000n }), now), null);

  // The flash-loan gate: `lock_end >= voting_ends_at`, and the boundary is
  // inclusive on the program's side too.
  assert.equal(whyCannotVote(open, position({ lock_end: open.voting_ends_at }), now), null);
  assert.match(
    whyCannotVote(open, position({ lock_end: open.voting_ends_at - 1n }), now) ?? "",
    /unlocks before the vote closes/,
  );

  // The electorate gate (F-10): `position_id < position_count_snapshot`.
  assert.equal(whyCannotVote(open, position({ position_id: 3n }), now), null);
  assert.match(
    whyCannotVote(open, position({ position_id: 4n }), now) ?? "",
    /opened after the proposal was activated/,
  );

  // Window and state.
  assert.match(whyCannotVote(open, position(), 1_700_259_200n) ?? "", /Voting has closed/);
  assert.match(whyCannotVote(open, position(), 1_699_999_999n) ?? "", /has not started/);
  assert.match(
    whyCannotVote(proposal({ state: { kind: "Queued" } }), position(), now) ?? "",
    /is Queued, not open for voting/,
  );
});

// --------------------------------------------------------------- safeguards

test("a Pool at a non-canonical address is refused before anything is signed", () => {
  // Anchor would refuse it too, with `ConstraintSeeds` — code 2006, one round
  // trip later, naming nothing.
  assert.throws(
    () =>
      buildStake({
        poolAddress: PublicKey.unique(),
        pool: pool(),
        owner: OWNER,
        amount: 1_000n,
        tier: "Flexible",
        tokenProgram: TOKEN_PROGRAM_ID,
      }),
    /canonical address for its mints/,
  );
});

test("a PDA the IDL can derive cannot be overridden by the caller", () => {
  // The property that stops a look-alike vault being passed in.
  assert.throws(
    () =>
      buildInstruction(STAKING, STAKING_PROGRAM_ID, "unstake", {
        accounts: {
          owner: OWNER,
          stake_mint: STAKE_MINT,
          owner_token_account: PublicKey.unique(),
          token_program: TOKEN_PROGRAM_ID,
          stake_vault: PublicKey.unique(),
        },
        args: { amount: 1n },
        seedFields: {
          "pool.stake_mint": STAKE_MINT,
          "pool.reward_mint": REWARD_MINT,
          "position.position_id": 2n,
        },
      }),
    /derived, not passed/,
  );
});

test("a PDA seeded on another account's field will not be guessed at", () => {
  assert.throws(
    () =>
      buildInstruction(STAKING, STAKING_PROGRAM_ID, "unstake", {
        accounts: { owner: OWNER },
        args: { amount: 1n },
      }),
    /seeded on "pool.stake_mint"/,
  );
});

test("the pool address derives from its own mints", () => {
  assert.ok(poolAddress(pool()).equals(POOL));
});

test("the tier table matches the program's variant order and durations", () => {
  // `LockTier` in programs/staking/src/state.rs: Flexible 0d/1.00x, Bronze
  // 30d/1.25x, Silver 90d/1.50x, Gold 180d/2.00x.
  assert.deepEqual(
    LOCK_TIERS.map((t) => [t.name, t.days, t.multiplier]),
    [
      ["Flexible", 0, "1.00×"],
      ["Bronze", 30, "1.25×"],
      ["Silver", 90, "1.50×"],
      ["Gold", 180, "2.00×"],
    ],
  );

  // And the order is the encoding, so this list cannot be reordered for display
  // without changing what gets signed.
  const encoded = LOCK_TIERS.map(
    (t) =>
      buildStake({
        poolAddress: POOL,
        pool: pool(),
        owner: OWNER,
        amount: 1_000n,
        tier: t.name,
        tokenProgram: TOKEN_PROGRAM_ID,
      }).instructions[0].data.at(-1),
  );
  assert.deepEqual(encoded, [0, 1, 2, 3]);
});
