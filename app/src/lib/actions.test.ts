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
  ADVANCE,
  EXECUTION_GRACE_PERIOD,
  LOCK_TIERS,
  buildAdvance,
  buildClaim,
  buildCreateProposal,
  buildClosePosition,
  buildStake,
  buildUnstake,
  buildVote,
  poolAddress,
  positionAddress,
  whyCannotAdvance,
  whyCannotPropose,
  whyCannotVote,
  type AdvanceKind,
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

// ------------------------------------------------------------ create proposal

test("create_proposal matches the CreateProposal accounts struct", () => {
  const held = positionAddress(pool(), OWNER, 2n);
  const ix = buildCreateProposal({
    realm: realm(),
    proposerPosition: held,
    proposer: OWNER,
    action: { kind: "Signal" },
    title: "Fund the audit",
    descriptorUri: "https://example.invalid/1",
  }).instructions[0];

  // proposal.rs — realm(mut), proposer(mut, Signer), proposer_position,
  // owner, proposal(init), system_program.
  assert.deepEqual(shape(ix.keys), [
    `w- ${REALM.toBase58()}`,
    `ws ${OWNER.toBase58()}`,
    `-- ${held.toBase58()}`,
    // `has_one = owner` ties this to the position, and the handler requires it
    // to equal the signer — so the only correct value is the proposer.
    `-- ${OWNER.toBase58()}`,
    `w- ${pda(
      [Buffer.from("proposal"), REALM.toBytes(), u64le(3n)],
      GOVERNANCE_PROGRAM_ID,
    ).toBase58()}`,
    `-- ${SYSTEM_PROGRAM_ID.toBase58()}`,
  ]);
});

test("the proposal id is the realm's current count, which is what the program requires", () => {
  // The same race `stake` has. Until this flow was written the program reported
  // a stale id as `MathOverflow` — the defect `stake.rs` had already corrected
  // for `position_id` and never carried across. It is `UnexpectedProposalId` now.
  for (const count of [0n, 3n, 2n ** 53n + 1n]) {
    const prepared = buildCreateProposal({
      realm: { ...realm(), proposal_count: count },
      proposerPosition: positionAddress(pool(), OWNER, 2n),
      proposer: OWNER,
      action: { kind: "Signal" },
      title: "t",
      descriptorUri: "",
    });

    assert.deepEqual([...prepared.instructions[0].data.subarray(8, 16)], [...u64le(count)]);
    assert.ok(
      prepared.instructions[0].keys[4].pubkey.equals(
        pda([Buffer.from("proposal"), REALM.toBytes(), u64le(count)], GOVERNANCE_PROGRAM_ID),
      ),
    );
  }
});

test("the title and link are borsh strings, length-prefixed", () => {
  const ix = buildCreateProposal({
    realm: realm(),
    proposerPosition: positionAddress(pool(), OWNER, 2n),
    proposer: OWNER,
    action: { kind: "Signal" },
    title: "abc",
    descriptorUri: "de",
  }).instructions[0];

  assert.deepEqual(
    [...ix.data.subarray(8)],
    [
      // proposal_id: u64 = 3
      3, 0, 0, 0, 0, 0, 0, 0,
      // action: Signal, the first variant
      0,
      // title: 4-byte length then utf-8
      3, 0, 0, 0, 0x61, 0x62, 0x63,
      // descriptor_uri
      2, 0, 0, 0, 0x64, 0x65,
    ],
  );
});

test("the proposal gates are counted in bytes, as the program counts them", () => {
  const r = realm();
  const enough = position({ weighted_amount: r.min_weight_to_propose });
  const short = position({ weighted_amount: r.min_weight_to_propose - 1n });

  assert.equal(whyCannotPropose(r, enough, "A title", ""), null);
  assert.match(whyCannotPropose(r, short, "A title", "") ?? "", /minimum weight|requires/);
  assert.match(whyCannotPropose(r, null, "A title", "") ?? "", /Select a position/);
  assert.match(whyCannotPropose(r, enough, "   ", "") ?? "", /title is required/);

  // 64 bytes exactly is accepted; 65 is not. `title.len()` on a Rust String is a
  // byte count, so a 64-*character* title of non-ASCII text is over the limit —
  // and a UI counting characters would pass it here and fail on chain.
  assert.equal(whyCannotPropose(r, enough, "a".repeat(64), ""), null);
  assert.match(whyCannotPropose(r, enough, "a".repeat(65), "") ?? "", /65 bytes/);

  // "é" is two bytes in UTF-8, so 33 of them exceed 64 while being 33 characters.
  const accented = "é".repeat(33);
  assert.equal(accented.length, 33);
  assert.match(whyCannotPropose(r, enough, accented, "") ?? "", /66 bytes/);

  assert.match(whyCannotPropose(r, enough, "t", "u".repeat(201)) ?? "", /201 bytes/);
});

// --------------------------------------------------------- lifecycle moves

