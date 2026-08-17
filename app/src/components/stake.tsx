"use client";

/**
 * The staking write flows: open a position, withdraw, claim, close.
 *
 * Everything on screen is read from the chain rather than from the indexer. That
 * is a deliberate split from the analytics panels next to it — see the note at
 * the top of `chain.ts` — and it means these controls keep working when the
 * indexer does not.
 */

import { useConnection, useWallet } from "@solana/wallet-adapter-react";
import { PublicKey } from "@solana/web3.js";
import { useCallback, useEffect, useState } from "react";

import { Preview } from "@/components/preview";
import { useFlow } from "@/components/flow";
import { shortAddress, toDisplay } from "@/lib/amount";
import {
  LOCK_TIERS,
  buildClaim,
  buildClosePosition,
  buildStake,
  buildUnstake,
} from "@/lib/actions";
import type { LockTierName } from "@/lib/chain";
import {
  fetchPool,
  fetchPositions,
  tokenBalance,
  tokenProgramForMint,
  type Fetched,
  type Pool,
  type Position,
} from "@/lib/chain";
import { associatedTokenAddress } from "@/lib/programs";
import { useCluster } from "@/components/Wallet";

/** Everything a staking transaction needs, read once and shared by the panels. */
interface PoolContext {
  address: PublicKey;
  pool: Pool;
  stakeTokenProgram: PublicKey;
  rewardTokenProgram: PublicKey;
  /** The connected wallet's balance of the stake mint, `null` if no ATA yet. */
  walletBalance: bigint | null;
}

type Load<T> = { kind: "idle" } | { kind: "loading" } | { kind: "ok"; value: T } | { kind: "error"; message: string };

/**
 * Reads the pool and the caller's positions.
 *
 * `reload` is exposed and called after every confirmed transaction, because
 * `pool.position_count` is a seed for the next `stake` and a stale one produces
 * `UnexpectedPositionId`. Refreshing after a write is not cosmetic here.
 */
function usePoolContext(address: string, owner: PublicKey | null) {
  const { connection } = useConnection();
  const [context, setContext] = useState<Load<PoolContext>>({ kind: "idle" });
  const [positions, setPositions] = useState<Fetched<Position>[]>([]);
  const [nonce, setNonce] = useState(0);

  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    if (!address) {
      setContext({ kind: "idle" });
      setPositions([]);
      return;
    }

    let live = true;
    setContext({ kind: "loading" });

    (async () => {
      let poolPk: PublicKey;
      try {
        poolPk = new PublicKey(address);
      } catch {
        throw new Error(`"${address}" is not a valid address.`);
      }

      const pool = await fetchPool(connection, poolPk);
      if (!pool) throw new Error("No account at this address on this cluster.");

      const [stakeTokenProgram, rewardTokenProgram] = await Promise.all([
        tokenProgramForMint(connection, pool.stake_mint),
        tokenProgramForMint(connection, pool.reward_mint),
      ]);

      const walletBalance = owner
        ? await tokenBalance(
            connection,
            associatedTokenAddress(owner, pool.stake_mint, stakeTokenProgram),
          )
        : null;

      const mine = owner ? await fetchPositions(connection, owner, poolPk) : [];

      return { context: { address: poolPk, pool, stakeTokenProgram, rewardTokenProgram, walletBalance }, mine };
    })()
      .then(({ context: value, mine }) => {
        if (!live) return;
        setContext({ kind: "ok", value });
        setPositions(mine);
      })
      .catch((cause) => {
        if (!live) return;
        setContext({
          kind: "error",
          message: cause instanceof Error ? cause.message : String(cause),
        });
        setPositions([]);
      });

    return () => {
      live = false;
    };
  }, [address, connection, owner, nonce]);

  return { context, positions, reload };
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="panel">
      <header>
        <h2>{title}</h2>
      </header>
      {children}
    </section>
  );
}

/** A wallet is required for every control here, and saying so beats a dead button. */
function NeedsWallet() {
  return <p className="state muted">Connect a wallet to stake, claim or vote.</p>;
}

// --------------------------------------------------------------------- stake

