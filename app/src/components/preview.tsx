"use client";

/**
 * What stands between "I want to do this" and a wallet prompt.
 *
 * A wallet's own confirmation screen shows a program id and a list of accounts.
 * That is not enough to decide with, and the habit it teaches — approve and find
 * out — is the habit every drainer relies on. So the button that opens the
 * wallet is not reachable until a simulation has come back, and what it came
 * back with is on screen: the amounts the program itself computed, what it cost,
 * and, when it failed, the program's own sentence rather than `0x1771`.
 *
 * The failure case is deliberately not a red toast that disappears. A refusal is
 * usually the most informative thing that happens in a session — "this position
 * is still locked", "you have already voted with it" — and it stays until the
 * inputs change.
 */

import { toDisplay } from "@/lib/amount";
import { amountField, type DecodedEvent } from "@/lib/events";
import type { SimulationOutcome } from "@/lib/tx";
import { computeHeadroom } from "@/lib/tx";

/** The state a write flow can be in. Named so the UI cannot render two at once. */
export type FlowState =
  | { kind: "idle" }
  | { kind: "simulating" }
  | { kind: "previewed"; outcome: SimulationOutcome; summary: string }
  | { kind: "signing" }
  | { kind: "sent"; signature: string; summary: string }
  | { kind: "failed"; message: string; detail?: string };

/**
 * The amounts worth pulling out of an event and putting in front of someone.
 *
 * Every one of these is a figure the program computed during simulation. None is
 * recomputed here — see `events.ts` for why that matters more than it sounds.
 */
const HIGHLIGHTS: { event: string; field: string; label: string; hint?: string }[] = [
  {
    event: "Staked",
    field: "amount_credited",
    label: "Credited to the position",
    hint: "What the vault receives. On a fee-bearing mint this is less than what leaves your wallet.",
  },
  { event: "Staked", field: "weighted_amount", label: "Vote weight" },
  { event: "Unstaked", field: "amount", label: "Returned to your wallet" },
  { event: "Unstaked", field: "remaining", label: "Left in the position" },
  {
    event: "RewardsClaimed",
    field: "amount",
    label: "Rewards to be paid",
    hint: "Computed by the program during simulation, not estimated here.",
  },
  { event: "VoteCast", field: "weight", label: "Weight cast" },
];

function Highlights({ events }: { events: DecodedEvent[] }) {
  const rows = HIGHLIGHTS.flatMap((h) => {
    const event = events.find((e) => e.name === h.event);
    const value = amountField(event, h.field);
    return value === undefined ? [] : [{ ...h, value }];
  });

  if (rows.length === 0) return null;

  return (
    <dl className="highlights">
      {rows.map((row) => (
        <div key={`${row.event}.${row.field}`}>
          <dt title={row.hint}>{row.label}</dt>
          <dd title={`${row.value} base units`}>{toDisplay(row.value)}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Preview({
  state,
  onConfirm,
  onCancel,
  explorerUrl,
}: {
  state: FlowState;
  onConfirm: () => void;
  onCancel: () => void;
  explorerUrl?: (signature: string) => string;
}) {
  if (state.kind === "idle") return null;

  if (state.kind === "simulating") {
    return <p className="state muted">simulating against the cluster…</p>;
  }

  if (state.kind === "signing") {
    return <p className="state muted">waiting for your wallet, then for confirmation…</p>;
  }

  if (state.kind === "failed") {
    return (
      <div className="state warn">
        <strong>{state.message}</strong>
        {state.detail && <p className="muted mono small">{state.detail}</p>}
        <button onClick={onCancel}>Dismiss</button>
      </div>
    );
  }

  if (state.kind === "sent") {
    return (
      <div className="state ok">
        <strong>{state.summary} — confirmed.</strong>
        <p className="mono small">
          {explorerUrl ? (
            <a href={explorerUrl(state.signature)} target="_blank" rel="noreferrer">
              {state.signature}
            </a>
          ) : (
            state.signature
          )}
        </p>
        <button onClick={onCancel}>Done</button>
      </div>
    );
  }

  const { outcome, summary } = state;

  if (!outcome.ok) {
    return (
      <div className="state warn">
        <strong>This would fail.</strong>
        <p>{outcome.error?.message}</p>
        <p className="muted small">
          {outcome.error?.name && (
            <>
              <code>
                {outcome.error.program}::{outcome.error.name}
              </code>{" "}
              ({outcome.error.code}) ·{" "}
            </>
          )}
          simulated at slot {outcome.slot}
        </p>
        {/* Kept visible rather than behind a console: when the decode above
            cannot name the failure, these lines are the only evidence. */}
        {outcome.logs.length > 0 && (
          <details>
            <summary className="muted small">program logs</summary>
            <pre className="mono small">{outcome.logs.join("\n")}</pre>
          </details>
        )}
        <button onClick={onCancel}>Dismiss</button>
      </div>
    );
  }

  return (
    <div className="state preview">
      <strong>{summary}</strong>
      <Highlights events={outcome.events} />
      <p className="muted small">
        Simulated at slot {outcome.slot}
        {computeHeadroom(outcome.unitsConsumed) && ` · ${computeHeadroom(outcome.unitsConsumed)}`}
      </p>
      {/* Said plainly rather than implied by a green tick. A simulation is a
          preview against a recent slot, and state can move underneath it. */}
      <p className="muted small">
        This ran against slot {outcome.slot}, not against the slot it will land in. Another
        transaction can still change the outcome.
      </p>
      <div className="actions">
        <button className="primary" onClick={onConfirm}>
          Sign and send
        </button>
        <button onClick={onCancel}>Cancel</button>
      </div>
    </div>
  );
}
