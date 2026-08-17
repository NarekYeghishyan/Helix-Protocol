//! A minimal client for driving a real validator, for the tests that need one.
//!
//! Almost everything in this suite runs against LiteSVM, which is in-process,
//! millisecond-fast and cannot be asked for an RPC endpoint. A handful of claims
//! are not about the programs but about *reading them over JSON-RPC*, and those
//! need a cluster — [`rpc_source_live.rs`](../tests/rpc_source_live.rs) is the
//! whole of it.
//!
//! # Scope, deliberately small
//!
//! Enough to submit a transaction and read an account back. No retries beyond
//! confirmation, no priority fees, no versioned transactions, no address lookup
//! tables. This is test scaffolding for a local validator, not a client library;
//! anything more is a second implementation of something that already exists.
//!
//! It speaks JSON-RPC directly for the same reason
//! [`helix_indexer::rpc`](../../../indexer/src/rpc.rs) does — `solana-rpc-client`
//! is published at 4.2.0-rc against a workspace resolving the Solana crates at
//! 3.x. The types needed to *build and sign* a transaction are already here at
//! matching versions; only the socket was missing.
//!
//! # Skipping rather than failing
//!
//! [`Cluster::from_env`] returns `None` when `HELIX_RPC_URL` is unset, and the
//! tests that use it return early. A suite that fails when no validator is
//! running would make the default `cargo test` red on every machine that has not
//! started one, and a red suite that is expected to be red stops being read.

use anchor_lang::prelude::Pubkey;
use anchor_lang::AccountDeserialize;
use serde_json::{json, Value};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::str::FromStr as _;
use std::time::{Duration, Instant};

/// Set this to a validator's RPC URL to enable the live tests.
pub const RPC_URL_ENV: &str = "HELIX_RPC_URL";

pub struct Cluster {
    url: String,
    agent: ureq::Agent,
    payer: Keypair,
}

#[derive(Debug)]
pub enum ClusterError {
    Transport(String),
    Rpc(String),
    /// The transaction was submitted and did not confirm inside the deadline.
    ConfirmationTimeout(String),
    /// The transaction confirmed and failed. Carries the logs, because a bare
    /// `custom program error: 0x1771` is not a diagnosis.
    Failed {
        signature: String,
        logs: Vec<String>,
    },
}

impl std::fmt::Display for ClusterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Rpc(e) => write!(f, "rpc: {e}"),
            Self::ConfirmationTimeout(s) => write!(f, "{s} did not confirm"),
            Self::Failed { signature, logs } => {
                writeln!(f, "{signature} failed:")?;
                for line in logs {
                    writeln!(f, "  {line}")?;
                }
                Ok(())
            }
        }
    }
}

type Result<T> = std::result::Result<T, ClusterError>;

