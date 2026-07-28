# Deployment runbook

*Deliverable 5 — deployment instructions.*

> **Nothing is deployed yet.** This is the procedure to follow, written before the first
> deployment rather than after, so the checks exist when they are needed. Program IDs
> below are the local keypairs in `target/deploy/`; the devnet table is empty until
> Phase 3 of [ROADMAP.md](./ROADMAP.md).

## Program IDs

| Program | ID | Localnet | Devnet | Mainnet |
|---|---|---|---|---|
| `helix_token_manager` | `5RU35Eni3MxkuSc9Zv5xm8LLd2QX85XdbYjRUaLkFRFr` | ✅ | — | — |
| `helix_staking` | `9RuZJZpgCwbiF9JRAsyR8cqDhFSaFYus1mzobKzEZzP3` | ✅ | — | — |
| `helix_governance` | `nSZnzJR8uUuZu8t1SqmLU2ExCvXNYABuVHwrDQJqSf5` | ✅ | — | — |
| `helix_treasury` | `B9HenpXUQzzGdT7mv93MQM8f6ytdPRKhJCbdx1CcBvdh` | ✅ | — | — |

## 0. Prerequisites

```bash
bash scripts/bootstrap-wsl.sh     # Rust, Solana CLI, Anchor, Node
anchor --version                  # expect anchor-cli 1.1.2
solana --version                  # expect 3.1.10
```

Program keypairs live in `target/deploy/` and are **gitignored**. They are the deploy
authority for their program IDs — losing them means losing the ability to upgrade;
leaking them means losing the program.

```bash
node scripts/gen-program-keys.mjs   # generates only what is missing, never overwrites
anchor keys sync                    # aligns declare_id! and Anchor.toml
```

`gen-program-keys.mjs` refuses to replace an existing keypair. Filenames must match each
crate's `[lib] name` — otherwise `anchor build` silently generates its own keypair and
deploys to a different address than you expect.

## 1. Pre-deploy gate

Every item must pass. This is a gate, not a checklist to skim.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
anchor build 2>&1 | tee build.log
```

- [ ] `fmt`, `clippy`, tests all clean
- [ ] **`grep -i "stack offset" build.log` returns nothing.** `anchor build` reports SBF
      frame overflows as errors and *still exits 0* — the exit code cannot be trusted.
      See [F-3](./SECURITY-ASSESSMENT.md#f-3--sbf-stack-frame-overflow).
- [ ] `anchor keys verify` — declared IDs match the keypairs
- [ ] Integration tests pass (**blocked**: Phase 2 not built — do not deploy anything
      holding value until this line can be ticked)
- [ ] `cargo audit` clean

## 2. Deploy

```bash
solana config set --url devnet
solana balance                       # ~10 SOL is comfortable for four programs
anchor deploy --provider.cluster devnet
```

Record the resulting IDs in the table above, in the same commit.

If a deploy fails part-way, the buffer account persists and holds SOL. Recover with
`solana program show --buffers` and `solana program close <BUFFER>`; do not simply retry
until the balance is gone.

## 3. Bootstrap — run atomically

> **This is the security-critical step.** `initialize_pool`, `initialize_realm` and
> `initialize_treasury` take the privileged party as an argument, and the first caller
> wins. An observer can front-run them between deploy and bootstrap and install
> themselves as authority — permanently, since the PDAs are seeded by the mints. See
> [F-1](./SECURITY-ASSESSMENT.md#f-1--initialisers-are-front-runnable).
>
> Build all of step 3 as **one transaction**. Do not run these as separate commands.

Order, because each step depends on the previous:

1. `initialize_token` — creates the HLX mint with a PDA mint authority.
2. `initialize_pool` — stake mint = HLX, reward mint = HLX; both vaults are PDAs.
3. `initialize_realm` — over that pool; sets quorum, approval, voting period, timelock,
   and the guardian.
4. `initialize_treasury` — `governance_executor` = the realm's executor PDA,
   `["executor", realm]`.
5. `register_minter` — authority = the pool's reward-funding PDA, with an epoch cap
   matching intended emissions.

## 4. Wire the authorities

Until this is done, emissions and treasury are operator-controlled, not DAO-controlled.
Both handovers are deliberately two-step, so each needs both halves.

```text
staking pool authority   ──▶ realm executor PDA   (propose_authority → accept_authority)
token-manager admin      ──▶ realm executor PDA   (propose_admin → accept_admin)
```

`accept_*` requires the new authority to **sign**, and the executor PDA can only sign
inside a governance execution — so the accepting half must be driven by a passed
proposal. Sequence it as: propose off-chain → create/activate/vote/finalize/queue →
execute. Budget a full voting period plus timelock for this, and do not treat it as a
five-minute deployment step.

## 5. Verify

Do not skip. This is what catches F-1 and misconfiguration.

```bash
anchor idl fetch <PROGRAM_ID> --provider.cluster devnet   # IDL is published
```

- [ ] HLX mint authority == `["mint_authority", config]` PDA, and **no keypair holds it**
- [ ] `pool.authority` == realm executor PDA
- [ ] `treasury.governance_executor` == realm executor PDA
- [ ] `realm.guardian` == the intended multisig, and nothing else
- [ ] `token_config.admin` == intended
- [ ] `pool.reward_rate` and `reward_period_end` match intent, and the reward vault
      holds at least `unpaid_liability + emission_for(rate, now)`
- [ ] `treasury.epoch_spend_cap` is set (a zero cap blocks all spending; an unbounded
      one removes the control)

Smoke test with a small amount before announcing anything: stake → wait → claim →
unstake, and one `Signal` proposal through the full lifecycle including `execute`.

## 6. Verifiable builds

So a third party can confirm the deployed bytecode came from this source tree.

```bash
solana-verify build
solana-verify verify-from-repo -u devnet --program-id <ID> https://github.com/NarekYeghishyan/Helix-Protocol
```

Publish the resulting hash in the release notes. Without this, "the code is on GitHub"
is an assertion, not something anyone can check.

## 7. Upgrade authority

```bash
solana program set-upgrade-authority <PROGRAM_ID> --new-upgrade-authority <MULTISIG>
```

Beta: a 3-of-5 Squads multisig. Intended end state: the governance program itself, so
upgrades follow the same timelocked path as spends.

- [ ] All four upgrade authorities transferred
- [ ] Verified with `solana program show <PROGRAM_ID>` for each

Treat this as an explicit, verifiable step. It is the most commonly skipped one, and
skipping it makes every other control in [SECURITY-ASSESSMENT.md](./SECURITY-ASSESSMENT.md)
conditional on whoever holds the key.

## Rollback

Programs are upgradeable, so rollback means deploying the previous verified build:

```bash
git checkout <PREVIOUS_TAG> && anchor build
anchor upgrade target/deploy/<program>.so --program-id <ID> --provider.cluster devnet
```

**State is not rolled back.** If a bad version corrupted account data, the fix is a
migration instruction, not an upgrade — Anchor 1.x provides a `Migration` type for
moving accounts between layouts. Plan for this before it is needed.

Incident order of operations: `set_paused` on the staking pool (stops new deposits, and
deliberately does *not* block `unstake`/`claim`) → guardian-cancel any in-flight
proposals → then diagnose. Pausing is cheap and reversible; a rushed upgrade is neither.