test("the permissionless transitions take no signer at all", () => {
  // `lifecycle.rs` is explicit that these are permissionless: the outcome of each
  // is a pure function of state already on chain, and permissioning finalisation
  // would let whoever held it strand a proposal in Voting forever. An account
  // list with a signer in it would mean that had stopped being true.
  const kinds: AdvanceKind[] = [
    "activate_proposal",
    "finalize_proposal",
    "queue_proposal",
    "execute_signal",
  ];

  for (const kind of kinds) {
    const ix = buildAdvance({
      realm: realm(),
      proposal: proposal(),
      payer: OWNER,
      stakingPool: POOL,
      kind,
    }).instructions[0];

    assert.ok(ix.programId.equals(GOVERNANCE_PROGRAM_ID));
    assert.equal(
      ix.keys.filter((k) => k.isSigner).length,
      0,
      `${kind} requires a signature it should not`,
    );
    // Only the proposal is written. A transition that wrote the realm would be
    // changing the rules while applying them.
    assert.deepEqual(
      ix.keys.filter((k) => k.isWritable).length,
      1,
      `${kind} writes more than the proposal`,
    );
    assert.equal(ix.data.length, 8, `${kind} takes arguments`);
  }
});

test("activate is the only transition that needs the pool", () => {
  // It snapshots `total_weighted` and `position_count` — the quorum denominator
  // and the electorate boundary. The other three read nothing outside the realm.
  const activate = buildAdvance({
    realm: realm(),
    proposal: proposal({ state: { kind: "Draft" } }),
    payer: OWNER,
    stakingPool: POOL,
    kind: "activate_proposal",
  }).instructions[0];
  assert.equal(activate.keys.length, 3);
  assert.ok(activate.keys[2].pubkey.equals(POOL));

  const finalize = buildAdvance({
    realm: realm(),
    proposal: proposal(),
    payer: OWNER,
    kind: "finalize_proposal",
  }).instructions[0];
  assert.equal(finalize.keys.length, 2);

  // And building an activation without it is refused rather than sending a
  // transaction missing an account.
  assert.throws(
    () =>
      buildAdvance({
        realm: realm(),
        proposal: proposal(),
        payer: OWNER,
        kind: "activate_proposal",
      }),
    /needs the realm's staking pool/,
  );
});

test("each transition is offered only from the state the program accepts", () => {
  const now = 1_800_000_000n;
  const states = ["Draft", "Voting", "Succeeded", "Queued"] as const;
  const expected: Record<AdvanceKind, string> = {
    activate_proposal: "Draft",
    finalize_proposal: "Voting",
    queue_proposal: "Succeeded",
    execute_signal: "Queued",
  };

  for (const [kind, from] of Object.entries(expected) as [AdvanceKind, string][]) {
    for (const state of states) {
      // Clock conditions satisfied, so state is the only variable.
      const p = proposal({
        state: { kind: state },
        voting_ends_at: now - 1n,
        eta: now - 100n,
      });
      const reason = whyCannotAdvance(p, kind, now);

      if (state === from) {
        assert.equal(reason, null, `${kind} should be possible from ${state}`);
      } else {
        assert.match(reason ?? "", /only possible while the proposal is/);
      }
    }
  }
});

test("finalize waits for the voting window, and the boundary is the program's", () => {
  // `finalize_proposal` requires `now >= voting_ends_at`, inclusive.
  const open = proposal({ voting_ends_at: 1_700_259_200n });
  assert.match(
    whyCannotAdvance(open, "finalize_proposal", 1_700_259_199n) ?? "",
    /still open/,
  );
  assert.equal(whyCannotAdvance(open, "finalize_proposal", 1_700_259_200n), null);
});

test("execution is refused before the timelock and after the grace period", () => {
  // `authorize_execution` requires `now >= eta` and `now <= eta + 14 days`.
  const queued = proposal({ state: { kind: "Queued" }, eta: 1_800_000_000n });

  assert.match(whyCannotAdvance(queued, "execute_signal", 1_799_999_999n) ?? "", /not elapsed/);
  assert.equal(whyCannotAdvance(queued, "execute_signal", 1_800_000_000n), null);

  const expiry = 1_800_000_000n + EXECUTION_GRACE_PERIOD;
  assert.equal(whyCannotAdvance(queued, "execute_signal", expiry), null);
  assert.match(whyCannotAdvance(queued, "execute_signal", expiry + 1n) ?? "", /expired/);

  // The grace period is the program's constant, not a guess.
  assert.equal(EXECUTION_GRACE_PERIOD, 14n * 86_400n);
});

test("only a Signal proposal is executable from here, and the rest say why", () => {
  // The other fourteen `execute_*` instructions each name a treasury, a mint or a
  // minter the *vote* chose. A generic button would mean the UI picking accounts
  // a proposal already decided.
  const transfer = proposal({
    state: { kind: "Queued" },
    eta: 1_700_000_000n,
    action: { kind: "TreasuryTransfer", destination: PublicKey.unique(), amount: 1n },
  });
  assert.match(
    whyCannotAdvance(transfer, "execute_signal", 1_800_000_000n) ?? "",
    /names accounts a vote already chose/,
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
