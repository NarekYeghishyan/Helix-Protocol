"use client";

/**
 * The one path from intent to signature.
 *
 * Every write in this app goes through `useFlow`, and it enforces the ordering
 * that `tx.ts` exists to protect: a `Prepared` is built once, simulated, held,
 * and — if the user then confirms — sent. `confirm` cannot construct anything;
 * it can only send what was previewed. That is the whole reason the prepared
 * transaction lives in a ref rather than being rebuilt on click, and it is not a
 * micro-optimisation: rebuilding would re-read `pool.position_count` and could
 * send a transaction the user never saw a preview of.
 */

import { useConnection, useWallet } from "@solana/wallet-adapter-react";
import { useCallback, useRef, useState } from "react";

import { send, simulate, type Prepared } from "@/lib/tx";
import type { FlowState } from "@/components/preview";

export interface Flow {
  state: FlowState;
  /** Builds and simulates. The builder runs inside, so a throw becomes a message. */
  preview: (build: () => Prepared) => Promise<void>;
  /** Signs and sends exactly what was previewed. */
  confirm: () => Promise<void>;
  reset: () => void;
  /** True while the wallet or the cluster is busy, for disabling inputs. */
  busy: boolean;
}

export function useFlow(onSettled?: () => void): Flow {
  const { connection } = useConnection();
  const { publicKey, sendTransaction } = useWallet();

  const [state, setState] = useState<FlowState>({ kind: "idle" });
  const prepared = useRef<Prepared | null>(null);

  const reset = useCallback(() => {
    prepared.current = null;
    setState({ kind: "idle" });
  }, []);

  const preview = useCallback(
    async (build: () => Prepared) => {
      if (!publicKey) {
        setState({ kind: "failed", message: "Connect a wallet first." });
        return;
      }

      setState({ kind: "simulating" });
      try {
        // Building can fail on its own — a non-canonical pool, a missing
        // account — and those messages are the most specific ones available.
        // They are surfaced as-is rather than replaced with "could not build".
        const next = build();
        prepared.current = next;

        const outcome = await simulate(connection, next);
        setState({ kind: "previewed", outcome, summary: next.summary });
      } catch (cause) {
        prepared.current = null;
        setState({
          kind: "failed",
          message: cause instanceof Error ? cause.message : String(cause),
        });
      }
    },
    [connection, publicKey],
  );

  const confirm = useCallback(async () => {
    const pending = prepared.current;
    if (!pending) {
      setState({ kind: "failed", message: "Nothing has been previewed." });
      return;
    }

    setState({ kind: "signing" });
    try {
      const outcome = await send(connection, sendTransaction, pending);

      if (outcome.error) {
        setState({
          kind: "failed",
          message: outcome.error.message,
          detail: `signature ${outcome.signature}`,
        });
      } else {
        setState({ kind: "sent", signature: outcome.signature, summary: pending.summary });
      }
    } catch (cause) {
      // A user closing the wallet popup lands here, and it is not an error
      // worth shouting about.
      const message = cause instanceof Error ? cause.message : String(cause);
      setState({
        kind: "failed",
        message: /user rejected|reject/i.test(message)
          ? "You declined the signature. Nothing was sent."
          : message,
      });
    } finally {
      prepared.current = null;
      onSettled?.();
    }
  }, [connection, sendTransaction, onSettled]);

  return {
    state,
    preview,
    confirm,
    reset,
    busy: state.kind === "simulating" || state.kind === "signing",
  };
}
