# Helix Protocol

A composable token, staking, governance and treasury suite for Solana, written in
Rust with the Anchor framework.

[![CI](https://github.com/narekyeghishyan/helix-protocol/actions/workflows/ci.yml/badge.svg)](https://github.com/narekyeghishyan/helix-protocol/actions/workflows/ci.yml)
[![Anchor](https://img.shields.io/badge/anchor-1.1.2-blue)](https://www.anchor-lang.com)
[![License](https://img.shields.io/badge/license-Apache--2.0-green)](./LICENSE)

> **Status: unaudited, devnet only.** See [SECURITY.md](./SECURITY.md).

Four programs that compose into one system. Nothing here is a wrapper around an
existing protocol — the reward accounting, the vote-weight mechanism and the
governance state machine are implemented from scratch, and the reasoning behind each
is written down in [`docs/`](./docs).

| Program | Responsibility | Holds authority over |
|---------|---------------|---------------------|
| [`token-manager`](./programs/token-manager) | HLX mint (Token-2022), minter registry, epoch caps | The mint authority |
| [`staking`](./programs/staking) | Lock tiers, O(1) reward distribution, voter weight | Stake + reward vaults |
| [`governance`](./programs/governance) | Proposals, voting, quorum, timelock | Nothing transferable |
| [`treasury`](./programs/treasury) | Protocol funds, vesting streams, spend limits | Treasury vault |

Plus a [Next.js dashboard](./app) with wallet-adapter integration and an
[event indexer](./indexer) that turns on-chain events into the analytics the dashboard
reads.

---

## Three decisions worth reading the code for

**Reward distribution is O(1), not O(stakers).** Rewards use a `reward_per_token`
accumulator in u128 fixed point — the Synthetix/MasterChef shape. No instruction
iterates over the staker set. The naive alternative works fine with ten stakers in a
test and permanently bricks the pool at ten thousand, when distribution exceeds the
compute budget. [Details](./docs/ARCHITECTURE.md#reward-accounting).

**Flash-loan governance attacks fail by construction.** A position may vote only if
`lock_end >= proposal.voting_ends_at` — you can only vote with stake you are unable to
withdraw before the vote closes. Borrowed capital has `lock_end == now` and carries
zero weight. This is stronger than a snapshot (which can be gamed by borrowing before
the snapshot) and costs one comparison rather than a history of balance checkpoints.
[Details](./docs/THREAT-MODEL.md#a1--flash-loan-governance-capture).

**Token-2022 transfer fees are handled honestly.** When a mint carries the
transfer-fee extension, the amount sent is not the amount that arrives. Every deposit
path credits the *observed vault balance delta*, never the `amount` argument. The
staking suite runs end to end twice — once on a plain mint, once on a fee-bearing mint
— because this bug is invisible until someone enables the extension.
[Details](./docs/INVARIANTS.md#2-token-2022-transfer-fees).

## Documentation

| | |
|---|---|
| [ARCHITECTURE.md](./docs/ARCHITECTURE.md) | How the four programs compose, and why each design choice was made |
| [INVARIANTS.md](./docs/INVARIANTS.md) | Properties that must always hold, each mapped to the test that asserts it |
| [THREAT-MODEL.md](./docs/THREAT-MODEL.md) | Attacks defended, trust assumptions, and what is explicitly out of scope |
| [SECURITY.md](./SECURITY.md) | Disclosure policy and security practices |

## Quick start

Requires Linux or WSL2 — the Solana BPF toolchain does not build natively on Windows.

```bash
# One-time toolchain setup (Rust, Solana CLI, Anchor, Node, Surfpool)
bash scripts/bootstrap-wsl.sh

# Program keypairs (gitignored; generated once per developer)
node scripts/gen-program-keys.mjs
anchor keys sync

pnpm install
anchor build
anchor test
```

## Repository layout

```
programs/
  token-manager/   HLX mint, minter registry, two-step admin transfer
  staking/         lock tiers, reward accumulator, voter weight records
  governance/      proposal lifecycle, vote tallying, timelock
  treasury/        vault, vesting streams, per-epoch spend limits
tests/             TypeScript integration + invariant assertions
app/               Next.js dashboard, wallet-adapter
indexer/           Anchor event → Postgres pipeline feeding the dashboard
scripts/           toolchain bootstrap, key generation, deployment
docs/              architecture, invariants, threat model
```

## Testing

```bash
cargo test --workspace --lib    # fast Rust unit tests (math, state transitions)
anchor test                     # full integration suite against a local validator
trident fuzz run                # stateful fuzzing
```

The suite asserts the invariants in [INVARIANTS.md](./docs/INVARIANTS.md) directly,
including a differential test of the fixed-point reward math against exact rational
arithmetic, to prove rounding never favours the user over the pool.

## License

Apache-2.0. See [LICENSE](./LICENSE).
