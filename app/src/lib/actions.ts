/**
 * The five things a wallet can do: stake, unstake, claim, close, vote.
 *
 * Each returns a `Prepared` — instructions and a one-line summary — and nothing
 * here touches the network or a wallet. That split is what makes these testable
 * without a cluster, and it is why the assertions in `actions.test.ts` are about
 * bytes and addresses rather than about mocks.
 *
 * Two rules run through all five:
 *
 * - **Derived, then checked.** Every PDA comes from the IDL's seed description.
 *   Where the caller also supplies the address (the pool they typed in), the
 *   derivation is compared against it and a mismatch is refused. Anchor would
 *   refuse it too, one round trip later, with `ConstraintSeeds` — this says
 *   which account and why.
 * - **The token program comes from the mint.** Never a default. These programs
 *   accept either token program, and the ATA differs between them.
 */

import { PublicKey, type TransactionInstruction } from "@solana/web3.js";

import type { LockTierName, Pool, Position, Proposal, Realm } from "./chain.ts";
import {
  GOVERNANCE,
  GOVERNANCE_PROGRAM_ID,
  STAKING,
  STAKING_PROGRAM_ID,
  associatedTokenAddress,
  buildInstruction,
  createAssociatedTokenAccountIdempotentIx,
  derivePda,
} from "./programs.ts";
import { GOVERNANCE_BOUNDS } from "./proposal.ts";
import type { Prepared } from "./tx.ts";

export type VoteChoiceName = "For" | "Against" | "Abstain";

/** The lock tiers, in the order the program declares them. */
export const LOCK_TIERS: { name: LockTierName; days: number; multiplier: string }[] = [
  { name: "Flexible", days: 0, multiplier: "1.00×" },
  { name: "Bronze", days: 30, multiplier: "1.25×" },
  { name: "Silver", days: 90, multiplier: "1.50×" },
  { name: "Gold", days: 180, multiplier: "2.00×" },
];

/** Seeds every staking instruction needs, read off the pool rather than assumed. */
function poolSeeds(pool: Pool): Record<string, PublicKey> {
  return {
    "pool.stake_mint": pool.stake_mint,
    "pool.reward_mint": pool.reward_mint,
  };
}

/**
 * Refuses a pool account that is not at its own canonical address.
 *
 * The seeds come out of the account's own fields, so the derivation only differs
 * from the supplied address if that address holds a `Pool` sitting somewhere the
 * program would never put one. On chain that is a `ConstraintSeeds` failure;
 * catching it here names the account instead of returning code 2006.
 */
function assertCanonicalPool(supplied: PublicKey, pool: Pool): void {
  const derived = poolAddress(pool);
  if (!derived.equals(supplied)) {
    throw new Error(
      `${supplied.toBase58()} holds a Pool, but the canonical address for its mints is ` +
        `${derived.toBase58()}. Refusing to build a transaction against it.`,
    );
  }
}

// ------------------------------------------------------------------- staking

export interface StakeParams {
  poolAddress: PublicKey;
  pool: Pool;
  owner: PublicKey;
  /** Base units, never a display amount and never a `number`. */
  amount: bigint;
  tier: LockTierName;
  /** The token program that owns `pool.stake_mint`. */
  tokenProgram: PublicKey;
}

export function buildStake(params: StakeParams): Prepared {
  const { poolAddress, pool, owner, amount, tier, tokenProgram } = params;
  assertCanonicalPool(poolAddress, pool);

  /**
   * `position_id` must equal the pool's *current* `position_count`, and this is
   * the read that goes stale fastest in the whole app: anyone else's stake
   * advances the counter. The program refuses a stale one with its own
   * `UnexpectedPositionId` — an error that exists because the alternative was
   * reporting this race as `MathOverflow` and sending people to look at their
   * arithmetic (see `stake.rs`). Simulation catches it before the signature.
   */
  const positionId = pool.position_count;

  const instruction = buildInstruction(STAKING, STAKING_PROGRAM_ID, "stake", {
    accounts: {
      owner,
      stake_mint: pool.stake_mint,
      owner_token_account: associatedTokenAddress(owner, pool.stake_mint, tokenProgram),
      token_program: tokenProgram,
    },
    args: { position_id: positionId, amount, tier: { kind: tier } },
    seedFields: poolSeeds(pool),
  });

  return {
    summary: `Stake ${amount} base units at ${tier}`,
    instructions: [instruction],
    feePayer: owner,
  };
}

