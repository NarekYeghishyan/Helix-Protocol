"use client";

/**
 * Voting.
 *
 * The interesting part of this panel is not the three buttons, it is the two
 * gates in front of them. `cast_vote` accepts a position only if
 *
 *   - `lock_end >= voting_ends_at` — the flash-loan gate, and
 *   - `position_id < position_count_snapshot` — the electorate gate that F-10
 *     installed after fuzzing found weight staked *after* activation voting
 *     anyway.
 *
 * Neither is guessable from a proposal list. A UI that shows a vote button for
 * every position teaches people to click it and read `0x1775`, so each position
 * is listed with the reason it can or cannot vote, taken from `whyCannotVote`,
 * which restates the program's comparisons rather than approximating them.
 *
 * Proposals are read from the chain, not from the indexer — the two fields both
 * gates turn on are not in the read API, and putting them there would make an
 * analytics service a dependency of casting a vote.
 */

import { useConnection, useWallet } from "@solana/wallet-adapter-react";
import { PublicKey } from "@solana/web3.js";
import { useCallback, useEffect, useState } from "react";

import { useFlow } from "@/components/flow";
import { Preview } from "@/components/preview";
import { ProposeForm } from "@/components/propose";
import { useExplorer } from "@/components/stake";
import {
  ADVANCE,
  EXECUTION_GRACE_PERIOD,
  buildAdvance,
  buildVote,
  whyCannotAdvance,
  whyCannotVote,
  type AdvanceKind,
  type VoteChoiceName,
} from "@/lib/actions";
import { bpsToPercent, shortAddress, toDisplay } from "@/lib/amount";
import {
  fetchPositions,
  fetchProposals,
  fetchRealm,
  type Fetched,
  type Position,
  type Proposal,
  type Realm,
} from "@/lib/chain";

const CHOICES: VoteChoiceName[] = ["For", "Against", "Abstain"];

interface RealmContext {
  address: PublicKey;
  realm: Realm;
  proposals: Fetched<Proposal>[];
  positions: Fetched<Position>[];
}

type Load<T> =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; value: T }
  | { kind: "error"; message: string };

function useRealmContext(address: string, voter: PublicKey | null) {
  const { connection } = useConnection();
  const [state, setState] = useState<Load<RealmContext>>({ kind: "idle" });
  const [nonce, setNonce] = useState(0);
  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    if (!address) {
      setState({ kind: "idle" });
      return;
    }

    let live = true;
    setState({ kind: "loading" });

    (async () => {
      let realmPk: PublicKey;
      try {
        realmPk = new PublicKey(address);
      } catch {
        throw new Error(`"${address}" is not a valid address.`);
      }

      const realm = await fetchRealm(connection, realmPk);
      if (!realm) throw new Error("No realm at this address on this cluster.");

      const [proposals, positions] = await Promise.all([
        fetchProposals(connection, realmPk),
        // Positions in the pool this realm governs — voting weight comes from
        // there and nowhere else.
        voter ? fetchPositions(connection, voter, realm.staking_pool) : Promise.resolve([]),
      ]);

      return { address: realmPk, realm, proposals, positions };
    })()
      .then((value) => live && setState({ kind: "ok", value }))
      .catch(
        (cause) =>
          live &&
          setState({
            kind: "error",
            message: cause instanceof Error ? cause.message : String(cause),
          }),
      );

    return () => {
      live = false;
    };
  }, [address, connection, voter, nonce]);

  return { state, reload };
}

function Panel({ children }: { children: React.ReactNode }) {
  return (
    <section className="panel">
      <header>
        <h2>Vote</h2>
      </header>
      {children}
    </section>
  );
}

