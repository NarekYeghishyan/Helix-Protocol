/**
 * Typed client for the Helix read API.
 *
 * The shapes here mirror `indexer/src/api.rs`. They are hand-written rather than
 * generated, and the trade is stated so it can be revisited: generation would
 * make drift impossible, at the cost of a codegen step in a repo that currently
 * has none. What keeps them honest meanwhile is that every field is used — an
 * unused field is one nobody would notice going stale.
 *
 * Note what is **not** here: no `number` for any amount. See `amount.ts`.
 */

export type Finality = "finalized" | "head";

/** Which projection an answer came from, and as of when. */
export interface Meta {
  finality: Finality;
  /** Last finalised slot folded in. */
  slot: number;
  /** How many transactions in this answer a fork could still take back. */
  pending_transactions: number;
}

export interface Envelope<T> {
  meta: Meta;
  data: T;
}

export interface PoolView {
  address: string;
  total_staked: string;
  total_weighted: string;
  position_count: number;
  reward_rate: string;
  reward_period_end: number;
  total_rewards_funded: string;
  total_rewards_paid: string;
  /** `null` when nothing is staked: APR is then undefined, not zero. */
  apr_bps: number | null;
  paused: boolean;
  /** The pool was first seen mid-history, so these figures are incomplete. */
  partial_history: boolean;
}

export interface StakerView {
  owner: string;
  staked: string;
  share_bps: number;
}

export interface ProposalView {
  address: string;
  realm: string;
  id: number;
  proposer: string;
  title: string;
  state: string;
  for_votes: string;
  against_votes: string;
  abstain_votes: string;
  total_weight_snapshot: string;
  position_count_snapshot: number;
  distinct_voters: number;
  eta: number | null;
}

export interface TreasuryView {
  address: string;
  governance_executor: string | null;
  total_deposited: string;
  total_spent: string;
  total_stream_claims: string;
  balance: string;
  committed_to_streams: string;
  epoch_spend_cap: string;
  open_streams: number;
  partial_history: boolean;
}

export interface Health {
  finalized_slot: number;
  pending_transactions: number;
}

/**
 * Why a request produced nothing.
 *
 * Distinguished because they mean different things to a reader and should look
 * different on screen. "The indexer is not running" is not "this pool has no
 * stakers", and a dashboard that renders both as an empty table is lying by
 * omission — see `EmptyState` in the UI.
 */
export type ApiFailure =
  | { kind: "unreachable"; detail: string }
  | { kind: "not-found" }
  | { kind: "error"; status: number };

export type ApiResult<T> = { ok: true; value: T } | { ok: false; failure: ApiFailure };

const BASE_URL = process.env.NEXT_PUBLIC_HELIX_API ?? "http://127.0.0.1:8080";

const query = (finality: Finality) => (finality === "head" ? "?finality=head" : "");

/**
 * Built against a base URL rather than reading one from the module scope, so a
 * test can point it at a stub server. The alternative — monkey-patching global
 * `fetch` — tests the patch as much as the client.
 */
export function createClient(baseUrl: string) {
  async function get<T>(path: string): Promise<ApiResult<T>> {
    let response: Response;
    try {
      response = await fetch(`${baseUrl}${path}`, { cache: "no-store" });
    } catch (cause) {
      // The API being down is the expected state right now, not an exception,
      // and the UI renders it as its own thing rather than as an empty table.
      return {
        ok: false,
        failure: {
          kind: "unreachable",
          detail: cause instanceof Error ? cause.message : String(cause),
        },
      };
    }

    if (response.status === 404) return { ok: false, failure: { kind: "not-found" } };
    if (!response.ok) return { ok: false, failure: { kind: "error", status: response.status } };

    return { ok: true, value: (await response.json()) as T };
  }

  return {
    health: () => get<Health>("/health"),

    pool: (address: string, finality: Finality) =>
      get<Envelope<PoolView>>(`/pools/${address}${query(finality)}`),

    stakers: (address: string, finality: Finality, limit = 20) =>
      get<Envelope<StakerView[]>>(
        `/pools/${address}/stakers?limit=${limit}${finality === "head" ? "&finality=head" : ""}`,
      ),

    proposals: (realm: string, finality: Finality) =>
      get<Envelope<ProposalView[]>>(`/realms/${realm}/proposals${query(finality)}`),

    treasury: (address: string, finality: Finality) =>
      get<Envelope<TreasuryView>>(`/treasuries/${address}${query(finality)}`),
  };
}

export const api = createClient(BASE_URL);

export { BASE_URL };
