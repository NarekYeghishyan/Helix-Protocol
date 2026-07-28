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
cargo test --workspace
anchor build 2>&1 | tee build.log
```

- [ ] `fmt`, `clippy`, tests all clean
- [ ] **`grep -i "stack offset" build.log` returns nothing.** `anchor build` reports SBF
      frame overflows as errors and *still exits 0* — the exit code cannot be trusted.
      See [F-3](./SECURITY-ASSESSMENT.md#f-3--sbf-stack-frame-overflow).
- [ ] `anchor keys verify` — declared IDs match the keypairs
- [ ] Runtime tests pass — `cargo test --workspace` covers the unit, doc and 83 runtime
      tests, including the fuzz campaign
- [ ] `cargo audit` clean

## 2. Deploy

**Budget ~10 SOL, and have ~20 available.** Measured rent-exemption cost for the four
programs, at ~6960 lamports per byte:

| Program | Size | Rent |
|---|---|---|
| `helix_governance` | 436,632 B | ~3.04 SOL |
| `helix_staking` | 364,136 B | ~2.53 SOL |
| `helix_treasury` | 328,176 B | ~2.28 SOL |
| `helix_token_manager` | 298,976 B | ~2.08 SOL |
| **total** | **1,427,920 B** | **~9.94 SOL** |

Deployment writes to a temporary buffer of the same size before finalising, so peak
requirement is roughly double. Devnet's built-in `solana airdrop` is capped at 2 SOL and
rate-limits aggressively — expect to use [faucet.solana.com](https://faucet.solana.com)
(GitHub-authenticated, higher daily limit) rather than the CLI.

```bash
solana config set --url devnet
solana balance                       # need ~20 SOL headroom
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
> Build steps 2–4 below as **one transaction**. Do not run them as separate commands.
>
> This is measured, not hoped: `bootstrap_atomicity.rs` builds the real transaction and
> reports **748 bytes across 17 accounts**, against Solana's 1232-byte limit. The test
> asserts that limit, so it fails if the bootstrap ever grows past it.

The mint is created first and separately — it is a fresh keypair the deployer signs for,
so it cannot be front-run. The three PDA-seeded initialisers go together.

1. `initialize_token` — creates the HLX mint with a PDA mint authority. **Admin stays a
   human key for now**; see step 4.
2. `initialize_pool` — stake mint = HLX, reward mint = HLX, both vaults PDAs.
   **`authority` = the realm's executor PDA**, `["executor", realm]`.
3. `initialize_realm` — over that pool; sets quorum, approval, voting period, timelock and
   the guardian. `authority` = its own executor PDA, so parameter changes need a vote.
4. `initialize_treasury` — `governance_executor` = the same executor PDA.

Note that steps 2–4 name the executor PDA directly. The executor's address is derivable
before the realm exists, because it is a PDA of a PDA of the pool — so **there is never a
moment when a human key controls emissions or the treasury**. No handover is required for
either, and the earlier version of this runbook was wrong to prescribe one.

### Build it with the planner, and read the plan first

```bash
cargo run -p helix-ops --bin helix-bootstrap -- \
    --payer <DEPLOYER> --mint <HLX_MINT> --guardian <MULTISIG>
```

It prints the transaction size against the 1232-byte cap, every derived address, and who
will hold each authority once it lands. **Read that last section before sending anything** —
it is the only chance to notice that an authority is a key rather than the executor PDA,
and the tool refuses to emit at all if one is.

```bash
# ... and the instructions, for whatever client will sign and send them:
cargo run -p helix-ops --bin helix-bootstrap -- ... --json > bootstrap.json
```

The instruction set is not a transcription of the list above: it is
[`helix_ops::plan`](../ops/src/lib.rs), and
[`bootstrap_atomicity.rs`](../tests/integration/tests/bootstrap_atomicity.rs) executes that
same function against the real programs. **The plan you are about to send is the plan the
test suite has run.** Most deployment scripts are written once, run once, and tested never;
this one is exercised on every push.

