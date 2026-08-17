/**
 * Reading protocol state straight from the cluster.
 *
 * Deliberately not through the indexer, even though the indexer already serves
 * pools and proposals. Two reasons, and the second is the important one:
 *
 * - **The write path must work when the indexer does not.** A staker who cannot
 *   unstake because an analytics service is down has been given a worse product
 *   than no dashboard at all. Everything on this path — the pool's mints, the
 *   caller's positions, the proposal's voting window — comes from the RPC the
 *   wallet is already connected to.
 * - **Signing needs the authoritative value, not the projected one.** The
 *   indexer's `head` view is explicitly revisable and its `finalized` view is
 *   explicitly behind. `pool.position_count` is a seed for the account `stake`
 *   creates, so reading it one slot stale produces a transaction that fails on
 *   an account collision. The API is right to answer with uncertainty attached;
 *   this is the one caller that cannot use an answer like that.
 *
 * Field names are the program's own, in snake_case, because these objects come
 * out of the IDL coder with the IDL's names. Renaming them here would be a
 * hand-written mapping that compiles after a field is renamed on chain and
 * yields `undefined` at runtime — `chain.test.ts` pins the names against the
 * IDL instead.
 */

import { PublicKey, type Connection, type GetProgramAccountsFilter } from "@solana/web3.js";
import { Buffer } from "buffer";

