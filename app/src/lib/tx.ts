/**
 * Simulate, then sign — in that order, always.
 *
 * A wallet prompt is the last moment a user can refuse, and it is also the
 * moment they have the least information: the popup shows a list of accounts and
 * a program id. Everything this module exists for is to put the outcome in front
 * of them *before* that prompt — what it will cost, what it will emit, and, when
 * it would fail, which of the program's own error messages it would fail with.
 *
 * ## The one property that matters
 *
 * The transaction that is simulated must be the transaction that is signed.
 * Simulating one instruction list and signing a freshly built one is how a
 * preview stops meaning anything, and it happens by accident the moment the two
 * are built in different places. So a `Prepared` holds the instruction array,
 * and both `simulate` and `send` take that same object — there is no second
 * construction to drift from the first. The only thing that differs between the
 * simulated message and the signed one is the blockhash, which is what
 * `replaceRecentBlockhash` substitutes anyway.
 *
 * ## What a green simulation does not promise
 *
 * It is a preview against a recent slot, not a guarantee. State can change in
 * between — someone else's `stake` moves `pool.position_count`, a lock expires,
 * a proposal's voting window closes. `SimulationOutcome` says which slot it
 * reflects for exactly this reason, in the same spirit as the read API naming
 * the projection an answer came from.
 */

import {
  PublicKey,
  TransactionMessage,
  VersionedTransaction,
  type Connection,
  type TransactionInstruction,
} from "@solana/web3.js";

import { decodeEvents, type DecodedEvent } from "./events.ts";
import { decodeTransactionError, programLogs, type DecodedError } from "./errors.ts";
import { GOVERNANCE, STAKING } from "./programs.ts";

/** A transaction that has been built and not yet simulated or signed. */
export interface Prepared {
  /** Shown to the user as the thing they are about to do. */
  summary: string;
  instructions: TransactionInstruction[];
  feePayer: PublicKey;
}

export interface SimulationOutcome {
  ok: boolean;
  /** Present exactly when `ok` is false. */
  error?: DecodedError;
  /** Compute units the runtime charged. `null` when the node did not report it. */
  unitsConsumed: number | null;
  /** The slot the simulation ran against, so a stale preview is identifiable. */
  slot: number;
  /** Events the program emitted — the authoritative amounts, not an estimate. */
  events: DecodedEvent[];
  /** `Program log:` lines, kept so an unexplained failure still has evidence. */
  logs: string[];
}

/** The default compute budget a transaction gets without asking for more. */
const DEFAULT_COMPUTE_UNITS = 200_000;

/**
 * Runs the transaction against the cluster without signing it.
 *
 * `sigVerify: false` with `replaceRecentBlockhash: true` is what makes this
 * possible before the wallet is involved at all: the node executes the message
 * against its current bank and never checks that anyone agreed to it.
 */
export async function simulate(
  connection: Connection,
  prepared: Prepared,
): Promise<SimulationOutcome> {
  const message = new TransactionMessage({
    payerKey: prepared.feePayer,
    // Replaced by the node. A valid-looking placeholder is required for the
    // message to compile at all; `replaceRecentBlockhash` discards it.
    recentBlockhash: PublicKey.default.toBase58(),
    instructions: prepared.instructions,
  }).compileToV0Message();

  const response = await connection.simulateTransaction(new VersionedTransaction(message), {
    sigVerify: false,
    replaceRecentBlockhash: true,
    commitment: "confirmed",
  });

  const { err, logs, unitsConsumed } = response.value;
  const events = decodeEvents([STAKING, GOVERNANCE], logs);

  if (err) {
    return {
      ok: false,
      error: decodeTransactionError(err, logs),
      unitsConsumed: unitsConsumed ?? null,
      slot: response.context.slot,
      events,
      logs: programLogs(logs),
    };
  }

  return {
    ok: true,
    unitsConsumed: unitsConsumed ?? null,
    slot: response.context.slot,
    events,
    logs: programLogs(logs),
  };
}

/** How close a successful simulation came to the default budget. */
export function computeHeadroom(units: number | null): string | null {
  if (units === null) return null;
  const share = (units / DEFAULT_COMPUTE_UNITS) * 100;
  return `${units.toLocaleString()} CU · ${share.toFixed(0)}% of the default budget`;
}

/**
 * A wallet-adapter `sendTransaction`, narrowed to what is used here.
 *
 * Typed structurally rather than imported so this module stays testable without
 * a React context: everything above is pure enough to run under `node --test`,
 * and one import of `@solana/wallet-adapter-react` would end that.
 */
export type SendTransaction = (
  transaction: VersionedTransaction,
  connection: Connection,
) => Promise<string>;

export interface SendOutcome {
  signature: string;
  /** Set when the transaction landed but the cluster reported it as failed. */
  error?: DecodedError;
}

/**
 * Signs and sends, then waits for confirmation.
 *
 * A fresh blockhash is fetched here rather than reused from the simulation,
 * because a blockhash that was current when the preview rendered may well have
 * expired while the user was reading it — and `BlockhashNotFound` after a
 * successful preview is the most confusing failure this flow can produce.
 */
export async function send(
  connection: Connection,
  sendTransaction: SendTransaction,
  prepared: Prepared,
): Promise<SendOutcome> {
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash("confirmed");

  const message = new TransactionMessage({
    payerKey: prepared.feePayer,
    recentBlockhash: blockhash,
    instructions: prepared.instructions,
  }).compileToV0Message();

  const signature = await sendTransaction(new VersionedTransaction(message), connection);

  const confirmation = await connection.confirmTransaction(
    { signature, blockhash, lastValidBlockHeight },
    "confirmed",
  );

  if (confirmation.value.err) {
    return { signature, error: decodeTransactionError(confirmation.value.err) };
  }
  return { signature };
}
