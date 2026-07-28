"use client";

import { useEffect, useState } from "react";

import { PoolPanel, ProposalsPanel, StakersPanel, TreasuryPanel } from "@/components/panels";
import { ClusterPicker, WalletMultiButton } from "@/components/Wallet";
import { api, BASE_URL, type Finality, type Health } from "@/lib/api";

/**
 * Addresses are typed in rather than discovered.
 *
 * Nothing is deployed, so there is no registry to read them from and no
 * plausible default to pre-fill. Inventing one would produce a dashboard that
 * looks populated and is not.
 */
function AddressInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <label className="address">
      <span className="muted small">{label}</span>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value.trim())}
        placeholder="base58 address"
        spellCheck={false}
      />
    </label>
  );
}

/**
 * The finality switch, surfaced rather than buried in a settings menu.
 *
 * The two views answer different questions. `finalized` is what the cluster will
 * not take back; `head` includes transactions a fork could still remove. A
 * dashboard that silently served `head` would show a TVL that sometimes goes
 * down for no reason the reader can see, so the choice belongs in front of them.
 */
function FinalityToggle({ value, onChange }: { value: Finality; onChange: (f: Finality) => void }) {
  return (
    <div className="toggle" role="group" aria-label="Finality">
      {(["finalized", "head"] as const).map((option) => (
        <button
          key={option}
          className={value === option ? "on" : ""}
          onClick={() => onChange(option)}
          title={
            option === "finalized"
              ? "Only what the cluster has committed to. Never revised downward."
              : "Includes transactions a fork could still take back."
          }
        >
          {option}
        </button>
      ))}
    </div>
  );
}

function IndexerStatus() {
  const [health, setHealth] = useState<Health | null | "down">(null);

  useEffect(() => {
    let live = true;
    const check = () =>
      api.health().then((r) => {
        if (live) setHealth(r.ok ? r.value : "down");
      });
    check();
    const timer = setInterval(check, 10_000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, []);

  if (health === null) return <span className="muted small">checking indexer…</span>;
  if (health === "down")
    return (
      <span className="badge warn" title={BASE_URL}>
        indexer unreachable
      </span>
    );

  return (
    <span className="muted small mono" title={BASE_URL}>
      slot {health.finalized_slot}
      {health.pending_transactions > 0 && ` · ${health.pending_transactions} pending`}
    </span>
  );
}

export default function Dashboard() {
  const [finality, setFinality] = useState<Finality>("finalized");
  const [pool, setPool] = useState("");
  const [realm, setRealm] = useState("");
  const [treasury, setTreasury] = useState("");

  return (
    <main>
      <header className="top">
        <div>
          <h1>Helix</h1>
          <p className="muted small">
            Analytics over the event stream, reconciled against the chain.
          </p>
        </div>
        <div className="controls">
          <IndexerStatus />
          <ClusterPicker />
          <WalletMultiButton />
        </div>
      </header>

      <section className="inputs">
        <AddressInput label="pool" value={pool} onChange={setPool} />
        <AddressInput label="realm" value={realm} onChange={setRealm} />
        <AddressInput label="treasury" value={treasury} onChange={setTreasury} />
        <FinalityToggle value={finality} onChange={setFinality} />
      </section>

      <div className="grid">
        <PoolPanel address={pool} finality={finality} />
        <StakersPanel address={pool} finality={finality} />
        <ProposalsPanel realm={realm} finality={finality} />
        <TreasuryPanel address={treasury} finality={finality} />
      </div>

      <footer className="muted small">
        Nothing is deployed. The indexer serves an empty projection until an ingestion
        source is wired — ROADMAP 4.1. Amounts are rendered from string base units via
        BigInt, never through a JavaScript number.
      </footer>
    </main>
  );
}