import { decodeAccount } from "./coder.ts";
import { accountDiscriminator } from "./idl.ts";
import {
  GOVERNANCE,
  STAKING,
  STAKING_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "./programs.ts";

// ------------------------------------------------------------------- shapes

export type LockTierName = "Flexible" | "Bronze" | "Silver" | "Gold";

export interface Pool {
  authority: PublicKey;
  pending_authority: PublicKey | null;
  stake_mint: PublicKey;
  reward_mint: PublicKey;
  stake_vault: PublicKey;
  reward_vault: PublicKey;
  total_staked: bigint;
  total_weighted: bigint;
  reward_rate: bigint;
  reward_period_end: bigint;
  reward_per_token: bigint;
  last_update_ts: bigint;
  total_rewards_funded: bigint;
  total_rewards_accrued: bigint;
  total_rewards_paid: bigint;
  position_count: bigint;
  paused: boolean;
  bump: number;
  vault_authority_bump: number;
}

export interface Position {
  pool: PublicKey;
  owner: PublicKey;
  position_id: bigint;
  amount: bigint;
  weighted_amount: bigint;
  tier: { kind: LockTierName };
  lock_end: bigint;
  reward_per_token_paid: bigint;
  pending_rewards: bigint;
  created_at: bigint;
  bump: number;
}

export interface Proposal {
  realm: PublicKey;
  proposer: PublicKey;
  id: bigint;
  state: { kind: string };
  action: { kind: string; [field: string]: unknown };
  title: string;
  descriptor_uri: string;
  created_at: bigint;
  voting_starts_at: bigint;
  voting_ends_at: bigint;
  eta: bigint;
  for_votes: bigint;
  against_votes: bigint;
  abstain_votes: bigint;
  total_weight_snapshot: bigint;
  position_count_snapshot: bigint;
  bump: number;
}

export interface Realm {
  authority: PublicKey;
  guardian: PublicKey;
  staking_pool: PublicKey;
  quorum_bps: number;
  approval_bps: number;
  voting_period: bigint;
  timelock_delay: bigint;
  min_weight_to_propose: bigint;
  proposal_count: bigint;
  bump: number;
  executor_bump: number;
}

/** An account together with the address it was read from. */
export interface Fetched<T> {
  address: PublicKey;
  account: T;
}

// -------------------------------------------------------------------- reads

/**
 * `null` when the account does not exist, throwing when it exists and is
 * something else.
 *
 * The distinction is the one `states.tsx` already makes for the API: "there is
 * nothing here" and "there is something here and it is not what you named" are
 * different facts, and collapsing them sends a user to check the wrong thing.
 */
async function fetchAccount<T>(
  connection: Connection,
  idl: typeof STAKING,
  type: string,
  address: PublicKey,
): Promise<T | null> {
  const info = await connection.getAccountInfo(address, "confirmed");
  if (!info) return null;

  if (!info.owner.equals(new PublicKey(idl.address))) {
    throw new Error(
      `${address.toBase58()} is owned by ${info.owner.toBase58()}, not ${idl.metadata.name}. ` +
        `Wrong address, or wrong cluster.`,
    );
  }

  return decodeAccount<T>(idl, type, info.data);
}

export function fetchPool(connection: Connection, address: PublicKey): Promise<Pool | null> {
  return fetchAccount<Pool>(connection, STAKING, "Pool", address);
}

export function fetchProposal(
  connection: Connection,
  address: PublicKey,
): Promise<Proposal | null> {
  return fetchAccount<Proposal>(connection, GOVERNANCE, "Proposal", address);
}

export function fetchRealm(connection: Connection, address: PublicKey): Promise<Realm | null> {
  return fetchAccount<Realm>(connection, GOVERNANCE, "Realm", address);
}

/**
 * Every position `owner` holds in `pool`.
 *
 * Filtered server-side on the discriminator and on the two pubkeys at their
 * fixed offsets, rather than fetched-then-filtered: `Position` is a fixed-layout
 * account so the offsets are exact, and a public RPC will refuse an unfiltered
 * `getProgramAccounts` over a program with many accounts. The offsets are
 * derived from the IDL field order below rather than typed in.
 */
export async function fetchPositions(
  connection: Connection,
  owner: PublicKey,
  pool?: PublicKey,
): Promise<Fetched<Position>[]> {
  const filters: GetProgramAccountsFilter[] = [
    {
      memcmp: {
        offset: 0,
        encoding: "base64",
        bytes: Buffer.from(accountDiscriminator(STAKING, "Position")).toString("base64"),
      },
    },
    // `pool` is the first field, `owner` the second — 8 bytes of discriminator
    // ahead of them, 32 bytes each.
    { memcmp: { offset: 8 + 32, bytes: owner.toBase58() } },
  ];
  if (pool) filters.push({ memcmp: { offset: 8, bytes: pool.toBase58() } });

  const found = await connection.getProgramAccounts(STAKING_PROGRAM_ID, {
    commitment: "confirmed",
    filters,
  });

  return found
    .map(({ pubkey, account }) => ({
      address: pubkey,
      account: decodeAccount<Position>(STAKING, "Position", account.data),
    }))
    .sort((a, b) => Number(a.account.position_id - b.account.position_id));
}

/**
 * Every proposal in a realm, read from the chain.
 *
 * The indexer already serves proposals and serves them with more context, but
 * voting reads `voting_ends_at` and `position_count_snapshot` — the two fields
 * both eligibility gates turn on — and the read API does not carry either. It
 * could be extended to; it should not be, because that would put the indexer
 * back on the critical path of a transaction the wallet can build without it.
 */
export async function fetchProposals(
  connection: Connection,
  realm: PublicKey,
): Promise<Fetched<Proposal>[]> {
  const found = await connection.getProgramAccounts(new PublicKey(GOVERNANCE.address), {
    commitment: "confirmed",
    filters: [
      {
        memcmp: {
          offset: 0,
          encoding: "base64",
          bytes: Buffer.from(accountDiscriminator(GOVERNANCE, "Proposal")).toString("base64"),
        },
      },
      // `realm` is the first field of `Proposal`.
      { memcmp: { offset: 8, bytes: realm.toBase58() } },
    ],
  });

  return found
    .map(({ pubkey, account }) => ({
      address: pubkey,
      account: decodeAccount<Proposal>(GOVERNANCE, "Proposal", account.data),
    }))
    .sort((a, b) => Number(b.account.id - a.account.id));
}

/**
 * Which token program owns a mint.
 *
 * These programs take `Interface<TokenInterface>`, so the mint decides whether
 * this is Token or Token-2022 — and the ATA for the same mint differs between
 * them. Assuming the classic program is the mistake that makes a fee-bearing
 * Token-2022 mint fail at the last step, which is the mint this protocol was
 * specifically built to handle correctly (`ROADMAP` 2.3).
 */
export async function tokenProgramForMint(
  connection: Connection,
  mint: PublicKey,
): Promise<PublicKey> {
  const info = await connection.getAccountInfo(mint, "confirmed");
  if (!info) throw new Error(`mint ${mint.toBase58()} does not exist on this cluster`);

  if (info.owner.equals(TOKEN_PROGRAM_ID) || info.owner.equals(TOKEN_2022_PROGRAM_ID)) {
    return info.owner;
  }
  throw new Error(`${mint.toBase58()} is not a token mint — it is owned by ${info.owner.toBase58()}`);
}

/**
 * A token account's `amount`, read at its fixed offset.
 *
 * `mint` and `owner` are two pubkeys, then `amount` — offset 64 in the SPL Token
 * layout, and the same in Token-2022, whose extensions are appended *after* the
 * 165-byte base account rather than woven into it. One offset therefore serves
 * both, which is the only reason reading it directly is defensible instead of
 * pulling in a token library for one `u64`.
 */
export function tokenAmountFrom(data: Uint8Array): bigint {
  if (data.length < 72) {
    throw new Error(`${data.length} bytes is too short to be a token account`);
  }
  const view = new DataView(data.buffer, data.byteOffset + 64, 8);
  return view.getBigUint64(0, true);
}

/** A token account's `amount`, or `null` when the account does not exist yet. */
export async function tokenBalance(
  connection: Connection,
  address: PublicKey,
): Promise<bigint | null> {
  const info = await connection.getAccountInfo(address, "confirmed");
  if (!info) return null;
  return tokenAmountFrom(info.data);
}