export interface PositionParams {
  poolAddress: PublicKey;
  pool: Pool;
  position: Position;
  owner: PublicKey;
  tokenProgram: PublicKey;
}

export function buildUnstake(params: PositionParams & { amount: bigint }): Prepared {
  const { poolAddress, pool, position, owner, amount, tokenProgram } = params;
  assertCanonicalPool(poolAddress, pool);

  const instruction = buildInstruction(STAKING, STAKING_PROGRAM_ID, "unstake", {
    accounts: {
      owner,
      stake_mint: pool.stake_mint,
      owner_token_account: associatedTokenAddress(owner, pool.stake_mint, tokenProgram),
      token_program: tokenProgram,
    },
    args: { amount },
    seedFields: { ...poolSeeds(pool), "position.position_id": position.position_id },
  });

  return {
    summary: `Withdraw ${amount} base units from position #${position.position_id}`,
    instructions: [instruction],
    feePayer: owner,
  };
}

/**
 * Claim, with the reward ATA created first if it is not there.
 *
 * The create is unconditional and idempotent rather than conditional on a read.
 * A staker whose reward mint differs from the stake mint has probably never held
 * the reward token, so the account genuinely does not exist — and a claim that
 * fails with `AccountNotInitialized` on an account the dashboard could have
 * created is a dead end for someone who has no other way to create it.
 */
export function buildClaim(params: PositionParams): Prepared {
  const { poolAddress, pool, position, owner, tokenProgram } = params;
  assertCanonicalPool(poolAddress, pool);

  const rewardAccount = associatedTokenAddress(owner, pool.reward_mint, tokenProgram);

  const instructions: TransactionInstruction[] = [
    createAssociatedTokenAccountIdempotentIx(owner, owner, pool.reward_mint, tokenProgram),
    buildInstruction(STAKING, STAKING_PROGRAM_ID, "claim", {
      accounts: {
        owner,
        reward_mint: pool.reward_mint,
        owner_reward_account: rewardAccount,
        token_program: tokenProgram,
      },
      seedFields: { ...poolSeeds(pool), "position.position_id": position.position_id },
    }),
  ];

  return {
    summary: `Claim rewards from position #${position.position_id}`,
    instructions,
    feePayer: owner,
  };
}

export function buildClosePosition(params: Omit<PositionParams, "tokenProgram">): Prepared {
  const { poolAddress, pool, position, owner } = params;
  assertCanonicalPool(poolAddress, pool);

  const instruction = buildInstruction(STAKING, STAKING_PROGRAM_ID, "close_position", {
    accounts: { owner },
    seedFields: { ...poolSeeds(pool), "position.position_id": position.position_id },
  });

  return {
    summary: `Close position #${position.position_id} and reclaim its rent`,
    instructions: [instruction],
    feePayer: owner,
  };
}

/** The canonical address of a pool, derived from its own mints. */
export function poolAddress(pool: Pool): PublicKey {
  return derivePda(STAKING, STAKING_PROGRAM_ID, "close_position", "pool", {
    seedFields: poolSeeds(pool),
  });
}

/** The position account address, without building a transaction for it. */
export function positionAddress(pool: Pool, owner: PublicKey, positionId: bigint): PublicKey {
  return derivePda(STAKING, STAKING_PROGRAM_ID, "close_position", "position", {
    accounts: { owner },
    seedFields: { ...poolSeeds(pool), "position.position_id": positionId },
  });
}

// ---------------------------------------------------------------- governance

export interface VoteParams {
  realm: Realm;
  proposal: Proposal;
  /** The position casting its weight, and the address it lives at. */
  position: Position;
  positionAddress: PublicKey;
  voter: PublicKey;
}

export function buildVote(params: VoteParams & { choice: VoteChoiceName }): Prepared {
  const { realm, proposal, positionAddress: position, voter, choice } = params;

  const instruction = buildInstruction(GOVERNANCE, GOVERNANCE_PROGRAM_ID, "cast_vote", {
    accounts: { voter, position },
    args: { choice: { kind: choice } },
    seedFields: {
      "realm.staking_pool": realm.staking_pool,
      "proposal.id": proposal.id,
    },
  });

  return {
    summary: `Vote ${choice} on proposal #${proposal.id}`,
    instructions: [instruction],
    feePayer: voter,
  };
}

