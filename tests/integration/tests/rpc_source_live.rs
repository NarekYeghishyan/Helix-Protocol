//! Phase 4.1 — the indexer against a cluster, rather than against a fake.
//!
//! Every other test in this suite runs on LiteSVM, which hands back the
//! runtime's log buffer directly. That proves the *decoder* and the *fold*, and
//! it is why those are where the bugs get caught. It cannot prove the step in
//! between: that `getSignaturesForAddress` and `getTransaction` deliver the same
//! logs, in the same order, under the same identity.
//!
//! That step has no interesting logic and four ways to be silently wrong — see
//! [`helix_indexer::rpc`]. Each of them produces a projection that is merely
//! *incorrect*, never one that errors, so the only way to find out is to run it.
//!
//! # Running it
//!
//! ```text
//! solana-test-validator --reset &
//! solana program deploy -u localhost --program-id target/deploy/<p>-keypair.json target/deploy/<p>.so
//! HELIX_RPC_URL=http://127.0.0.1:8899 cargo test -p helix-integration-tests --test rpc_source_live
//! ```
//!
//! Without `HELIX_RPC_URL` every test here returns early and passes. That is a
//! deliberate trade: the alternative is a suite that is red on every machine
//! with no validator running, and a suite expected to be red stops being read at
//! all. `docs/TESTING.md` records the count separately for the same reason.

use anchor_lang::prelude::Pubkey;
use anchor_spl::token_2022::spl_token_2022;
use helix_indexer::rpc::RpcLogSource;
use helix_indexer::{Backfill, Ingestor, LogSource, Program, SettledTransaction};
use helix_integration_tests::bootstrap::default_realm_params;
use helix_integration_tests::cluster::{Cluster, ClusterError};
use helix_integration_tests::{cluster_or_skip, pda, TestEnv};
use helix_ops::BootstrapConfig;
use helix_staking::state::{LockTier, Pool, Position};
use solana_keypair::Keypair;
use solana_signer::Signer as _;
use spl_token_2022::extension::ExtensionType;
use spl_token_2022::state::{Account as T22Account, Mint as T22Mint};

const DECIMALS: u8 = 9;

/// A Helix deployment on a real cluster, plus the keys that can act on it.
struct Live {
    cluster: Cluster,
    mint: Pubkey,
    pool: Pubkey,
}

impl Live {
    /// Mints a token, runs the bootstrap, and returns the wired system.
    ///
    /// The bootstrap is [`helix_ops::plan`] — the same instructions
    /// `helix-bootstrap` prints and `bootstrap_atomicity.rs` executes under
    /// LiteSVM. Running it here is the first time it has been submitted to
    /// something with a mempool.
    fn bootstrap(cluster: Cluster) -> Self {
        let mint = Keypair::new();
        let mint_authority = cluster.payer().insecure_clone();

        let space = ExtensionType::try_calculate_account_len::<T22Mint>(&[]).expect("mint size");
        let create = solana_system_interface::instruction::create_account(
            &cluster.payer().pubkey(),
            &mint.pubkey(),
            cluster.rent_exemption(space).expect("rent"),
            space as u64,
            &spl_token_2022::ID,
        );
        let initialize = spl_token_2022::instruction::initialize_mint2(
            &spl_token_2022::ID,
            &mint.pubkey(),
            &mint_authority.pubkey(),
            None,
            DECIMALS,
        )
        .expect("initialize_mint2");

        cluster
            .send(&[create, initialize], &[&mint])
            .expect("create mint");

        let plan = helix_ops::plan(&BootstrapConfig {
            payer: cluster.payer().pubkey(),
            mint: mint.pubkey(),
            guardian: Keypair::new().pubkey(),
            realm: default_realm_params(),
            epoch_spend_cap: 1_000_000_000,
            epoch_duration: 24 * 3_600,
        });

        cluster
            .send(&plan.instructions, &[])
            .expect("the bootstrap transaction the runbook tells an operator to send");

        let (pool, _) = pda::pool(&mint.pubkey(), &mint.pubkey());
        Self {
            cluster,
            mint: mint.pubkey(),
            pool,
        }
    }

