//! Follows a cluster and reports what it has ingested.
//!
//! ```text
//! helix-index --url http://127.0.0.1:8899 \
//!             [--database-url postgres://helix@localhost/helix] \
//!             [--interval 2] [--once]
//! ```
//!
//! Without `--database-url` the projection is in memory and dies with the
//! process, which is fine for a look at a cluster and useless as a service. With
//! one it resumes where the last run stopped, and says which of the two it is
//! doing on the first line — a service that silently starts from genesis because
//! a flag was missing is the failure this prints to prevent.
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
//!   anywhere else. Each is printed with its signature, because "line 12 was
//!   truncated" with no transaction to re-fetch is not actionable.
//! - **Orphans.** Events referring to entities never seen created, which is what
//!   a dropped transaction looks like from the inside.
//!
//! A reorg is reported when it happens rather than counted, because the useful
//! question after one is "what changed", not "how many".

use helix_indexer::rpc::RpcLogSource;
use helix_indexer::{IngestError, Ingestor, SettledTransaction};
use std::time::Duration;

#[cfg(feature = "postgres")]
use helix_indexer::Store;

struct Args {
    url: String,
    database_url: Option<String>,
    interval: Duration,
    once: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut url = None;
    let mut database_url = None;
    let mut interval = 2u64;
    let mut once = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = Some(args.next().ok_or("--url needs a value")?),
            "--database-url" => {
                database_url = Some(args.next().ok_or("--database-url needs a value")?)
            }
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

    // Rejected at parse time rather than ignored. A build without the feature
    // silently dropping the flag would start from genesis and write nothing,
    // while looking exactly like a run that is persisting.
    #[cfg(not(feature = "postgres"))]
    if database_url.is_some() {
        return Err("--database-url needs a build with the `postgres` feature".into());
    }

    Ok(Args {
        url: url.ok_or("--url is required")?,
        database_url,
        interval: Duration::from_secs(interval.max(1)),
        once,
    })
}

fn usage() {
    eprintln!(
        "helix-index --url <rpc> [--database-url <postgres url>] [--interval <seconds>] [--once]\n\
         \n\
         Follows the four Helix programs on <rpc> and folds their events into a\n\
         projection. Without --database-url that projection is in memory only and\n\
         is lost when the process stops; with one it is persisted as each slot\n\
         finalises and reloaded on the next run."
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

    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("fatal: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The store, or a stand-in that persists nothing.
///
/// Written as one type rather than two code paths so the loop below cannot drift
/// between "with a database" and "without" — the only difference between the two
/// runs should be whether the writes go anywhere.
enum Sink {
    Memory,
    #[cfg(feature = "postgres")]
    Postgres(Box<Store>),
}

impl Sink {
    fn open(database_url: Option<&str>) -> Result<(Self, Ingestor), String> {
        match database_url {
            None => Ok((Self::Memory, Ingestor::new())),
            #[cfg(feature = "postgres")]
            Some(url) => {
                let mut store = Store::connect(url).map_err(|e| e.to_string())?;
                store.migrate().map_err(|e| e.to_string())?;
                let restored = store.load().map_err(|e| e.to_string())?;

                if restored.cursor.slot == 0 {
                    println!("database is empty — ingesting from the start of available history");
                } else {
                    println!(
                        "resuming at slot {} | {} pools, {} positions, {} proposals, \
                         {} treasuries, {} realms restored",
                        restored.cursor.slot,
                        restored.state.pools.len(),
                        restored.state.positions.len(),
                        restored.state.proposals.len(),
                        restored.state.treasuries.len(),
                        restored.state.realms.len(),
                    );
                }

                let ingestor = Ingestor::restore(restored.cursor, restored.state);
                Ok((Self::Postgres(Box::new(store)), ingestor))
            }
            #[cfg(not(feature = "postgres"))]
            Some(_) => Err("built without the `postgres` feature".into()),
        }
    }

    #[cfg_attr(not(feature = "postgres"), allow(unused_variables))]
    fn commit(
        &mut self,
        ingestor: &Ingestor,
        settled: &[SettledTransaction],
    ) -> Result<usize, String> {
        match self {
            Self::Memory => Ok(0),
            #[cfg(feature = "postgres")]
            Self::Postgres(store) => store
                .commit(ingestor.cursor(), ingestor.finalized(), settled)
                .map_err(|e| e.to_string()),
        }
    }

    fn describe(&self) -> &'static str {
        match self {
            Self::Memory => "in memory only, not persisted",
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => "persisting to Postgres",
        }
    }
}

fn run(args: &Args) -> Result<std::process::ExitCode, String> {
    let (mut sink, mut ingestor) = Sink::open(args.database_url.as_deref())?;
    let mut source = RpcLogSource::new(&args.url);
    println!(
        "following {} ({}) — Ctrl-C to stop",
        args.url,
        sink.describe()
    );

    loop {
        match ingestor.poll(&mut source, 500) {
            Ok(outcome) => {
                if let Some(reverted) = outcome.reverted {
                    println!(
                        "reorg: {reverted} transaction(s) rolled back and replayed from the \
                         finalized projection"
                    );
                }
                for reported in &outcome.anomalies {
                    println!("anomaly: {} {:?}", reported.signature, reported.anomaly);
                }

                // The cursor and the rows move together or not at all. Committing
                // *after* the fold rather than alongside it is what makes that
                // true: `ingestor.cursor()` is already past everything in
                // `settled`, so a crash before this line loses a poll and a crash
                // during it changes nothing.
                let stored = sink.commit(&ingestor, &outcome.settled)?;

                // `--once` is a question, so it always gets an answer. The watch
                // loop prints only on change, because a line every two seconds
                // saying nothing happened is how a log stops being read.
                if args.once || outcome.applied > 0 || outcome.finalized() > 0 {
                    let head = ingestor.head();
                    println!(
                        "+{} applied, {} finalized, {} rows stored | cursor slot {} | \
                         {} unfinalized | {} pools, {} positions, {} proposals, {} treasuries, \
                         {} realms | {} orphaned",
                        outcome.applied,
                        outcome.finalized(),
                        stored,
                        ingestor.cursor().slot,
                        ingestor.pending_count(),
                        head.pools.len(),
                        head.positions.len(),
                        head.proposals.len(),
                        head.treasuries.len(),
                        head.realms.len(),
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
                return Ok(std::process::ExitCode::FAILURE);
            }
            Err(IngestError::Source(e)) => {
                eprintln!("source error: {e}");
                return Ok(std::process::ExitCode::FAILURE);
            }
        }

        if args.once {
            return Ok(std::process::ExitCode::SUCCESS);
        }
        std::thread::sleep(args.interval);
    }
}