export interface CreateProposalParams {
  realm: Realm;
  /** The position proving the proposer meets `min_weight_to_propose`. */
  proposerPosition: PublicKey;
  proposer: PublicKey;
  /** As `composeAction` builds it — `{ kind, ...fields }`. */
  action: { kind: string; [field: string]: unknown };
  title: string;
  descriptorUri: string;
}

export function buildCreateProposal(params: CreateProposalParams): Prepared {
  const { realm, proposerPosition, proposer, action, title, descriptorUri } = params;

  /**
   * The same race `stake` has: `proposal_id` must equal the realm's *current*
   * `proposal_count`, and anyone else's proposal moves it.
   *
   * The two ways to get it wrong fail differently, which matters because the
   * preview is where someone reads about it. Losing the race — submitting the id
   * another proposal just took — never reaches the program's check: the proposal
   * PDA is seeded on this id, so `init` refuses the occupied account first and
   * the failure is system-program error 0. Being *ahead* of the counter does
   * reach it, and until this flow was written it reported `MathOverflow` — the
   * defect `stake.rs` had corrected for `position_id` and never carried across.
   * It is `UnexpectedProposalId` now. `errors.ts` decodes both.
   */
  const proposalId = realm.proposal_count;

  const instruction = buildInstruction(GOVERNANCE, GOVERNANCE_PROGRAM_ID, "create_proposal", {
    accounts: {
      proposer,
      proposer_position: proposerPosition,
      // `has_one = owner` ties this to the position and the handler requires it
      // to equal the signer. Passing anything but the proposer is a transaction
      // the program refuses.
      owner: proposer,
    },
    args: {
      proposal_id: proposalId,
      action,
      title,
      descriptor_uri: descriptorUri,
    },
    seedFields: { "realm.staking_pool": realm.staking_pool },
  });

  return {
    summary: `Create proposal #${proposalId}: ${title}`,
    instructions: [instruction],
    feePayer: proposer,
  };
}

/**
 * Why a proposal cannot be created, or `null` if it can.
 *
 * The length limits are counted in **bytes**, not characters, because that is
 * what `title.len()` measures on a Rust `String`. A 64-character title of
 * non-ASCII text is longer than 64 bytes and would be refused on chain after
 * passing a `length <= 64` check here — the kind of disagreement that only shows
 * up for the users least likely to be able to explain it.
 */
export function whyCannotPropose(
  realm: Realm,
  position: Position | null,
  title: string,
  descriptorUri: string,
): string | null {
  if (!position) return "Select a position to propose with.";

  if (position.weighted_amount < realm.min_weight_to_propose) {
    return (
      `This position carries ${position.weighted_amount} of weight and the realm requires ` +
      `${realm.min_weight_to_propose} to propose. Proposing costs weight, not just rent.`
    );
  }

  if (title.trim() === "") return "A title is required.";

  const bytes = (text: string) => new TextEncoder().encode(text).length;
  if (bytes(title) > GOVERNANCE_BOUNDS.MAX_TITLE_LEN) {
    return `The title is ${bytes(title)} bytes; the program accepts ${GOVERNANCE_BOUNDS.MAX_TITLE_LEN}.`;
  }
  if (bytes(descriptorUri) > GOVERNANCE_BOUNDS.MAX_URI_LEN) {
    return `The link is ${bytes(descriptorUri)} bytes; the program accepts ${GOVERNANCE_BOUNDS.MAX_URI_LEN}.`;
  }

  return null;
}

/**
 * The transitions anyone may drive, and the instruction each maps to.
 *
 * All three are permissionless and take no signer at all — the outcome of each is
 * a pure function of state already on chain, so there is nothing to decide, only
 * to record. `lifecycle.rs` is explicit about why: making finalisation
 * permissioned would let whoever held the permission strand a proposal they
 * disliked in `Voting` forever.
 *
 * `execute_signal` is here and the other fourteen `execute_*` instructions are
 * not, because a signalling proposal moves nothing and needs no accounts beyond
 * the proposal itself. The rest each name a treasury, a mint or a minter the
 * proposal chose, and executing one from a generic button would mean the UI
 * choosing accounts a vote already decided.
 */