The tool does not submit. That is deliberate — see the note in
[`ops/Cargo.toml`](../ops/Cargo.toml) — and it also means the last human check is not
optional: nothing goes out until someone signs it.

## 4. Hand over the token-manager admin

The one authority that genuinely cannot be set at initialisation.

`register_minter` must run before governance can be asked to do anything useful — the
staking program has to be a registered minter before it can pay rewards — and only the
admin can register a minter. So the admin starts as a human key, registers the staking
minter, and then hands over:

```text
token-manager admin ──▶ realm executor PDA   (propose_admin → accept_admin)
```

`accept_admin` requires the new admin to **sign**, and the executor PDA signs only inside
a governance execution — so the accepting half is a proposal:

```text
1. register_minter        multisig, direct        the staking reward PDA
2. propose_admin          multisig, direct        new_admin = realm executor PDA
3. AcceptTokenManagerAdmin  proposal → execute    completes the handover
```

Budget a full voting period plus timelock for step 3, and do not treat it as a
five-minute deployment step.

After this the realm holds every admin power — pause, register, update and revoke minters,
and propose the admin onward — so nothing is stranded. That completeness was the point of
[F-9](./SECURITY-ASSESSMENT.md#f-9--token-manager-admin-cannot-be-handed-to-governance):
handing over the role without the powers would have been a fresh instance of the same bug.

## 4b. Hand over the realm authority

The realm is bootstrapped with a human `authority` so its parameters can be tuned during
setup. Until this step runs, **that key can rewrite what "passing" means** — lower
`quorum_bps` to the 0.01% floor and a dust position carries a treasury transfer. See
[F-11](./SECURITY-ASSESSMENT.md#f-11--the-rules-of-governance-were-owned-from-outside-it),
and `lowering_quorum_lets_a_dust_position_move_the_treasury`, which runs the attack.

Treat it as the last configuration change, not an afterthought: after it, every further
parameter change costs a voting period plus a timelock.

```bash
# 1. Settle on final parameters and set them directly, while that is still cheap.
#    update_realm_params(quorum_bps, approval_bps, voting_period,
#                        timelock_delay, min_weight_to_propose)

# 2. Propose handing the authority to the realm's own executor PDA, and pass it.
#    ProposalAction::SetRealmAuthority { new_authority: <executor PDA> }
#    -> activate -> vote -> finalize -> queue -> (timelock) -> execute_set_realm_authority

# 3. Confirm the old key is powerless.
#    update_realm_params signed by the old authority must now fail with NotAuthority.
```

Point it at the **executor PDA specifically**. Any other address that cannot be made to
sign would freeze the realm's configuration permanently — the F-8 defect one step further
along. Parameters stay reachable afterwards through
`ProposalAction::UpdateRealmParams`, which
`governance_can_retune_its_own_parameters` covers.

## 5. Verify

Do not skip. This is what catches F-1 and misconfiguration.

```bash
anchor idl fetch <PROGRAM_ID> --provider.cluster devnet   # IDL is published
```

- [ ] HLX mint authority == `["mint_authority", config]` PDA, and **no keypair holds it**
- [ ] `pool.authority` == realm executor PDA — set at init, so this should already hold
- [ ] `treasury.governance_executor` == realm executor PDA — likewise
- [ ] `realm.authority` == its own executor PDA, so parameter changes require a vote.
      **The single most important line in this list** — until it holds, everything below
      it can be undone by one key changing the thresholds (F-11)
- [ ] `realm.quorum_bps`, `approval_bps`, `voting_period`, `timelock_delay` and
      `min_weight_to_propose` all match intent, checked *after* the handover rather than
      before
- [ ] `realm.guardian` == the intended multisig, and nothing else
- [ ] `token_config.admin` == the realm executor PDA once step 4 completes, and
      `pending_admin` is cleared
- [ ] Exactly one registered minter, and it is the staking program's reward PDA
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
