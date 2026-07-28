# Dashboard

Analytics over the Helix event stream, and wallet connection.

```bash
npm install --ignore-scripts   # see "Install notes" below
npm test                       # 13 tests, no test framework installed
npm run typecheck
npm run build
npm run dev                    # http://localhost:3000
```

It reads the [read API](../indexer#the-read-api). Point it elsewhere with
`NEXT_PUBLIC_HELIX_API`; the default is `http://127.0.0.1:8080`.

```bash
cargo run -p helix-indexer --features server --bin helix-api
```

## What it shows, and what it admits

Nothing is deployed and no ingestion source is wired, so the API serves an empty
projection. **The dashboard says so rather than looking broken.** Three states that a
typical dashboard renders identically are kept distinct here, because they mean different
things:

| State | What it means |
|---|---|
| *The indexer is not answering* | A fact about the dashboard's dependency, not about the protocol |
| *The indexer has never seen this address* | It is ingesting, but nothing about this account reached it |
| *No stakers yet* | It is ingesting, it knows the account, and the answer is genuinely nothing |

An empty table for all three is lying by omission.

## Finality is in the UI, not in a settings menu

The API answers from one of two projections — `finalized`, which the cluster will not take
back, and `head`, which includes transactions a fork still might. The toggle is on the main
row and every panel shows which view it is displaying, with the slot and how many of its
transactions are revocable.

A dashboard that silently served `head` would show a TVL that sometimes goes *down* for no
reason the reader can see. That is a worse failure than being a little behind.

## Amounts never touch a JavaScript number

The API sends amounts as strings because they are `u64` on chain and JSON numbers stop
being exact above 2^53 — about nine million tokens at 9 decimals. That care is wasted if
the client's first move is `Number(response.total_staked)`, which is the default thing to
write.

[`amount.ts`](./src/lib/amount.ts) goes through `BigInt` and places the decimal point by
string manipulation rather than dividing, because dividing by `10 ** decimals` puts the
value straight back into a double.

Writing the test for it found a real bug: `BigInt("")` is `0n` rather than a throw, so a
missing field rendered as a confident `0`. It now renders as `—`.

## Tests without a test framework

`node --test` with Node's native TypeScript type-stripping — no vitest, no jest, no build
step, no dependency. Two files:

- [`amount.test.ts`](./src/lib/amount.test.ts) — formatting, including the 2^53 case and an
  assertion that the value genuinely does not survive a `Number`, so the test cannot go
  vacuous.
- [`api.test.ts`](./src/lib/api.test.ts) — the client against a stub HTTP server the test
  starts, rather than a patched `fetch`. Patching the dependency tests the patch.

## Install notes

`npm install` needs `--ignore-scripts`: a transitive package's postinstall shells out to
`yarn`, which is not a dependency of anything here.

`@solana/wallet-adapter-wallets` is **deliberately not used.** The meta-package pulls in
every adapter ever written, including WalletConnect, which drags in Reown and `lit` and
fails to build:

```text
Attempted import error: 'classMap' is not exported from 'lit/directives/class-map.js'
```

Dropping it removed **603 packages**. Wallets are discovered through the Wallet Standard
instead, which Phantom, Solflare and Backpack all implement — so the wallet list is not a
hand-maintained allowlist that goes stale.

## Not built yet

Stake, unstake, claim and voting flows — ROADMAP 5.2 and 5.3. They need a deployment to be
worth writing against, and the recommendation they are waiting on is already recorded:
simulate every transaction before presenting it for signature, and surface the decoded
Anchor error rather than `custom program error: 0x1771`.
