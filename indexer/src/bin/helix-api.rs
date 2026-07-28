//! Serves the read model over HTTP.
//!
//! ```bash
//! cargo run -p helix-indexer --features server --bin helix-api
//! ```
//!
//! **There is no ingestion loop here, and that is not an oversight.** The
//! `LogSource` implementation that talks to an RPC node does not exist yet — see
//! [Phase 4.1](../../../docs/ROADMAP.md#phase-4--indexer-and-analytics-api) — so
//! this serves an empty projection until one does. Wiring a poll loop to a source
//! that cannot exist would be a loop that has never run.
//!
//! What it does establish is the shape: an `Ingestor` behind an `RwLock`,
//! handlers reading a consistent snapshot, and the whole read model reachable
//! over the routes a dashboard will call.

use std::sync::{Arc, RwLock};

use helix_indexer::ingest::Ingestor;
use helix_indexer::server;

#[tokio::main]
async fn main() {
    let address = std::env::var("HELIX_API_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());

    let state = Arc::new(RwLock::new(Ingestor::new()));
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {address}: {e}"));

    eprintln!("helix-api listening on http://{address}");
    eprintln!("  GET /health");
    eprintln!("  GET /pools/{{address}}[?finality=head]");
    eprintln!("  GET /pools/{{address}}/stakers[?limit=50]");
    eprintln!("  GET /realms/{{address}}/proposals");
    eprintln!("  GET /treasuries/{{address}}");
    eprintln!();
    eprintln!("serving an empty projection: no ingestion source is wired yet (ROADMAP 4.1)");

    axum::serve(listener, server::router(state))
        .await
        .expect("server failed");
}
