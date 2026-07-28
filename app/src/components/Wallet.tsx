"use client";

/**
 * Wallet connection and cluster switching.
 *
 * `autoConnect` is deliberately on: the adapter only reconnects a wallet the
 * user has already authorised for this origin, so it restores a session rather
 * than initiating one.
 *
 * Cluster is state rather than a build-time constant. A dashboard hard-wired to
 * one cluster is the reason people end up signing devnet transactions against
 * mainnet keys — the switch has to be visible and in the UI.
 */

import { ConnectionProvider, WalletProvider } from "@solana/wallet-adapter-react";
import { WalletModalProvider, WalletMultiButton } from "@solana/wallet-adapter-react-ui";
import { clusterApiUrl } from "@solana/web3.js";
import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

import "@solana/wallet-adapter-react-ui/styles.css";

export type Cluster = "devnet" | "mainnet-beta" | "localnet";

const ClusterContext = createContext<{
  cluster: Cluster;
  setCluster: (c: Cluster) => void;
}>({ cluster: "devnet", setCluster: () => {} });

export const useCluster = () => useContext(ClusterContext);

function endpointFor(cluster: Cluster): string {
  return cluster === "localnet" ? "http://127.0.0.1:8899" : clusterApiUrl(cluster);
}

export function WalletShell({ children }: { children: ReactNode }) {
  const [cluster, setCluster] = useState<Cluster>("devnet");

  const endpoint = useMemo(() => endpointFor(cluster), [cluster]);

  // Empty on purpose. `WalletProvider` discovers anything implementing the
  // Wallet Standard, which Phantom, Solflare, Backpack and the rest have done
  // for some time, so naming adapters explicitly only *limits* what a visitor
  // can connect with.
  //
  // The alternative — `@solana/wallet-adapter-wallets` — pulls in every adapter
  // ever written, including the WalletConnect one, which drags in Reown and
  // `lit` and does not build:
  //
  //   Attempted import error: 'classMap' is not exported from
  //   'lit/directives/class-map.js'
  //
  // Listing individual adapter packages would also work and would still be a
  // hand-maintained allowlist of wallets. Constructed once regardless, because
  // rebuilding this array on every render tears the connection down underneath
  // the user.
  const wallets = useMemo(() => [], []);

  return (
    <ClusterContext.Provider value={{ cluster, setCluster }}>
      <ConnectionProvider endpoint={endpoint}>
        <WalletProvider wallets={wallets} autoConnect>
          <WalletModalProvider>{children}</WalletModalProvider>
        </WalletProvider>
      </ConnectionProvider>
    </ClusterContext.Provider>
  );
}

export function ClusterPicker() {
  const { cluster, setCluster } = useCluster();
  return (
    <label className="cluster">
      <span className="muted">cluster</span>
      <select value={cluster} onChange={(e) => setCluster(e.target.value as Cluster)}>
        <option value="devnet">devnet</option>
        <option value="mainnet-beta">mainnet-beta</option>
        <option value="localnet">localnet</option>
      </select>
    </label>
  );
}

export { WalletMultiButton };
