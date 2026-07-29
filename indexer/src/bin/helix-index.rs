//! Follows a cluster and reports what it has ingested.
//!
//! The operator-facing end of Phase 4.1. It holds the projection in memory and
//! prints on every change — there is no database binding yet (Phase 4.2), and
//! this says so rather than implying durability it does not have.
//!
//! ```text
//! helix-index --url http://127.0.0.1:8899 [--interval 2] [--once]
//! ```
//!
//! # What it prints, and why that and not more
//!
//! Three numbers an operator needs to decide whether to trust a dashboard:
//!
//! - **The gap between head and finalized.** How many transactions a fork could
//!   still take back. A dashboard reading `head` is showing figures that include
//!   these.
//! - **Anomalies.** Truncated logs and payloads this build cannot decode. Both
//!   mean the projection is missing something, and neither shows up as an error
//!   anywhere else.
//! - **Orphans.** Events referring to entities never seen created, which is what
//!   a dropped transaction looks like from the inside.
//!
//! A reorg is reported when it happens rather than counted, because the useful
//! question after one is "what changed", not "how many".

use helix_indexer::rpc::RpcLogSource;
use helix_indexer::{IngestError, Ingestor};
use std::time::Duration;

struct Args {
    url: String,
    interval: Duration,
    once: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut url = None;
    let mut interval = 2u64;
    let mut once = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = Some(args.next().ok_or("--url needs a value")?),
            "--interval" => {
                interval = args
                    .next()
                    .ok_or("--interval needs a value")?
                    .parse()
                    .map_err(|_| "--interval must be a whole number of seconds")?
            }
            "--once" => once = true,
            "--help" | "-h" => return Err("help".into()),
            other => return Err(format!("unrecognised argument: {other}")),
        }
    }

    Ok(Args {
        url: url.ok_or("--url is required")?,
        interval: Duration::from_secs(interval.max(1)),
        once,
    })
}

fn usage() {
    eprintln!(
        "helix-index --url <rpc> [--interval <seconds>] [--once]\n\
         \n\
         Follows the four Helix programs on <rpc> and folds their events into an\n\
         in-memory projection. State is not persisted — see indexer/sql/schema.sql\n\
         for the shape it will take once Phase 4.2 binds it to a database."
    );
}

fn main() -> std::process::ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            if message != "help" {
                eprintln!("error: {message}\n");
            }
            usage();
            return std::process::ExitCode::from(2);
        }
    };

    let mut source = RpcLogSource::new(&args.url);
    let mut ingestor = Ingestor::new();
    println!("following {} — Ctrl-C to stop", args.url);

    loop {
        match ingestor.poll(&mut source, 500) {
            Ok(outcome) => {
                if let Some(reverted) = outcome.reverted {
                    println!(
                        "reorg: {reverted} transaction(s) rolled back and replayed from the \
                         finalized projection"
                    );
                }
                for anomaly in &outcome.anomalies {
                    println!("anomaly: {anomaly:?}");
                }
                // `--once` is a question, so it always gets an answer. The
                // watch loop prints only on change, because a line every two
                // seconds saying nothing happened is how a log stops being
                // read.
                if args.once || outcome.applied > 0 || outcome.finalized > 0 {
                    let head = ingestor.head();
                    println!(
                        "+{} applied, {} finalized | cursor slot {} | {} unfinalized | \
                         {} pools, {} positions, {} proposals, {} treasuries | {} orphaned",
                        outcome.applied,
                        outcome.finalized,
                        ingestor.cursor().slot,
                        ingestor.pending_count(),
                        head.pools.len(),
                        head.positions.len(),
                        head.proposals.len(),
                        head.treasuries.len(),
                        head.orphaned.len(),
                    );
                }
            }
            Err(IngestError::FinalizedHistoryChanged { slot, signature }) => {
                // Not recoverable by retrying: either the node is serving a
                // different ledger or the stored cursor belongs to one. Both
                // make every figure downstream suspect, so stopping loudly beats
                // continuing quietly.
                eprintln!(
                    "fatal: finalised history changed at slot {slot} ({signature}).\n\
                     The cursor and this endpoint do not agree on settled history — check \
                     that the URL is the cluster this cursor was built against."
                );
                return std::process::ExitCode::FAILURE;
            }
            Err(IngestError::Source(e)) => {
                eprintln!("source error: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }

        if args.once {
            return std::process::ExitCode::SUCCESS;
        }
        std::thread::sleep(args.interval);
    }
}