impl Cluster {
    /// A client for the validator named by `HELIX_RPC_URL`, or `None`.
    ///
    /// The payer is generated per run and airdropped, so two runs against the
    /// same validator do not share positions, proposals or nonce-like counters —
    /// a suite that only passes against a fresh ledger is a suite nobody can run
    /// twice.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var(RPC_URL_ENV).ok()?;
        let cluster = Self {
            url,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(30)))
                    .build(),
            ),
            payer: Keypair::new(),
        };
        cluster.airdrop(cluster.payer.pubkey(), 500).ok()?;
        Some(cluster)
    }

    pub fn payer(&self) -> &Keypair {
        &self.payer
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut response = self
            .agent
            .post(&self.url)
            .send_json(json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
            }))
            .map_err(|e| ClusterError::Transport(e.to_string()))?;

        let envelope: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| ClusterError::Transport(e.to_string()))?;

        if let Some(error) = envelope.get("error") {
            return Err(ClusterError::Rpc(error.to_string()));
        }
        envelope
            .get("result")
            .cloned()
            .ok_or_else(|| ClusterError::Rpc("no result".into()))
    }

    pub fn airdrop(&self, to: Pubkey, sol: u64) -> Result<()> {
        let signature = self
            .call(
                "requestAirdrop",
                json!([to.to_string(), sol * 1_000_000_000]),
            )?
            .as_str()
            .ok_or_else(|| ClusterError::Rpc("airdrop returned no signature".into()))?
            .to_owned();
        self.confirm(&signature)?;
        Ok(())
    }

    pub fn latest_blockhash(&self) -> Result<[u8; 32]> {
        let value = self.call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))?;
        let encoded = value
            .pointer("/value/blockhash")
            .and_then(Value::as_str)
            .ok_or_else(|| ClusterError::Rpc("no blockhash".into()))?;
        let hash = Pubkey::from_str(encoded)
            .map_err(|e| ClusterError::Rpc(format!("undecodable blockhash: {e}")))?;
        Ok(hash.to_bytes())
    }

    pub fn rent_exemption(&self, space: usize) -> Result<u64> {
        self.call("getMinimumBalanceForRentExemption", json!([space]))?
            .as_u64()
            .ok_or_else(|| ClusterError::Rpc("rent exemption returned no number".into()))
    }

    /// Signs, submits and waits for confirmation. Returns the signature.
    pub fn send(&self, instructions: &[Instruction], signers: &[&Keypair]) -> Result<String> {
        self.submit(instructions, signers, false)
    }

    /// The same, with preflight skipped, so a transaction that will fail is
    /// still recorded in the ledger.
    ///
    /// Preflight simulates and rejects before submission, which is what any real
    /// client wants and is useless for testing what an indexer does with a
    /// failed transaction: a rejected transaction never lands, so there is
    /// nothing to read back. Returns `Err(ClusterError::Failed)` on the failure
    /// it was asked to produce.
    pub fn send_expecting_failure(
        &self,
        instructions: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<String> {
        self.submit(instructions, signers, true)
    }

    /// Submits without waiting, so several transactions can be in flight at
    /// once and land in the same slot.
    ///
    /// Confirming between sends guarantees they do not: a validator producing a
    /// slot every 400ms puts each confirmed-then-sent transaction in a slot of
    /// its own, and a test that never sees two transactions share one cannot
    /// tell ledger order from slot order. Pair with [`Self::confirm`].
    pub fn send_nowait(
        &self,
        instructions: &[Instruction],
        signers: &[&Keypair],
        blockhash: [u8; 32],
    ) -> Result<String> {
        self.submit_with(instructions, signers, blockhash, false)
    }

    fn submit(
        &self,
        instructions: &[Instruction],
        signers: &[&Keypair],
        skip_preflight: bool,
    ) -> Result<String> {
        let blockhash = self.latest_blockhash()?;
        let signature = self.submit_with(instructions, signers, blockhash, skip_preflight)?;
        self.confirm(&signature)?;
        Ok(signature)
    }

    fn submit_with(
        &self,
        instructions: &[Instruction],
        signers: &[&Keypair],
        blockhash: [u8; 32],
        skip_preflight: bool,
    ) -> Result<String> {
        let mut all: Vec<&Keypair> = vec![&self.payer];
        all.extend(signers.iter().filter(|k| k.pubkey() != self.payer.pubkey()));

        let transaction = Transaction::new_signed_with_payer(
            instructions,
            Some(&self.payer.pubkey()),
            &all,
            blockhash.into(),
        );
        let wire = bincode::serialize(&transaction)
            .map_err(|e| ClusterError::Rpc(format!("serialize: {e}")))?;

        let signature = self
            .call(
                "sendTransaction",
                json!([
                    base64_encode(&wire),
                    {
                        "encoding": "base64",
                        // Preflight simulates before submitting, so an ordinary
                        // mistake surfaces as an error here rather than as a
                        // confirmed failure thirty seconds later.
                        "skipPreflight": skip_preflight,
                        "preflightCommitment": "confirmed",
                    }
                ]),
            )?
            .as_str()
            .ok_or_else(|| ClusterError::Rpc("sendTransaction returned no signature".into()))?
            .to_owned();

        Ok(signature)
    }

    /// Waits until the signature is confirmed, or the deadline passes.
    pub fn confirm(&self, signature: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let statuses = self.call(
                "getSignatureStatuses",
                json!([[signature], { "searchTransactionHistory": true }]),
            )?;
            let status = statuses.pointer("/value/0");

            if let Some(status) = status.filter(|s| !s.is_null()) {
                let confirmed = status
                    .get("confirmationStatus")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s == "confirmed" || s == "finalized");

                if confirmed {
                    if status.get("err").is_some_and(|e| !e.is_null()) {
                        return Err(ClusterError::Failed {
                            signature: signature.to_owned(),
                            logs: self.logs_of(signature).unwrap_or_default(),
                        });
                    }
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(ClusterError::ConfirmationTimeout(signature.to_owned()))
    }

    fn logs_of(&self, signature: &str) -> Result<Vec<String>> {
        let value = self.call(
            "getTransaction",
            json!([signature, {
                "encoding": "json",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0,
            }]),
        )?;
        Ok(value
            .pointer("/meta/logMessages")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The slot the cluster has finalised.
    pub fn finalized_slot(&self) -> Result<u64> {
        self.call("getSlot", json!([{ "commitment": "finalized" }]))?
            .as_u64()
            .ok_or_else(|| ClusterError::Rpc("getSlot returned no number".into()))
    }

    /// Blocks until finality has caught up with the current confirmed tip.
    ///
    /// For tests that compare something which only reads finalised history
    /// against something which reads to the tip. Without this the two are being
    /// asked about different ranges and the comparison fails intermittently,
    /// which is worse than failing — an intermittent test gets re-run rather than
    /// read.
    ///
    /// Polls rather than subscribes because the whole `Cluster` harness is
    /// request/response; a local validator finalises in a couple of seconds.
    pub fn wait_for_finality(&self) -> Result<u64> {
        let target = self
            .call("getSlot", json!([{ "commitment": "confirmed" }]))?
            .as_u64()
            .ok_or_else(|| ClusterError::Rpc("getSlot returned no number".into()))?;

        for _ in 0..120 {
            let finalized = self.finalized_slot()?;
            if finalized >= target {
                return Ok(finalized);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        Err(ClusterError::Rpc(format!(
            "the cluster did not finalise slot {target} within 60s"
        )))
    }

    /// Reads an Anchor account back off the chain.
    ///
    /// The point of the live test is comparing the projection against what the
    /// programs actually wrote, which means reading the accounts rather than
    /// trusting the events twice.
    pub fn account<T: AccountDeserialize>(&self, address: Pubkey) -> Result<T> {
        let value = self.call(
            "getAccountInfo",
            json!([address.to_string(), { "encoding": "base64", "commitment": "confirmed" }]),
        )?;
        let encoded = value
            .pointer("/value/data/0")
            .and_then(Value::as_str)
            .ok_or_else(|| ClusterError::Rpc(format!("{address} has no data")))?;
        let bytes = base64_decode(encoded)
            .ok_or_else(|| ClusterError::Rpc(format!("{address} data is not base64")))?;
        T::try_deserialize(&mut bytes.as_slice())
            .map_err(|e| ClusterError::Rpc(format!("{address} did not deserialise: {e}")))
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.encode(bytes)
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.decode(text).ok()
}

/// Returns from the calling test when no validator is configured.
///
/// A macro rather than a helper returning `Option`, so the early return is at
/// the call site and cannot be forgotten by binding the result and ignoring it.
#[macro_export]
macro_rules! cluster_or_skip {
    () => {
        match $crate::cluster::Cluster::from_env() {
            Some(cluster) => cluster,
            None => {
                eprintln!(
                    "skipped: set {} to a validator's RPC URL to run this test",
                    $crate::cluster::RPC_URL_ENV
                );
                return;
            }
        }
    };
}
