# Helix Protocol

A composable token, staking, governance and treasury suite for Solana, written in
Rust with the Anchor framework.

[![CI](https://github.com/NarekYeghishyan/Helix-Protocol/actions/workflows/ci.yml/badge.svg)](https://github.com/NarekYeghishyan/Helix-Protocol/actions/workflows/ci.yml)
[![Anchor](https://img.shields.io/badge/anchor-1.1.2-blue)](https://www.anchor-lang.com)
[![Solana](https://img.shields.io/badge/solana-3.x-purple)](https://solana.com)
[![License](https://img.shields.io/badge/license-Apache--2.0-green)](./LICENSE)

> **Status: unaudited. Not deployed.** All four programs build to BPF and pass 71 tests
> locally, including runtime tests against a real Token-2022 mint with transfer fees.
> Governance/treasury runtime coverage, the analytics stack and a devnet deployment are
> scoped in [ROADMAP.md](./docs/ROADMAP.md). Nothing here has held real value.
> See [SECURITY.md](./SECURITY.md).

Four programs that compose into one system. Nothing here wraps an existing protocol —
the reward accounting, the vote-weight mechanism and the governance state machine are
implemented from scratch, and the reasoning behind each is written down in
[`docs/`](./docs).

| Program | Responsibility | Holds authority over | Unit tests |
|---------|---------------|---------------------|-----------|
| [`token-manager`](./programs/token-manager) | HLX mint (Token-2022), minter registry, epoch caps | The mint authority | 7 |
| [`staking`](./programs/staking) | Lock tiers, O(1) reward distribution | Stake + reward vaults | 24 |
| [`governance`](./programs/governance) | Proposals, voting, quorum, timelock | Nothing transferable | 17 |
| [`treasury`](./programs/treasury) | Protocol funds, vesting streams, spend limits | Treasury vault | 17 |

```mermaid
graph TD
    TM["token-manager<br/><i>owns the mint authority</i>"]
    ST["staking<br/><i>owns stake + reward vaults</i>"]
    GV["governance<br/><i>owns nothing transferable</i>"]
    TR["treasury<br/><i>owns protocol funds</i>"]

    TM -->|"mint_to — PDA-signed CPI,<br/>caller must be a registered minter"| ST
    ST -->|"position weight,<br/>gated on lock_end"| GV
    GV -->|"spend — only after quorum<br/>+ timelock, PDA-signed CPI"| TR
    GV -->|"set_reward_rate"| ST
```

There is no address that can move treasury funds. `treasury` accepts spend instructions
from exactly one signer — the `governance` execution PDA — and `governance` will only
produce that signature after a proposal has passed quorum *and* cleared its timelock.
That chain is the security model.

---

## Three decisions worth reading the code for

**Reward distribution is O(1), not O(stakers).** Rewards use a `reward_per_token`
accumulator in u128 fixed point — the Synthetix/MasterChef shape. No instruction
iterates over the staker set. The naive alternative passes a ten-staker test and then
permanently bricks the pool at ten thousand, once distribution exceeds the compute
budget. → [`staking/src/state.rs`](./programs/staking/src/state.rs),
[architecture](./docs/ARCHITECTURE.md#reward-accounting).

**Flash-loan governance attacks fail by construction.** A position may vote only if
`lock_end >= proposal.voting_ends_at` — you can only vote with stake you are unable to
withdraw before the vote closes. Borrowed capital has `lock_end == now` and carries zero
weight. Stronger than a snapshot, which can be gamed by borrowing *before* the snapshot
block, and it costs one comparison rather than a history of balance checkpoints. →
[`governance/src/instructions/vote.rs`](./programs/governance/src/instructions/vote.rs),
[threat model](./docs/THREAT-MODEL.md#a1--flash-loan-governance-capture).

**Token-2022 transfer fees are accounted for — and proven so.** When a mint carries the
transfer-fee extension, the amount sent is not the amount that arrives. Every deposit path
credits the *observed vault balance delta*, never the `amount` argument.

This is invisible on a plain SPL mint, where the two are identical — so the whole unit
suite passes either way. It is verified by running the staking flow against a real
fee-bearing mint, and that test was **mutation-tested**: reverting the fix produces a
30,000-unit shortfall between positions and vault, while the plain-mint test stays green.
→ [`staking_transfer_fee.rs`](./tests/integration/tests/staking_transfer_fee.rs),
[invariants](./docs/INVARIANTS.md#2-token-2022-transfer-fees).

## Documentation

Structured as the five deliverables of an architecture-and-enhancement engagement.

| | Deliverable |
|---|---|
| [ARCHITECTURE-REVIEW.md](./docs/ARCHITECTURE-REVIEW.md) | **1 — Architecture review.** The review method, and it applied to this codebase: strengths, weaknesses, scalability limits, improvement opportunities |
| [SECURITY-ASSESSMENT.md](./docs/SECURITY-ASSESSMENT.md) | **2 — Security assessment.** Access-control matrix, risk register with severities, recommended mitigations |
| [ROADMAP.md](./docs/ROADMAP.md) | **3 — Technical roadmap.** Prioritised phases, milestones, estimates, and an explicit list of what is *not* built |
| [ARCHITECTURE.md](./docs/ARCHITECTURE.md) | **4 — Enhancements.** How the four programs compose and why each design choice was made |
| [INVARIANTS.md](./docs/INVARIANTS.md) · [TESTING.md](./docs/TESTING.md) · [RUNBOOK.md](./docs/RUNBOOK.md) | **5 — Documentation.** Invariants mapped to tests, testing procedures, deployment runbook |
| [THREAT-MODEL.md](./docs/THREAT-MODEL.md) | Attacks defended, trust assumptions, and what is explicitly out of scope |

## Quick start

Requires Linux or WSL2 — the Solana BPF toolchain does not build natively on Windows.

```bash
# One-time toolchain setup (Rust, Solana CLI, Anchor, Node)
bash scripts/bootstrap-wsl.sh

# Program keypairs (gitignored; generated once per developer)
node scripts/gen-program-keys.mjs
anchor keys sync

anchor build
cargo test --workspace --lib
```

Toolchain: Anchor 1.1.2, Solana 3.1.10, Rust stable. See
[RUNBOOK.md](./docs/RUNBOOK.md) for deployment.

## Repository layout

```text
programs/
  token-manager/   HLX mint, minter registry, two-step admin transfer
  staking/         lock tiers, reward accumulator, position accounting
  governance/      proposal lifecycle, lock-gated voting, timelock
  treasury/        vault, vesting streams, per-epoch spend limits
scripts/           toolchain bootstrap, program key generation
docs/              the five deliverables above
.github/workflows/ fmt, clippy -D warnings, cargo-audit, build, test
```

## Testing

```bash
anchor build 2>&1 | tee build.log
grep -i "stack offset" build.log   # must be empty — anchor build exits 0 even when it isn't
cargo test --workspace             # 71 tests: unit + doc + integration
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

**65 unit tests** cover the reward accumulator, vesting schedule, tally arithmetic and
every state machine directly — including a differential check that fixed-point rounding
never favours the user over the pool. **6 runtime tests** ([`tests/integration/`](./tests/integration))
exercise the real BPF programs against the real Token-2022 program via LiteSVM.

Governance and treasury runtime flows are **not yet covered** — the executor PDA signing a
treasury spend is still proven only to type-check. See [TESTING.md](./docs/TESTING.md) for
the full list of what is and is not covered; it is kept honest deliberately.

## License

Apache-2.0. See [LICENSE](./LICENSE).