export const ADVANCE = {
  activate_proposal: {
    from: "Draft",
    label: "Activate",
    hint: "Opens voting and fixes the quorum denominator. Anyone may do this.",
  },
  finalize_proposal: {
    from: "Voting",
    label: "Finalize",
    hint: "Records the outcome once voting has closed.",
  },
  queue_proposal: {
    from: "Succeeded",
    label: "Queue",
    hint: "Starts the timelock. The delay is fixed at this moment and never recomputed.",
  },
  execute_signal: {
    from: "Queued",
    label: "Execute",
    hint: "Records the outcome of a signalling proposal. Moves nothing.",
  },
} as const;

export type AdvanceKind = keyof typeof ADVANCE;

export interface AdvanceParams {
  realm: Realm;
  proposal: Proposal;
  /** Pays the fee. Not a signer of the instruction — there is no signer. */
  payer: PublicKey;
  /** Required by `activate_proposal`, which snapshots the pool. */
  stakingPool?: PublicKey;
}

export function buildAdvance(params: AdvanceParams & { kind: AdvanceKind }): Prepared {
  const { realm, proposal, payer, stakingPool, kind } = params;

  const accounts: Record<string, PublicKey> = {};
  if (kind === "activate_proposal") {
    if (!stakingPool) throw new Error("activating a proposal needs the realm's staking pool");
    accounts.staking_pool = stakingPool;
  }

  const instruction = buildInstruction(GOVERNANCE, GOVERNANCE_PROGRAM_ID, kind, {
    accounts,
    seedFields: {
      "realm.staking_pool": realm.staking_pool,
      "proposal.id": proposal.id,
    },
  });

  return {
    summary: `${ADVANCE[kind].label} proposal #${proposal.id}`,
    instructions: [instruction],
    feePayer: payer,
  };
}

/** How long a queued proposal stays executable after its timelock elapses. */
export const EXECUTION_GRACE_PERIOD = 14n * 86_400n;

/**
 * Why a transition cannot be driven right now, or `null` if it can.
 *
 * Restates the program's gates so a disabled button carries a reason. Same
 * standing as `whyCannotVote`: a courtesy, never the authority — the program
 * enforces all of it and simulation is what decides.
 */
export function whyCannotAdvance(
  proposal: Proposal,
  kind: AdvanceKind,
  now: bigint,
): string | null {
  const expected = ADVANCE[kind].from;
  if (proposal.state.kind !== expected) {
    return `This is only possible while the proposal is ${expected}; it is ${proposal.state.kind}.`;
  }

  if (kind === "finalize_proposal" && now < proposal.voting_ends_at) {
    return "Voting is still open.";
  }

  if (kind === "execute_signal") {
    if (proposal.action.kind !== "Signal") {
      return (
        `This proposal's action is ${proposal.action.kind}, which names accounts a vote ` +
        `already chose. Execute it with a client that supplies them.`
      );
    }
    if (now < proposal.eta) return "The timelock has not elapsed.";
    // A queued proposal nobody executed expires, so that one passed under one
    // set of conditions cannot be executed into a different world a year later.
    if (now > proposal.eta + EXECUTION_GRACE_PERIOD) {
      return "The execution window has closed — this proposal has expired.";
    }
  }

  return null;
}

/**
 * Why a position cannot vote on a proposal, or `null` if it can.
 *
 * Both gates are the program's, restated for the UI so a button can be disabled
 * with a reason attached instead of failing at signature. They are *only* a
 * courtesy: `cast_vote` enforces both, and simulation is what actually decides.
 * The one thing this must not do is disagree with the program in the permissive
 * direction, so each condition is the same comparison the program makes rather
 * than a looser approximation of it.
 */
export function whyCannotVote(
  proposal: Proposal,
  position: Position,
  now: bigint,
): string | null {
  if (proposal.state.kind !== "Voting") {
    return `The proposal is ${proposal.state.kind}, not open for voting.`;
  }
  if (now < proposal.voting_starts_at) return "Voting has not started.";
  if (now >= proposal.voting_ends_at) return "Voting has closed.";

  // `Position::can_vote_until` — the flash-loan gate.
  if (position.lock_end < proposal.voting_ends_at) {
    return (
      `This position unlocks before the vote closes, so it carries no weight here. ` +
      `Voting needs stake locked until at least the end of the voting window.`
    );
  }
  // The electorate gate — `position_id < position_count_snapshot` (F-10).
  if (position.position_id >= proposal.position_count_snapshot) {
    return (
      `This position was opened after the proposal was activated, so it is not in ` +
      `the electorate the quorum was measured over.`
    );
  }
  if (position.weighted_amount === 0n) return "This position has no weight left.";

  return null;
}
