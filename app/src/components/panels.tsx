"use client";

import { useEffect, useState } from "react";

import { bpsToPercent, shortAddress, toDisplay } from "@/lib/amount";
import {
  api,
  type ApiResult,
  type Envelope,
  type Finality,
  type PoolView,
  type ProposalView,
  type StakerView,
  type TreasuryView,
} from "@/lib/api";
import { Empty, Failure, Loading, PartialHistory } from "./states";

/** Fetches on mount and whenever the inputs change. */
function useApi<T>(fetcher: () => Promise<ApiResult<T>>, deps: unknown[]) {
  const [result, setResult] = useState<ApiResult<T> | null>(null);

  useEffect(() => {
    let live = true;
    setResult(null);
    fetcher().then((r) => {
      // A response that arrives after the inputs changed is stale, and showing
      // it would attribute one pool's numbers to another.
      if (live) setResult(r);
    });
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return result;
}

function Panel({
  title,
  meta,
  children,
}: {
  title: string;
  meta?: Envelope<unknown>["meta"];
  children: React.ReactNode;
}) {
  return (
    <section className="panel">
      <header>
        <h2>{title}</h2>
        {meta && (
          <span className="muted mono small">
            {meta.finality} · slot {meta.slot}
            {meta.pending_transactions > 0 && ` · ${meta.pending_transactions} revocable`}
          </span>
        )}
      </header>
      {children}
    </section>
  );
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="stat">
      <span className="muted small">{label}</span>
      <strong title={hint}>{value}</strong>
    </div>
  );
}

export function PoolPanel({ address, finality }: { address: string; finality: Finality }) {
  const result = useApi(() => api.pool(address, finality), [address, finality]);

  if (!address) return <Panel title="Pool"><Empty what="pool selected" /></Panel>;
  if (!result) return <Panel title="Pool"><Loading /></Panel>;
  if (!result.ok) return <Panel title="Pool"><Failure failure={result.failure} what="pool" /></Panel>;

  const pool = result.value.data;
  return (
    <Panel title="Pool" meta={result.value.meta}>
      <div className="badges">
        {pool.paused && <span className="badge warn">deposits paused</span>}
        {pool.partial_history && <PartialHistory />}
      </div>
      <div className="stats">
        <Stat label="TVL" value={toDisplay(pool.total_staked)} hint={`${pool.total_staked} base units`} />
        <Stat label="Weighted" value={toDisplay(pool.total_weighted)} />
        <Stat label="Positions" value={pool.position_count.toLocaleString()} />
        {/* Dash, not 0.00%, when the API says the figure is undefined. */}
        <Stat label="APR" value={bpsToPercent(pool.apr_bps)} />
        <Stat label="Rewards funded" value={toDisplay(pool.total_rewards_funded)} />
        <Stat label="Rewards paid" value={toDisplay(pool.total_rewards_paid)} />
      </div>
    </Panel>
  );
}

export function StakersPanel({ address, finality }: { address: string; finality: Finality }) {
  const result = useApi(() => api.stakers(address, finality), [address, finality]);

  if (!address) return null;
  if (!result) return <Panel title="Stakers"><Loading /></Panel>;
  if (!result.ok)
    return <Panel title="Stakers"><Failure failure={result.failure} what="pool" /></Panel>;

  const stakers: StakerView[] = result.value.data;
  if (stakers.length === 0)
    return <Panel title="Stakers" meta={result.value.meta}><Empty what="stakers" /></Panel>;

  return (
    <Panel title="Stakers" meta={result.value.meta}>
      <table>
        <thead>
          <tr>
            <th>Owner</th>
            <th className="right">Staked</th>
            <th className="right">Share</th>
          </tr>
        </thead>
        <tbody>
          {stakers.map((s) => (
            <tr key={s.owner}>
              <td className="mono" title={s.owner}>{shortAddress(s.owner)}</td>
              <td className="right">{toDisplay(s.staked)}</td>
              <td className="right">
                {bpsToPercent(s.share_bps)}
                <span className="bar" style={{ width: `${Math.min(s.share_bps / 100, 100)}%` }} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Panel>
  );
}

export function ProposalsPanel({ realm, finality }: { realm: string; finality: Finality }) {
  const result = useApi(() => api.proposals(realm, finality), [realm, finality]);

  if (!realm) return null;
  if (!result) return <Panel title="Proposals"><Loading /></Panel>;
  if (!result.ok)
    return <Panel title="Proposals"><Failure failure={result.failure} what="realm" /></Panel>;

  const proposals: ProposalView[] = result.value.data;
  if (proposals.length === 0)
    return <Panel title="Proposals" meta={result.value.meta}><Empty what="proposals" /></Panel>;

  return (
    <Panel title="Proposals" meta={result.value.meta}>
      <table>
        <thead>
          <tr>
            <th>#</th>
            <th>Title</th>
            <th>State</th>
            <th className="right">For</th>
            <th className="right">Against</th>
            <th className="right">Voters</th>
          </tr>
        </thead>
        <tbody>
          {proposals.map((p) => (
            <tr key={p.address}>
              <td className="mono">{p.id}</td>
              <td>{p.title}</td>
              <td><span className={`badge ${p.state.toLowerCase()}`}>{p.state}</span></td>
              <td className="right">{toDisplay(p.for_votes)}</td>
              <td className="right">{toDisplay(p.against_votes)}</td>
              {/* The electorate the quorum denominator was measured over — see
                  F-10. A tally is only meaningful against its own snapshot. */}
              <td className="right" title={`of ${p.position_count_snapshot} positions in the snapshot`}>
                {p.distinct_voters}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Panel>
  );
}

export function TreasuryPanel({ address, finality }: { address: string; finality: Finality }) {
  const result = useApi(() => api.treasury(address, finality), [address, finality]);

  if (!address) return null;
  if (!result) return <Panel title="Treasury"><Loading /></Panel>;
  if (!result.ok)
    return <Panel title="Treasury"><Failure failure={result.failure} what="treasury" /></Panel>;

  const treasury: TreasuryView = result.value.data;
  return (
    <Panel title="Treasury" meta={result.value.meta}>
      <div className="badges">{treasury.partial_history && <PartialHistory />}</div>
      <div className="stats">
        <Stat label="Balance" value={toDisplay(treasury.balance)} />
        <Stat
          label="Committed to streams"
          value={toDisplay(treasury.committed_to_streams)}
          hint="Promised to vesting and not yet claimed. Spending cannot touch it."
        />
        <Stat label="Deposited" value={toDisplay(treasury.total_deposited)} />
        <Stat label="Spent" value={toDisplay(treasury.total_spent)} />
        <Stat label="Open streams" value={String(treasury.open_streams)} />
      </div>
    </Panel>
  );
}