export function VotePanel({ realmAddress }: { realmAddress: string }) {
  const { publicKey } = useWallet();
  const { state, reload } = useRealmContext(realmAddress, publicKey ?? null);

  if (!realmAddress) return <Panel><p className="state muted">Name a realm above.</p></Panel>;
  if (!publicKey) return <Panel><p className="state muted">Connect a wallet to vote.</p></Panel>;
  if (state.kind === "idle" || state.kind === "loading")
    return <Panel><p className="state muted">reading the realm…</p></Panel>;
  if (state.kind === "error") return <Panel><p className="state warn">{state.message}</p></Panel>;

  const { realm, proposals, positions } = state.value;

  return (
    <Panel>
      <p className="muted small">
        Quorum {bpsToPercent(realm.quorum_bps)} of the weight snapshot · approval{" "}
        {bpsToPercent(realm.approval_bps)} · timelock{" "}
        {(Number(realm.timelock_delay) / 3600).toFixed(0)}h
      </p>

      <ProposeForm
        realm={realm}
        positions={positions}
        proposer={publicKey}
        reload={reload}
      />

      {proposals.length === 0 && <p className="state muted">This realm has no proposals.</p>}

      {/* Every proposal, not only the ones open for voting. A governance UI that
          hides the others makes the lifecycle invisible — and three of its four
          transitions are permissionless, so a proposal stuck in Draft or
          Succeeded is something any reader can move. */}
      {proposals.map((p) => (
        <ProposalCard
          key={p.address.toBase58()}
          proposal={p}
          realm={realm}
          positions={positions}
          voter={publicKey}
          reload={reload}
        />
      ))}
    </Panel>
  );
}

function ProposalCard({
  proposal,
  realm,
  positions,
  voter,
  reload,
}: {
  proposal: Fetched<Proposal>;
  realm: Realm;
  positions: Fetched<Position>[];
  voter: PublicKey;
  reload: () => void;
}) {
  const flow = useFlow(reload);
  const explorer = useExplorer();
  const [choice, setChoice] = useState<VoteChoiceName>("For");

  const p = proposal.account;
  const now = BigInt(Math.floor(Date.now() / 1000));
  const voting = p.state.kind === "Voting";
  const closesIn = Number(p.voting_ends_at - now);

  const cast = p.for_votes + p.against_votes + p.abstain_votes;

  return (
    <div className="proposal">
      <div className="position-head">
        <span className="mono">#{p.id.toString()}</span>
        <strong>{p.title}</strong>
        <span className={`badge ${p.state.kind.toLowerCase()}`}>{p.state.kind}</span>
        <span className="badge">{p.action.kind}</span>
      </div>

      <p className="muted small">
        {voting && <>Closes {closesIn > 0 ? `in ${formatDuration(closesIn)}` : "now"} · </>}
        <span title="Fixed at activation so a whale cannot inflate the denominator after seeing the tally">
          quorum measured against {toDisplay(p.total_weight_snapshot.toString())} of weight
        </span>
      </p>

      <Timelock proposal={p} now={now} />

      <div className="tally">
        <span>For {toDisplay(p.for_votes.toString())}</span>
        <span>Against {toDisplay(p.against_votes.toString())}</span>
        <span>Abstain {toDisplay(p.abstain_votes.toString())}</span>
        <span className="muted small">
          {p.total_weight_snapshot > 0n
            ? `${((Number(cast) / Number(p.total_weight_snapshot)) * 100).toFixed(1)}% of the snapshot has voted`
            : "the snapshot is empty"}
        </span>
      </div>

      {/* The permissionless half of the lifecycle. Shown for every proposal, with
          the one transition its state allows enabled — a proposal sitting in
          `Succeeded` is not waiting for an authority, it is waiting for anyone. */}
      <div className="row">
        {(Object.keys(ADVANCE) as AdvanceKind[]).map((kind) => {
          const reason = whyCannotAdvance(p, kind, now);
          return (
            <button
              key={kind}
              onClick={() =>
                flow.preview(() =>
                  buildAdvance({
                    realm,
                    proposal: p,
                    payer: voter,
                    stakingPool: realm.staking_pool,
                    kind,
                  }),
                )
              }
              disabled={flow.busy || reason !== null}
              title={reason ?? ADVANCE[kind].hint}
            >
              {ADVANCE[kind].label}
            </button>
          );
        })}
        <span className="muted small">
          No signature of authority — these take no signer at all, only a fee payer.
        </span>
      </div>

      {voting && (
        <div className="toggle" role="group" aria-label="Vote choice">
          {CHOICES.map((c) => (
            <button
              key={c}
              className={choice === c ? "on" : ""}
              onClick={() => setChoice(c)}
              disabled={flow.busy}
            >
              {c}
            </button>
          ))}
        </div>
      )}

      {voting && positions.length === 0 && (
        <p className="state muted">
          You hold no positions in the pool this realm governs, so you have no weight here.
        </p>
      )}

      {voting &&
        positions.map((held) => {
        const reason = whyCannotVote(p, held.account, now);
        return (
          <div className="row" key={held.address.toBase58()}>
            <span className="mono">#{held.account.position_id.toString()}</span>
            <span className="muted small">
              {toDisplay(held.account.weighted_amount.toString())} weight ·{" "}
              {held.account.tier.kind}
            </span>
            <button
              onClick={() =>
                flow.preview(() =>
                  buildVote({
                    realm,
                    proposal: p,
                    position: held.account,
                    positionAddress: held.address,
                    voter,
                    choice,
                  }),
                )
              }
              disabled={flow.busy || reason !== null}
              title={reason ?? `Cast ${choice} with position #${held.account.position_id}`}
            >
              Vote {choice}
            </button>
            {/* The reason is on screen, not only in a tooltip. "Why is this
                button grey" is the question this panel exists to answer. */}
            {reason && <span className="muted small">{reason}</span>}
          </div>
          );
        })}

      {voting && (
        <p className="muted small">
          Voting twice with the same position is impossible rather than checked for: the vote
          record is a PDA of (proposal, position), so a second attempt fails at account
          creation. The preview says so before you sign.
        </p>
      )}

      <Preview
        state={flow.state}
        onConfirm={flow.confirm}
        onCancel={flow.reset}
        explorerUrl={explorer}
      />

      <p className="muted small mono" title={proposal.address.toBase58()}>
        {shortAddress(proposal.address.toBase58())}
      </p>
    </div>
  );
}