    /// A token account owned by the payer, holding `amount`.
    fn token_account(&self, amount: u64) -> Pubkey {
        self.token_account_for(&self.cluster.payer().pubkey(), amount)
    }

    /// A funded staker with a token account holding `amount`.
    fn staker(&self, amount: u64) -> (Keypair, Pubkey) {
        let owner = Keypair::new();
        self.cluster.airdrop(owner.pubkey(), 10).expect("airdrop");
        let tokens = self.token_account_for(&owner.pubkey(), amount);
        (owner, tokens)
    }

    fn token_account_for(&self, owner: &Pubkey, amount: u64) -> Pubkey {
        let tokens = Keypair::new();
        let space =
            ExtensionType::try_calculate_account_len::<T22Account>(&[]).expect("account size");
        let create = solana_system_interface::instruction::create_account(
            &self.cluster.payer().pubkey(),
            &tokens.pubkey(),
            self.cluster.rent_exemption(space).expect("rent"),
            space as u64,
            &spl_token_2022::ID,
        );
        let initialize = spl_token_2022::instruction::initialize_account3(
            &spl_token_2022::ID,
            &tokens.pubkey(),
            &self.mint,
            owner,
        )
        .expect("initialize_account3");
        let mint_to = spl_token_2022::instruction::mint_to(
            &spl_token_2022::ID,
            &self.mint,
            &tokens.pubkey(),
            &self.cluster.payer().pubkey(),
            &[],
            amount,
        )
        .expect("mint_to");

        self.cluster
            .send(&[create, initialize, mint_to], &[&tokens])
            .expect("fund token account");

        tokens.pubkey()
    }

    /// A permissionless treasury deposit — no counter to match, so several can
    /// be in flight at once without racing each other.
    fn deposit_ix(&self, funder: &Pubkey, amount: u64) -> solana_instruction::Instruction {
        let (treasury, _) = pda::treasury(&self.mint);
        let (vault, _) = pda::treasury_vault(&treasury);

        TestEnv::ix(
            helix_treasury::ID,
            helix_treasury::accounts::Deposit {
                treasury,
                depositor: self.cluster.payer().pubkey(),
                mint: self.mint,
                depositor_token_account: *funder,
                vault,
                token_program: spl_token_2022::ID,
            },
            helix_treasury::instruction::Deposit { amount },
        )
    }

    fn stake_ix(
        &self,
        owner: &Pubkey,
        tokens: &Pubkey,
        position_id: u64,
        amount: u64,
        tier: LockTier,
    ) -> solana_instruction::Instruction {
        let (position, _) = pda::position(&self.pool, owner, position_id);
        let (stake_vault, _) = pda::stake_vault(&self.pool);

        TestEnv::ix(
            helix_staking::ID,
            helix_staking::accounts::Stake {
                pool: self.pool,
                owner: *owner,
                position,
                stake_mint: self.mint,
                owner_token_account: *tokens,
                stake_vault,
                token_program: spl_token_2022::ID,
                system_program: anchor_lang::system_program::ID,
            },
            helix_staking::instruction::Stake {
                position_id,
                amount,
                tier,
            },
        )
    }

    /// Stakes, returning the position's address.
    fn stake(
        &mut self,
        owner: &Keypair,
        tokens: &Pubkey,
        position_id: u64,
        amount: u64,
        tier: LockTier,
    ) -> Pubkey {
        let ix = self.stake_ix(&owner.pubkey(), tokens, position_id, amount, tier);
        self.cluster.send(&[ix], &[owner]).expect("stake");
        pda::position(&self.pool, &owner.pubkey(), position_id).0
    }

    /// Reads everything the cluster has, through the source a deployment uses.
    ///
    /// Polls until the ingestor stops finding new transactions, so the assertion
    /// is about the state reached rather than about how many polls it took.
    fn ingest(&self) -> Ingestor {
        let mut source = RpcLogSource::new(self.cluster.url());
        let mut ingestor = Ingestor::new();
        for _ in 0..32 {
            let outcome = ingestor.poll(&mut source, 500).expect("poll");
            if outcome.applied == 0 {
                break;
            }
        }
        ingestor
    }
}

