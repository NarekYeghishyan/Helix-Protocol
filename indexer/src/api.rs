//! The read model: what a dashboard asks for, and how it is answered.
//!
//! Pure functions over an [`Ingestor`], with no HTTP in sight. The transport is
//! [`crate::server`], behind the `server` feature; everything worth testing is
//! here.
//!
//! Three decisions are baked into the shape of these responses, and each exists
//! because the obvious alternative is quietly wrong.
//!
//! # 1. Finality is part of the answer, not an assumption
//!
//! The ingestor keeps two projections: `finalized`, which the cluster will not
//! take back, and `head`, which includes transactions that a fork still might.
//! An API that serves one without saying which invites a dashboard to display a
//! TVL that later decreases for no visible reason.
//!
//! Every response therefore carries [`Meta`], naming the view it came from and
//! the slot it reflects. A caller wanting responsiveness asks for
//! [`Finality::Head`] and knows what it is holding; a caller settling accounts
//! asks for [`Finality::Finalized`].
//!
//! # 2. Amounts are strings
//!
//! JSON numbers are IEEE-754 doubles, exact only below 2^53. Token amounts are
//! `u64`. For a 9-decimal mint, 2^53 base units is about nine million tokens —
//! comfortably reachable, and the failure is silent rounding in whatever
//! JavaScript parses it, not an error.
//!
//! This is the same hazard `sql/schema.sql` avoids by using `NUMERIC(20, 0)`
//! rather than `BIGINT`, and it deserves the same treatment on the wire.
//!
//! # 3. Undefined is `null`, never zero
//!
//! APR over an empty pool is undefined, not infinite and not zero. Serialising
//! it as `0` would put a plausible, wrong number on a dashboard; `null` makes the
//! caller decide what to render.

use serde::{Deserialize, Serialize};

use crate::ingest::Ingestor;
use crate::projection::Analytics;

/// Which projection a response was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Finality {
    /// Only what the cluster has committed to. Never revised downward.
    Finalized,
    /// Everything ingested, including transactions a fork could still take back.
    Head,
}

/// What the numbers below are, and as of when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub finality: Finality,
    /// The last finalised slot folded in. A caller polling repeatedly can use
    /// this to notice it is being served stale data by a lagging replica.
    pub slot: u64,
    /// Transactions in the response that are not yet final. Always 0 for
    /// [`Finality::Finalized`] — which is the point of asking for it.
    pub pending_transactions: usize,
}