export function StakePanel({ poolAddress }: { poolAddress: string }) {
  const { publicKey } = useWallet();
  const { context, positions, reload } = usePoolContext(poolAddress, publicKey ?? null);
  const flow = useFlow(reload);

  const [amount, setAmount] = useState("");
  const [tier, setTier] = useState<LockTierName>("Flexible");
  // Before the early returns below: a hook reached conditionally is a hook
  // whose position in the render order changes.
  const explorer = useExplorer();

  if (!poolAddress) return <Panel title="Stake"><p className="state muted">Name a pool above.</p></Panel>;
  if (!publicKey) return <Panel title="Stake"><NeedsWallet /></Panel>;
  if (context.kind === "loading" || context.kind === "idle")
    return <Panel title="Stake"><p className="state muted">reading the pool…</p></Panel>;
  if (context.kind === "error")
    return (
      <Panel title="Stake">
        <p className="state warn">{context.message}</p>
      </Panel>
    );

  const { pool, address, stakeTokenProgram, walletBalance } = context.value;

  // Base units throughout. The input is base units too, and labelled as such:
  // silently multiplying by 10^decimals is how a UI sends a thousand times what
  // the user meant, and the decimals are not knowable without reading the mint.
  const parsed = /^\d+$/.test(amount) ? BigInt(amount) : null;

  const onPreview = () => {
    if (parsed === null) return;
    flow.preview(() =>
      buildStake({
        poolAddress: address,
        pool,
        owner: publicKey,
        amount: parsed,
        tier,
        tokenProgram: stakeTokenProgram,
      }),
    );
  };

  return (
    <Panel title="Stake">
      {pool.paused && (
        <p className="badge warn">
          Deposits are paused. Unstaking and claiming stay open — that is deliberate.
        </p>
      )}

      <div className="field">
        <label>
          <span className="muted small">amount (base units)</span>
          <input
            value={amount}
            onChange={(e) => setAmount(e.target.value.trim())}
            placeholder="1000000000"
            inputMode="numeric"
            spellCheck={false}
            disabled={flow.busy}
          />
        </label>
        <span className="muted small">
          {walletBalance === null
            ? "You hold no account for this mint yet."
            : `You hold ${toDisplay(walletBalance.toString())}`}
        </span>
      </div>

      <div className="field">
        <span className="muted small">lock tier</span>
        <div className="toggle" role="group" aria-label="Lock tier">
          {LOCK_TIERS.map((t) => (
            <button
              key={t.name}
              className={tier === t.name ? "on" : ""}
              onClick={() => setTier(t.name)}
              disabled={flow.busy}
              title={`${t.days} days locked · ${t.multiplier} reward and vote weight`}
            >
              {t.name}
            </button>
          ))}
        </div>
        <span className="muted small">
          {LOCK_TIERS.find((t) => t.name === tier)?.days} days locked ·{" "}
          {LOCK_TIERS.find((t) => t.name === tier)?.multiplier} weight. Principal cannot be
          withdrawn early — the lock is refused, not penalised.
        </span>
      </div>

      <button className="primary" onClick={onPreview} disabled={flow.busy || parsed === null}>
        Preview
      </button>
      {amount !== "" && parsed === null && (
        <p className="muted small">Base units only — digits, no decimal point.</p>
      )}

      <Preview
        state={flow.state}
        onConfirm={flow.confirm}
        onCancel={flow.reset}
        explorerUrl={explorer}
      />

      <p className="muted small">
        This pool has opened {pool.position_count.toString()} positions in total; yours would be
        #{pool.position_count.toString()}. That number is a seed for the account being created, so
        another staker landing first is a real race — the program refuses a stale id rather than
        writing to the wrong account, and the preview above is where you would see it.
      </p>

      <PositionsList
        positions={positions}
        context={context.value}
        owner={publicKey}
        reload={reload}
      />
    </Panel>
  );
}

// ----------------------------------------------------------------- positions

function PositionsList({
  positions,
  context,
  owner,
  reload,
}: {
  positions: Fetched<Position>[];
  context: PoolContext;
  owner: PublicKey;
  reload: () => void;
}) {
  if (positions.length === 0) {
    return <p className="state muted">You hold no positions in this pool.</p>;
  }

  return (
    <div className="positions">
      <h3>Your positions</h3>
      {positions.map((p) => (
        <PositionRow
          key={p.address.toBase58()}
          held={p}
          context={context}
          owner={owner}
          reload={reload}
        />
      ))}
    </div>
  );
}