/// The whole of Phase 4.1: real programs, real RPC, matching the accounts.
#[test]
fn a_projection_built_over_rpc_matches_the_accounts_on_chain() {
    let mut live = Live::bootstrap(cluster_or_skip!());

    let (owner, tokens) = live.staker(10_000_000);
    let first = live.stake(&owner, &tokens, 0, 1_000_000, LockTier::Flexible);
    // A locked tier as well as a flexible one, so `weighted_amount` differs from
    // `amount` and the comparison below is not satisfied by copying one field.
    let second = live.stake(&owner, &tokens, 1, 2_500_000, LockTier::Silver);

    let ingestor = live.ingest();
    let head = ingestor.head();

    // The chain's own answer, not a second reading of the same events.
    let pool: Pool = live.cluster.account(live.pool).expect("pool account");
    assert_eq!(
        head.tvl(&live.pool),
        pool.total_staked,
        "the projection disagrees with Pool.total_staked"
    );

    for (address, expected_amount) in [(first, 1_000_000u64), (second, 2_500_000)] {
        let on_chain: Position = live.cluster.account(address).expect("position account");
        assert_eq!(on_chain.amount, expected_amount);

        let projected = head
            .positions
            .get(&address)
            .unwrap_or_else(|| panic!("{address} is missing from the projection"));
        assert_eq!(projected.amount, on_chain.amount);
        assert_eq!(projected.weighted_amount, on_chain.weighted_amount);
        assert_eq!(projected.owner, on_chain.owner);
        assert_eq!(projected.lock_end, on_chain.lock_end);
    }

    assert!(
        head.orphaned.is_empty(),
        "an event referred to something never created: {:?}",
        head.orphaned
    );
}

/// `getSignaturesForAddress` answers newest-first. [`LogSource::fetch`] is
/// specified in ledger order, and the difference is invisible in the result.
///
/// Invisible because the projection *assigns* running totals rather than
/// accumulating them — which is what makes redelivery safe, and is exactly what
/// makes a reversed batch settle on the oldest value in it instead of failing.
///
/// # Why this bursts rather than staking
///
/// Sorting the merged batch by slot re-establishes ascending order on its own,
/// so **the only thing the reversal changes is the order of transactions
/// sharing a slot** — and a test that confirms each transaction before sending
/// the next never produces two. The first version of this test did exactly
/// that, passed, and went on passing with the reversal deleted.
///
/// So: several deposits, in flight at once, with the assertion that at least two
/// really did land together. Deposits rather than stakes because `position_id`
/// must match the pool's counter, which makes concurrent stakes a race with
/// itself rather than a test of ordering.
#[test]
fn ledger_order_survives_inside_a_slot() {
    let live = Live::bootstrap(cluster_or_skip!());
    let (treasury, _) = pda::treasury(&live.mint);

    let funder = live.token_account(100_000_000);
    let blockhash = live.cluster.latest_blockhash().expect("blockhash");
    let amounts = [1_000u64, 2_000, 4_000, 8_000, 16_000, 32_000];

    let sent: Vec<String> = amounts
        .iter()
        .map(|amount| {
            live.cluster
                .send_nowait(&[live.deposit_ix(&funder, *amount)], &[], blockhash)
                .expect("submit deposit")
        })
        .collect();
    for signature in &sent {
        live.cluster.confirm(signature).expect("confirm deposit");
    }

    let mut source = RpcLogSource::new(live.cluster.url());
    let batch = source
        .fetch(&Default::default(), 500)
        .expect("fetch the whole ledger");

    let slots: Vec<u64> = batch.iter().map(|tx| tx.slot).collect();
    let mut ascending = slots.clone();
    ascending.sort_unstable();
    assert_eq!(
        slots, ascending,
        "the batch is not in ledger order: {slots:?}"
    );

    // Without this the test is vacuous, and silently so — which is how it
    // passed against a deleted `reverse()`.
    let shared = slots
        .iter()
        .filter(|slot| slots.iter().filter(|s| s == slot).count() > 1)
        .count();
    assert!(
        shared >= 2,
        "no two transactions shared a slot, so intra-slot ordering was never \
         exercised and this test proves nothing: {slots:?}"
    );

    // The chain's running total is the newest deposit's. A batch reversed
    // inside a slot assigns an earlier one and stops there.
    let ingestor = live.ingest();
    let on_chain: helix_treasury::state::Treasury =
        live.cluster.account(treasury).expect("treasury account");
    assert_eq!(
        ingestor
            .head()
            .treasuries
            .get(&treasury)
            .expect("treasury is missing from the projection")
            .total_deposited,
        on_chain.total_deposited,
        "the projection settled on a running total from earlier in the slot"
    );
    assert_eq!(on_chain.total_deposited, amounts.iter().sum::<u64>());
}

