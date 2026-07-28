"use client";

/**
 * The states a panel can be in other than "here is your data".
 *
 * These are separate components because they mean different things and must not
 * look the same. A dashboard that renders "the indexer is not running" and "this
 * pool has no stakers" as the same empty table is lying by omission: one is a
 * fact about the protocol, the other is a fact about the dashboard.
 *
 * That distinction is not hypothetical here. The API currently serves an empty
 * projection because no ingestion source is wired yet, so *every* panel is in
 * one of these states — and saying so plainly is the difference between a demo
 * and a misrepresentation.
 */

import type { ApiFailure } from "@/lib/api";

export function Loading() {
  return <p className="state muted">loading…</p>;
}

/** Genuinely nothing to show, and that is the correct answer. */
export function Empty({ what }: { what: string }) {
  return <p className="state muted">No {what} yet.</p>;
}

export function Failure({ failure, what }: { failure: ApiFailure; what: string }) {
  if (failure.kind === "unreachable") {
    return (
      <div className="state warn">
        <strong>The indexer is not answering.</strong>
        <p>
          This is the expected state until an ingestion source is wired — see ROADMAP 4.1.
          Start it with <code>cargo run -p helix-indexer --features server --bin helix-api</code>;
          it will serve an empty projection, which is still different from no answer at all.
        </p>
        <p className="muted mono">{failure.detail}</p>
      </div>
    );
  }

  if (failure.kind === "not-found") {
    return (
      <p className="state muted">
        The indexer has never seen this {what}. Either the address is wrong, or nothing it
        emitted has been ingested.
      </p>
    );
  }

  return <p className="state warn">The indexer answered {failure.status}.</p>;
}

/**
 * Shown when a figure was reconstructed from history that started mid-stream.
 *
 * The numbers are the best available rather than complete, and the read model
 * tracks the difference precisely so that the UI can admit it.
 */
export function PartialHistory() {
  return (
    <span className="badge warn" title="Reconstructed from a stream that began after this account was created">
      partial history
    </span>
  );
}