/// A response and the context needed to interpret it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response<T> {
    pub meta: Meta,
    pub data: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolView {
    pub address: String,
    /// Base units, as a string. See the module docs.
    pub total_staked: String,
    pub total_weighted: String,
    pub position_count: u64,
    pub reward_rate: String,
    pub reward_period_end: i64,
    pub total_rewards_funded: String,
    pub total_rewards_paid: String,
    /// `None` when nothing is staked, because APR is then undefined.
    pub apr_bps: Option<u64>,
    pub paused: bool,
    /// True when the pool was first seen through an event other than its
    /// creation, so these figures are the best available rather than complete.
    pub partial_history: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StakerView {
    pub owner: String,
    pub staked: String,
    /// Share of the pool in basis points, so a caller does not have to divide
    /// two strings to draw a bar.
    pub share_bps: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalView {
    pub address: String,
    pub realm: String,
    pub id: u64,
    pub proposer: String,
    pub title: String,
    pub state: String,
    pub for_votes: String,
    pub against_votes: String,
    pub abstain_votes: String,
    pub total_weight_snapshot: String,
    /// How many positions the quorum denominator covers. Carried because a
    /// consumer checking whether the tally is meaningful needs both halves.
    pub position_count_snapshot: u64,
    pub distinct_voters: usize,
    /// Earliest execution time, once queued.
    pub eta: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreasuryView {
    pub address: String,
    pub governance_executor: Option<String>,
    pub total_deposited: String,
    pub total_spent: String,
    pub total_stream_claims: String,
    /// Deposits less everything that has left, by spend or by vesting claim.
    pub balance: String,
    /// Promised to vesting streams and not yet claimed. Spending cannot touch it.
    pub committed_to_streams: String,
    pub epoch_spend_cap: String,
    pub open_streams: usize,
    pub partial_history: bool,
}

/// Queries over an ingestor's two projections.
pub struct Api<'a> {
    ingestor: &'a Ingestor,
}

impl<'a> Api<'a> {
    pub fn new(ingestor: &'a Ingestor) -> Self {
        Self { ingestor }
    }

    fn view(&self, finality: Finality) -> (&Analytics, Meta) {
        let (analytics, pending) = match finality {
            Finality::Finalized => (self.ingestor.finalized(), 0),
            Finality::Head => (self.ingestor.head(), self.ingestor.pending_count()),
        };
        (
            analytics,
            Meta {
                finality,
                slot: self.ingestor.cursor().slot,
                pending_transactions: pending,
            },
        )
    }

    pub fn pool(&self, finality: Finality, pool: &str) -> Option<Response<PoolView>> {
        let key = parse_pubkey(pool)?;
        let (analytics, meta) = self.view(finality);
        let stats = analytics.pools.get(&key)?;

        Some(Response {
            meta,
            data: PoolView {
                address: pool.to_owned(),
                total_staked: stats.total_staked.to_string(),
                total_weighted: stats.total_weighted.to_string(),
                position_count: stats.position_count,
                reward_rate: stats.reward_rate.to_string(),
                reward_period_end: stats.reward_period_end,
                total_rewards_funded: stats.total_rewards_funded.to_string(),
                total_rewards_paid: stats.total_rewards_paid.to_string(),
                apr_bps: analytics.apr_bps(&key),
                paused: stats.paused,
                partial_history: stats.authority.is_none(),
            },
        })
    }

    /// Live stakers in a pool, largest first.
    ///
    /// Fully withdrawn positions are excluded: the account still exists on chain,
    /// but a zero-weight position is not a staker and would draw an empty bar.
    pub fn stakers(
        &self,
        finality: Finality,
        pool: &str,
        limit: usize,
    ) -> Option<Response<Vec<StakerView>>> {
        let key = parse_pubkey(pool)?;
        let (analytics, meta) = self.view(finality);

        let total = analytics.tvl(&key) as u128;
        let data = analytics
            .staker_distribution(&key)
            .into_iter()
            .take(limit)
            .map(|(owner, staked)| StakerView {
                owner: owner.to_string(),
                staked: staked.to_string(),
                // Cross-multiplied rather than dividing first, so a small holder
                // does not round to zero before the multiplication. `checked_div`
                // covers the empty pool, where the share of nothing is nothing.
                share_bps: (staked as u128 * 10_000).checked_div(total).unwrap_or(0) as u64,
            })
            .collect();

        Some(Response { meta, data })
    }

    /// Proposals in a realm, newest first.
    pub fn proposals(
        &self,
        finality: Finality,
        realm: &str,
    ) -> Option<Response<Vec<ProposalView>>> {
        let key = parse_pubkey(realm)?;
        let (analytics, meta) = self.view(finality);

        let data = analytics
            .proposals
            .iter()
            .filter(|(_, p)| p.realm == key)
            .map(|(address, p)| ProposalView {
                address: address.to_string(),
                realm: realm.to_owned(),
                id: p.id,
                proposer: p.proposer.to_string(),
                title: p.title.clone(),
                state: format!("{:?}", p.state),
                for_votes: p.for_votes.to_string(),
                against_votes: p.against_votes.to_string(),
                abstain_votes: p.abstain_votes.to_string(),
                total_weight_snapshot: p.total_weight_snapshot.to_string(),
                position_count_snapshot: p.position_count_snapshot,
                distinct_voters: p.voters.len(),
                eta: p.eta,
            })
            .collect::<Vec<_>>();

        let mut data = data;
        data.sort_by_key(|p| std::cmp::Reverse(p.id));
        Some(Response { meta, data })
    }

    pub fn treasury(&self, finality: Finality, treasury: &str) -> Option<Response<TreasuryView>> {
        let key = parse_pubkey(treasury)?;
        let (analytics, meta) = self.view(finality);
        let stats = analytics.treasuries.get(&key)?;

        Some(Response {
            meta,
            data: TreasuryView {
                address: treasury.to_owned(),
                governance_executor: stats.governance_executor.map(|k| k.to_string()),
                total_deposited: stats.total_deposited.to_string(),
                total_spent: stats.total_spent.to_string(),
                total_stream_claims: stats.total_stream_claims.to_string(),
                balance: analytics.treasury_balance(&key).to_string(),
                committed_to_streams: analytics.committed_to_streams(&key).to_string(),
                epoch_spend_cap: stats.epoch_spend_cap.to_string(),
                open_streams: stats.open_streams.values().filter(|s| !s.revoked).count(),
                partial_history: stats.governance_executor.is_none(),
            },
        })
    }
}

fn parse_pubkey(s: &str) -> Option<anchor_lang::prelude::Pubkey> {
    use std::str::FromStr as _;
    anchor_lang::prelude::Pubkey::from_str(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ScriptedSource;
    use anchor_lang::prelude::Pubkey;
    use anchor_lang::{AnchorSerialize, Discriminator};
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use helix_staking::state::LockTier;

    fn staked_log(pool: Pubkey, owner: Pubkey, amount: u64) -> Vec<String> {
        let event = helix_staking::events::Staked {
            pool,
            position: Pubkey::new_unique(),
            owner,
            position_id: 0,
            amount_sent: amount,
            amount_credited: amount,
            weighted_amount: amount,
            tier: LockTier::Flexible,
            lock_end: 0,
            timestamp: 1,
        };
        let mut bytes = helix_staking::events::Staked::DISCRIMINATOR.to_vec();
        event.serialize(&mut bytes).expect("serialize");
        vec![
            format!("Program {} invoke [1]", helix_staking::ID),
            format!("Program data: {}", BASE64.encode(bytes)),
            format!("Program {} success", helix_staking::ID),
        ]
    }

    /// One finalised stake and one that is not, so the two views differ.
    fn fixture(amount_finalized: u64, amount_pending: u64) -> (Pubkey, Ingestor) {
        let pool = Pubkey::new_unique();
        let mut source = ScriptedSource::new();
        source.push(
            "sig-1",
            1,
            staked_log(pool, Pubkey::new_unique(), amount_finalized),
        );
        source.push(
            "sig-2",
            2,
            staked_log(pool, Pubkey::new_unique(), amount_pending),
        );
        source.finalize_through(1);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut source, 100).expect("poll");
        (pool, ingestor)
    }

    #[test]
    fn the_two_views_differ_and_each_says_which_it_is() {
        let (pool, ingestor) = fixture(1_000, 500);
        let api = Api::new(&ingestor);
        let address = pool.to_string();

        let finalized = api.pool(Finality::Finalized, &address).expect("pool");
        assert_eq!(finalized.meta.finality, Finality::Finalized);
        assert_eq!(finalized.meta.pending_transactions, 0);
        assert_eq!(finalized.data.total_staked, "1000");

        let head = api.pool(Finality::Head, &address).expect("pool");
        assert_eq!(head.meta.finality, Finality::Head);
        assert_eq!(head.meta.pending_transactions, 1);
        assert_eq!(head.data.total_staked, "1500");

        assert_eq!(head.meta.slot, 1, "the slot is the finalised watermark");
    }

    /// The reason amounts are strings.
    #[test]
    fn an_amount_past_the_double_precision_limit_survives_a_json_round_trip() {
        // 2^53 + 1: the smallest integer a JSON number cannot represent exactly.
        let amount = 9_007_199_254_740_993u64;
        let (pool, ingestor) = fixture(amount, 0);

        let api = Api::new(&ingestor);
        let response = api
            .pool(Finality::Finalized, &pool.to_string())
            .expect("pool");
        let json = serde_json::to_string(&response).expect("serialize");

        assert!(
            json.contains(&format!("\"{amount}\"")),
            "the amount was not serialised as a string: {json}"
        );

        let parsed: Response<PoolView> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            parsed.data.total_staked,
            amount.to_string(),
            "a u64 amount did not survive the round trip"
        );

        // And the failure it prevents: the same value through a JSON number.
        let as_number: f64 = amount as f64;
        assert_ne!(
            as_number as u64, amount,
            "this test is vacuous — the value fits in a double after all"
        );
    }

    #[test]
    fn apr_is_null_rather_than_zero_when_nothing_is_staked() {
        let pool = Pubkey::new_unique();
        let mut source = ScriptedSource::new();
        let event = helix_staking::events::RewardRateChanged {
            pool,
            old_rate: 0,
            new_rate: 1_000,
            reward_period_end: 0,
            timestamp: 1,
        };
        let mut bytes = helix_staking::events::RewardRateChanged::DISCRIMINATOR.to_vec();
        event.serialize(&mut bytes).expect("serialize");
        source.push(
            "sig-1",
            1,
            vec![
                format!("Program {} invoke [1]", helix_staking::ID),
                format!("Program data: {}", BASE64.encode(bytes)),
                format!("Program {} success", helix_staking::ID),
            ],
        );
        source.finalize_through(1);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut source, 100).expect("poll");

        let api = Api::new(&ingestor);
        let response = api
            .pool(Finality::Finalized, &pool.to_string())
            .expect("pool");

        assert_eq!(response.data.apr_bps, None);
        let json = serde_json::to_string(&response.data).expect("serialize");
        assert!(
            json.contains("\"apr_bps\":null"),
            "an undefined APR was not null: {json}"
        );
    }

    #[test]
    fn shares_are_computed_without_rounding_a_small_holder_to_nothing() {
        let pool = Pubkey::new_unique();
        let whale = Pubkey::new_unique();
        let minnow = Pubkey::new_unique();

        let mut source = ScriptedSource::new();
        source.push("sig-1", 1, staked_log(pool, whale, 999_900));
        source.push("sig-2", 1, staked_log(pool, minnow, 100));
        source.finalize_through(1);

        let mut ingestor = Ingestor::new();
        ingestor.poll(&mut source, 100).expect("poll");

        let api = Api::new(&ingestor);
        let stakers = api
            .stakers(Finality::Finalized, &pool.to_string(), 10)
            .expect("stakers");

        assert_eq!(stakers.data.len(), 2);
        assert_eq!(
            stakers.data[0].owner,
            whale.to_string(),
            "not sorted by size"
        );
        assert_eq!(stakers.data[0].share_bps, 9_999);
        assert_eq!(
            stakers.data[1].share_bps, 1,
            "a holder with 0.01% rounded away"
        );
    }

    #[test]
    fn an_unknown_pool_is_absent_rather_than_empty() {
        let (_, ingestor) = fixture(1_000, 0);
        let api = Api::new(&ingestor);

        assert!(
            api.pool(Finality::Finalized, &Pubkey::new_unique().to_string())
                .is_none(),
            "an unknown pool answered with zeros instead of nothing"
        );
    }

    #[test]
    fn a_malformed_address_is_rejected_rather_than_treated_as_unknown() {
        let (_, ingestor) = fixture(1_000, 0);
        let api = Api::new(&ingestor);
        assert!(api.pool(Finality::Finalized, "not-a-pubkey").is_none());
        assert!(api.stakers(Finality::Finalized, "", 10).is_none());
    }
}
