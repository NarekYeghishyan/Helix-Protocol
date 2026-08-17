# Dashboard

Analytics over the Helix event stream, and the write flows: stake, unstake, claim, close,
vote.

```bash
npm install --ignore-scripts   # see "Install notes" below
npm test                       # 66 tests, no test framework installed
npm run typecheck
npm run build
npm run dev                    # http://localhost:3000
```

## Two halves, two data sources, on purpose

The **analytics** panels read the [read API](../indexer#the-read-api). Point it elsewhere
with `NEXT_PUBLIC_HELIX_API`; the default is `http://127.0.0.1:8080`.

```bash
cargo run -p helix-indexer --features server --bin helix-api
```

The **write** panels do not touch it. They read pools, positions, realms and proposals
straight from the cluster the wallet is connected to, for two reasons:

- A staker who cannot withdraw because an analytics service is down has been given
  something worse than no dashboard at all.
- Signing needs the authoritative value rather than the projected one.
  `pool.position_count` is a *seed* for the account `stake` creates, so a value one slot
  stale produces a transaction that fails on an account collision. The read API is right to
  answer with its uncertainty attached; this is the one caller that cannot use an answer
  like that.

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

## Transactions are built from the IDL, not from a copy of it

Nothing in [`src/lib/`](./src/lib) restates a program's account list.
[`programs.ts`](./src/lib/programs.ts) walks the IDL's accounts in order, derives every PDA
from the seed description the IDL carries, and takes `writable`/`signer` from it.

The alternative — five small functions that each write a discriminator and a couple of
`u64`s — is less code and is wrong in a way that does not surface until it matters. Anchor's
account list is **positional and carries no names on the wire**, so a client using
yesterday's order does not fail to encode. It builds a well-formed transaction naming the
right accounts in the wrong slots and asks a wallet to sign it.

This is the same hazard the indexer was corrected for in Phase 4.2, where a hand-maintained
event list had silently stopped decoding the two events that record governance becoming
self-governing. The answer is the same: use the artifact `anchor build` generates.

That leaves exactly one seam — the IDLs in [`src/idl/`](./src/idl) are a *copy*, because
`target/` is gitignored and this app's CI job has no Anchor toolchain. The seam is guarded:

```bash
anchor build && npm run sync-idl     # regenerate the copies
cargo test -p helix-integration-tests --test idl_sync
```

[`idl_sync.rs`](../tests/integration/tests/idl_sync.rs) runs in the job that *does* have
`anchor build` output and fails if a copy has stopped matching. Both layers catch it
independently: swapping `stake_vault` and `owner_token_account` in the copied IDL fails
`idl_sync.rs` **and** `actions.test.ts`.

## Simulate, then sign — in that order, always

A wallet prompt is the last moment a user can refuse and the moment they have the least
information: a program id and a list of accounts. So the button that opens the wallet is
unreachable until a simulation has come back, and what it came back with is on screen.

Three things follow from that, and each is a decision rather than a default:

- **The simulated transaction is the signed transaction.** A `Prepared` holds the
  instruction array; `simulate` and `send` take that same object. Simulating one list and
  signing a freshly built one is how a preview stops meaning anything, and it happens by
  accident the moment the two are built in different places.
- **Amounts come from the events, not from a second implementation.** "How much will I
  receive?" is answered by decoding the `RewardsClaimed` the *simulated* program emitted.
  Reimplementing `Position::earned` in TypeScript is twenty lines and is the exact mistake
  the indexer was corrected for in Phase 4.0 — a second implementation agrees until one of
  them changes.
- **Failures name themselves.** [`errors.ts`](./src/lib/errors.ts) resolves a program error
  against the IDL of the program the *logs* say failed, so `0x1771` becomes "Position is
  still locked". Anchor's `init` on an existing account arrives as system-program error 0
  with no IDL to look it up in — the most reachable failure in the whole UI, since it is
  what a second vote from one position produces — so it is named explicitly.

A green simulation is stated as what it is: a preview against a recent slot, not a promise.

## Voting says why a button is grey

`cast_vote` accepts a position only if `lock_end >= voting_ends_at` (the flash-loan gate)
and `position_id < position_count_snapshot` (the electorate gate that
[F-10](../docs/SECURITY-ASSESSMENT.md) installed). Neither is guessable from a proposal
list, so every position is shown with the reason it can or cannot vote.

`whyCannotVote` restates the program's comparisons rather than approximating them, and is
only a courtesy — the program enforces both, and simulation is what actually decides.

## Tests without a test framework

`node --test` with Node's native TypeScript type-stripping — no vitest, no jest, no build
step, no dependency.

The trap for a suite testing IDL-driven code is circularity: the encoder reads the IDL, so
asserting its output matches the IDL proves a file was read twice. Every assertion is
independent of that path in one of three ways — Anchor's own discriminator rule
(`sha256("global:<ix>")`), byte vectors written out by hand, or account lists transcribed
from the `#[derive(Accounts)]` structs.

- [`amount.test.ts`](./src/lib/amount.test.ts) — formatting, including the 2^53 case and an
  assertion that the value genuinely does not survive a `Number`, so the test cannot go
  vacuous.
- [`api.test.ts`](./src/lib/api.test.ts) — the client against a stub HTTP server the test
  starts, rather than a patched `fetch`. Patching the dependency tests the patch.
- [`coder.test.ts`](./src/lib/coder.test.ts) — every discriminator in both IDLs recomputed
  from Anchor's rule, exact instruction bytes, and a `Position` decoded from bytes the coder
  did not write.
- [`actions.test.ts`](./src/lib/actions.test.ts) — the five flows against hand-transcribed
  account lists. Note that `stake` and `unstake` order the vault and the owner's token
  account *differently*; positional encoding means a client that assumed one order for both
  would swap source and destination.
- [`errors.test.ts`](./src/lib/errors.test.ts) — error codes worked out from the
  `#[error_code]` enums, including the rule `errors.rs` documents: variants are appended,
  never inserted, because Anchor numbers them from 6000 in declaration order.
- [`chain.test.ts`](./src/lib/chain.test.ts) — the account field lists the UI reads. A cast
  is not a check: rename `total_staked` on chain and the TypeScript still compiles while
  every figure derived from it becomes `undefined`.
- [`events.test.ts`](./src/lib/events.test.ts) — the claimed amount surviving base64, exact
  past 2^53.

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

## What has not been driven from a wallet

Every flow here is built, typed, tested and simulated against a validator's own runtime —
but the tests are `node --test` against hand-written vectors, and simulation is a node
executing a message nobody signed. **No transaction in this app has been through a browser
wallet extension.** The failure modes that live only there — an adapter that re-serialises a
v0 message, a wallet that refuses `replaceRecentBlockhash`, a popup dismissed mid-flow — are
not reachable from anything in CI.

That is the remaining unknown, and it is smaller than it was: what used to be untested was
the instruction bytes, and those are now pinned from both sides.

## Not built yet

Proposal *creation* from the UI, and the lifecycle transitions after a vote closes —
finalize, queue, execute. Voting is the flow with the gates worth explaining; the rest are
permissionless single-account calls, and the same `useFlow` plumbing carries them.