/**
 * The timelock, once a proposal is queued.
 *
 * Two deadlines, not one, and the second is the one people forget. A queued
 * proposal becomes executable at `eta` and stops being executable fourteen days
 * later — without that expiry a proposal passed under one set of conditions could
 * lie dormant and then be executed into a completely different world. A countdown
 * that showed only "executable in 6h" would present an expired proposal as
 * perpetually ready.
 *
 * `eta` is fixed when the proposal is queued and never recomputed, so lowering
 * `realm.timelock_delay` afterwards cannot shorten a delay already running. That
 * is what stops the timelock being bypassed by governing the timelock.
 */
function Timelock({ proposal, now }: { proposal: Proposal; now: bigint }) {
  if (proposal.state.kind !== "Queued") return null;

  const expiresAt = proposal.eta + EXECUTION_GRACE_PERIOD;
  const untilExecutable = Number(proposal.eta - now);
  const untilExpiry = Number(expiresAt - now);

  if (untilExpiry <= 0) {
    return (
      <p className="state warn">
        The execution window closed {formatDuration(-untilExpiry)} ago. This proposal passed
        and can no longer be executed.
      </p>
    );
  }

  if (untilExecutable > 0) {
    return (
      <p className="muted small">
        Timelock: executable in <strong>{formatDuration(untilExecutable)}</strong>, then for{" "}
        {formatDuration(Number(EXECUTION_GRACE_PERIOD))} after that.
      </p>
    );
  }

  return (
    <p className="muted small">
      Executable now · the window closes in <strong>{formatDuration(untilExpiry)}</strong>.
    </p>
  );
}

function formatDuration(seconds: number): string {
  if (seconds >= 86_400) return `${Math.floor(seconds / 86_400)}d ${Math.floor((seconds % 86_400) / 3_600)}h`;
  if (seconds >= 3_600) return `${Math.floor(seconds / 3_600)}h ${Math.floor((seconds % 3_600) / 60)}m`;
  return `${Math.floor(seconds / 60)}m`;
}