/// A failed transaction is in the ledger, and its events are in its log.
///
/// Nothing it wrote survived, so folding those events credits a stake that never
/// landed. The signature listing reports the failure; the log does not.
///
/// # Why the transaction has two instructions
///
/// A transaction that fails on its *first* instruction never reaches the
/// `emit!`, so its log has no event in it and there is nothing for a missing
/// filter to fold. The first version of this test did exactly that and passed
/// with the filter deleted. The case that matters is a transaction that gets
/// part way — instruction one stakes and emits `Staked`, instruction two fails,
/// and the runtime rolls back the writes while the log keeps every line.
#[test]
fn a_failed_transaction_contributes_nothing() {
    let mut live = Live::bootstrap(cluster_or_skip!());
    let (owner, tokens) = live.staker(5_000_000);
    live.stake(&owner, &tokens, 0, 1_000_000, LockTier::Flexible);

    let before = live.ingest().head().tvl(&live.pool);
    assert_eq!(before, 1_000_000);

    let succeeds = live.stake_ix(&owner.pubkey(), &tokens, 1, 2_000_000, LockTier::Flexible);
    // Same id as the instruction before it, so the pool's counter has already
    // moved past it: `UnexpectedPositionId`, after the first instruction has
    // emitted its event.
    let fails = live.stake_ix(&owner.pubkey(), &tokens, 1, 1_000_000, LockTier::Flexible);

    let failure = live
        .cluster
        .send_expecting_failure(&[succeeds, fails], &[&owner])
        .expect_err("the second instruction reuses a spent position id");
    let ClusterError::Failed { signature, logs } = failure else {
        panic!("the transaction never reached the ledger, so nothing was proven: {failure}");
    };

    // The premise, asserted rather than assumed: a rolled-back transaction
    // whose log carries an event.
    assert!(
        logs.iter().any(|line| line.starts_with("Program data: ")),
        "the failed transaction emitted no event, so the filter was never \
         exercised: {logs:#?}"
    );

    let after = live.ingest().head().tvl(&live.pool);
    assert_eq!(
        after, before,
        "{signature} was rolled back and its events were folded anyway"
    );

    let pool: Pool = live.cluster.account(live.pool).expect("pool account");
    assert_eq!(after, pool.total_staked);
}

/// Resuming from a real, finalised cursor must not replay or false-alarm.
///
/// The cursor is one signature for the whole protocol while the source follows
/// four addresses, so it names a transaction three of them have never seen. If
/// that made any of them serve settled history, the ingestor would not read it
/// as redelivery — it compares each batch against the prefix it applied, so it
/// would read it as a fork, or refuse outright as
/// `FinalizedHistoryChanged`.
///
/// It does not, and measurement is the reason that is known rather than
/// assumed: Agave resolves `until` to the signature's slot globally, so the
/// signature form would have been safe here too. `docs/TESTING.md` records that
/// mutation as surviving, deliberately. What this test does pin down is the
/// behaviour that has to hold whichever form is used — a cursor advanced by
/// real finality, across four addresses, costing exactly the transactions that
/// are new.
#[test]
fn a_cursor_from_one_program_does_not_replay_the_others() {
    let mut live = Live::bootstrap(cluster_or_skip!());
    let (owner, tokens) = live.staker(10_000_000);
    live.stake(&owner, &tokens, 0, 1_000_000, LockTier::Flexible);

    // The bootstrap touched staking, governance and treasury; the stakes
    // touched only staking. Any cursor here names a transaction that at least
    // one followed address has never seen.
    let mut source = RpcLogSource::new(live.cluster.url());
    let mut ingestor = Ingestor::new();

    // Polling until nothing new arrives is not enough: the cursor only advances
    // as slots *finalise*, which lags confirmation by tens of slots. An earlier
    // version stopped at the first quiet poll, left the cursor at its default,
    // and so never sent the source a cursor at all — which is why it passed with
    // the stop condition reverted to a signature.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while ingestor.cursor().slot == 0 || ingestor.pending_count() > 0 {
        ingestor.poll(&mut source, 500).expect("poll");
        assert!(
            std::time::Instant::now() < deadline,
            "nothing finalised in 90s, so the cursor never moved off its default"
        );
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    let resumed_from = ingestor.cursor().clone();

    live.stake(&owner, &tokens, 1, 2_000_000, LockTier::Flexible);

    let outcome = ingestor
        .poll(&mut source, 500)
        .unwrap_or_else(|e| panic!("resuming from {resumed_from:?} was refused outright: {e:?}"));
    assert!(
        !outcome.was_reorg(),
        "re-reading settled history was mistaken for a fork: reverted {:?}",
        outcome.reverted
    );
    assert_eq!(
        outcome.applied, 1,
        "expected exactly the one new transaction, got {}",
        outcome.applied
    );

    let pool: Pool = live.cluster.account(live.pool).expect("pool account");
    assert_eq!(ingestor.head().tvl(&live.pool), pool.total_staked);
}