function PositionRow({
  held,
  context,
  owner,
  reload,
}: {
  held: Fetched<Position>;
  context: PoolContext;
  owner: PublicKey;
  reload: () => void;
}) {
  const flow = useFlow(reload);
  const explorer = useExplorer();
  const [withdraw, setWithdraw] = useState("");
  const position = held.account;

  const now = BigInt(Math.floor(Date.now() / 1000));
  const unlocked = now >= position.lock_end;
  const parsed = /^\d+$/.test(withdraw) ? BigInt(withdraw) : null;

  const params = {
    poolAddress: context.address,
    pool: context.pool,
    position,
    owner,
  };

  const empty =
    position.amount === 0n && position.weighted_amount === 0n && position.pending_rewards === 0n;

  return (
    <div className="position">
      <div className="position-head">
        <span className="mono">#{position.position_id.toString()}</span>
        <span className="badge">{position.tier.kind}</span>
        <span className="mono muted small" title={held.address.toBase58()}>
          {shortAddress(held.address.toBase58())}
        </span>
      </div>

      <div className="stats">
        <div className="stat">
          <span className="muted small">Principal</span>
          <strong title={`${position.amount} base units`}>
            {toDisplay(position.amount.toString())}
          </strong>
        </div>
        <div className="stat">
          <span className="muted small">Vote weight</span>
          <strong>{toDisplay(position.weighted_amount.toString())}</strong>
        </div>
        <div className="stat">
          <span className="muted small">Unlocks</span>
          <strong title={new Date(Number(position.lock_end) * 1000).toISOString()}>
            {unlocked ? "unlocked" : new Date(Number(position.lock_end) * 1000).toLocaleDateString()}
          </strong>
        </div>
        <div className="stat">
          {/* Deliberately not "claimable". This is the figure booked at the last
              settlement; what a claim actually pays is computed by the program,
              and the preview reports that. Calling this "claimable" would be a
              number the UI cannot stand behind. */}
          <span className="muted small" title="Booked at the last settlement — accrual since then is not shown here. Preview a claim for the real figure.">
            Settled rewards
          </span>
          <strong>{toDisplay(position.pending_rewards.toString())}</strong>
        </div>
      </div>

      <div className="row">
        <input
          value={withdraw}
          onChange={(e) => setWithdraw(e.target.value.trim())}
          placeholder="base units"
          inputMode="numeric"
          spellCheck={false}
          disabled={flow.busy || !unlocked}
        />
        <button
          onClick={() =>
            parsed !== null &&
            flow.preview(() =>
              buildUnstake({ ...params, amount: parsed, tokenProgram: context.stakeTokenProgram }),
            )
          }
          disabled={flow.busy || !unlocked || parsed === null}
          title={unlocked ? undefined : "Principal is locked until the tier's term ends."}
        >
          Unstake
        </button>
        <button
          onClick={() =>
            flow.preview(() =>
              buildClaim({ ...params, tokenProgram: context.rewardTokenProgram }),
            )
          }
          disabled={flow.busy}
          title="Available whether or not the position is locked, and whether or not deposits are paused."
        >
          Claim
        </button>
        <button
          onClick={() => flow.preview(() => buildClosePosition(params))}
          disabled={flow.busy || !empty}
          title={
            empty
              ? "Reclaims the account's rent."
              : "A position can only be closed once its principal, weight and rewards are all zero."
          }
        >
          Close
        </button>
      </div>

      <Preview
        state={flow.state}
        onConfirm={flow.confirm}
        onCancel={flow.reset}
        explorerUrl={explorer}
      />
    </div>
  );
}

// ------------------------------------------------------------------ explorer

/**
 * A link to the signature, on the cluster it was actually sent to.
 *
 * `localnet` gets a `customUrl`, because the explorer's default is mainnet and a
 * link that silently looks up a devnet signature against mainnet reports "not
 * found" for a transaction that succeeded.
 */
export function useExplorer(): (signature: string) => string {
  const { cluster } = useCluster();
  return (signature: string) => {
    const query =
      cluster === "mainnet-beta"
        ? ""
        : cluster === "localnet"
          ? "?cluster=custom&customUrl=http%3A%2F%2F127.0.0.1%3A8899"
          : `?cluster=${cluster}`;
    return `https://explorer.solana.com/tx/${signature}${query}`;
  };
}