/// The watermark has to come from the cluster, and it has to move.
#[test]
fn finality_is_read_from_the_cluster_and_advances() {
    let cluster = cluster_or_skip!();
    let mut source = RpcLogSource::new(cluster.url());

    let first = source.finalized_slot().expect("finalized slot");
    assert!(first > 0, "a running validator reported slot 0 as final");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let advanced = loop {
        let now = source.finalized_slot().expect("finalized slot");
        if now > first {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    assert!(advanced, "the finalized slot never moved past {first}");
}

/// One transaction, three programs, four address queries — folded once.
///
/// The bootstrap is a single transaction that initialises a pool, a realm and a
/// treasury, so three of the four followed addresses return the same signature.
/// Both halves matter: every program's events have to survive the trip, and the
/// transaction must not be counted once per address that reported it. Deduping
/// on the signature is what makes following several addresses safe at all.
#[test]
fn a_transaction_reported_by_three_addresses_is_folded_once() {
    let live = Live::bootstrap(cluster_or_skip!());
    let ingestor = live.ingest();
    let head = ingestor.head();

    assert!(
        head.pools.contains_key(&live.pool),
        "the pool was not indexed"
    );

    let (treasury, _) = pda::treasury(&live.mint);
    assert!(
        head.treasuries.contains_key(&treasury),
        "the treasury was not indexed, so the bootstrap's later events were lost"
    );

    // Folding the same transaction once per address that listed it would leave
    // the state identical — the projection assigns rather than accumulates — and
    // show up only here, as three times the events.
    let mut source = RpcLogSource::new(live.cluster.url());
    let batch = source.fetch(&Default::default(), 500).expect("fetch");
    let unique: std::collections::HashSet<&String> = batch.iter().map(|tx| &tx.signature).collect();
    assert_eq!(
        unique.len(),
        batch.len(),
        "a transaction was returned once per address it touched"
    );

    assert_eq!(
        Program::ALL.len(),
        4,
        "a program was added without the RPC source being told to follow it"
    );
}

/// Phase 4.3 — the descent, against the API it was written for.
///
/// `getSignaturesForAddress` pages backwards and counts `limit` from the newest
/// end, which is why [`LogSource::fetch`] has to descend to its cursor and
/// reverse. A backfill wants exactly what the API offers, so the interesting
/// question is not whether it can read — it is whether the two traversals see
/// the *same history*.
///
/// That is what is asserted: a descent from the tip to genesis yields the same
/// set of transactions the live poll does, and replaying it reconstructs the
/// same projection. A backfill that quietly skipped a page would still produce
/// plausible numbers, and only a comparison against the other direction finds
/// it.
#[test]
fn a_descent_sees_the_same_history_the_live_poll_does() {
    let mut live = Live::bootstrap(cluster_or_skip!());

    let (owner, tokens) = live.staker(10_000_000);
    live.stake(&owner, &tokens, 0, 1_000_000, LockTier::Flexible);
    live.stake(&owner, &tokens, 1, 2_500_000, LockTier::Silver);

    // Wait for finality: the descent refuses to claim the unfinalised tail, and
    // a test that did not wait would compare a complete forward pass against a
    // descent missing its most recent slots.
    live.cluster
        .wait_for_finality()
        .expect("the cluster did not finalise");

    let forward = live.ingest();

    // A small page on purpose. The whole ledger fits in one request otherwise,
    // and a paging bug is precisely what this is looking for.
    let mut source = RpcLogSource::new(live.cluster.url()).with_page_size(3);
    let mut backfill = Backfill::new();
    let mut descended: Vec<SettledTransaction> = Vec::new();

    for _ in 0..256 {
        let batch = backfill.step(&mut source, 3).expect("descend");
        let mut older = batch.transactions;
        older.extend(descended);
        descended = older;
        if batch.complete {
            break;
        }
    }
    assert!(
        backfill.is_complete(),
        "the descent did not reach genesis in 256 pages"
    );

    // Slots ascend, which is the contract `DescendingSource` states and the one
    // `Analytics::replay` depends on. Getting this wrong produces a projection
    // that is merely incorrect.
    let slots: Vec<u64> = descended.iter().map(|t| t.slot).collect();
    assert!(
        slots.windows(2).all(|w| w[0] <= w[1]),
        "the descent handed back transactions out of ledger order: {slots:?}"
    );

    let rebuilt = helix_indexer::Analytics::replay(
        descended
            .iter()
            .map(|t| (t.signature.as_str(), t.events.as_slice())),
    );

    assert_eq!(
        rebuilt.tvl(&live.pool),
        forward.finalized().tvl(&live.pool),
        "the two traversals disagree about how much is staked"
    );
    assert_eq!(
        rebuilt.applied_count(),
        forward.finalized().applied_count(),
        "the descent saw a different number of events than the live poll"
    );
    assert_eq!(
        rebuilt.pools.len(),
        forward.finalized().pools.len(),
        "the descent missed a pool"
    );

    // And against the chain, not only against the other traversal — two
    // traversals sharing a bug would agree with each other perfectly.
    let pool: Pool = live.cluster.account(live.pool).expect("pool account");
    assert_eq!(rebuilt.tvl(&live.pool), pool.total_staked);
}

/// A descent that stops mid-history resumes from what it persisted.
///
/// The `backfill` cursor is the second one the schema has always had room for,
/// and the property that makes it worth storing is this: a process restarted
/// half way down does not begin again at the tip. Asserted by descending in two
/// runs and comparing against one.
#[test]
fn a_descent_resumes_from_its_persisted_position() {
    let mut live = Live::bootstrap(cluster_or_skip!());
    let (owner, tokens) = live.staker(10_000_000);
    live.stake(&owner, &tokens, 0, 1_000_000, LockTier::Flexible);
    live.cluster
        .wait_for_finality()
        .expect("the cluster did not finalise");

    let descend = |from: Option<helix_indexer::Descent>, pages: usize| {
        let mut source = RpcLogSource::new(live.cluster.url()).with_page_size(2);
        let mut backfill = from.map_or_else(Backfill::new, Backfill::resume_at);
        let mut collected: Vec<SettledTransaction> = Vec::new();

        for _ in 0..pages {
            let batch = backfill.step(&mut source, 2).expect("descend");
            let mut older = batch.transactions;
            older.extend(collected);
            collected = older;
            if batch.complete {
                break;
            }
        }
        (
            backfill.descent().clone(),
            collected,
            backfill.is_complete(),
        )
    };

    let (_, single, complete) = descend(None, 256);
    assert!(complete, "the one-shot descent did not finish");

    // Two pages, then stop — the only thing carried across is what a store would
    // have written to the `backfill` row.
    let (persisted, first_half, done) = descend(None, 2);
    assert!(!done, "the ledger is too short to test resumption");

    let (_, second_half, finished) = descend(Some(persisted), 256);
    assert!(finished, "the resumed descent did not finish");

    let mut rejoined = second_half;
    rejoined.extend(first_half);

    assert_eq!(
        rejoined.iter().map(|t| &t.signature).collect::<Vec<_>>(),
        single.iter().map(|t| &t.signature).collect::<Vec<_>>(),
        "resuming produced a different history than descending in one run"
    );
}
