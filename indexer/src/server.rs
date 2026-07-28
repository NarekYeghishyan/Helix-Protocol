//! HTTP transport for [`crate::api`].
//!
//! Deliberately thin. Every decision that could be wrong — which projection to
//! read, how amounts are encoded, what an undefined APR serialises to — is in
//! `api.rs` and tested there. What is left here is routing, and routing that
//! contains logic is routing nobody tests.
//!
//! Behind the `server` feature so the default build does not pay for an async
//! runtime to compile pure functions.
//!
//! # Reading state
//!
//! Handlers take a snapshot behind an `RwLock`, held by whatever drives
//! [`crate::Ingestor::poll`]. Readers never block each other and never block
//! ingestion for longer than a projection clone.

use std::sync::{Arc, RwLock};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::api::{Api, Finality};
use crate::ingest::Ingestor;

pub type Shared = Arc<RwLock<Ingestor>>;

/// `?finality=head` — defaults to `finalized`.
///
/// Defaulting to the conservative view is the point. A caller that has not
/// thought about forks gets numbers that will not be revised; opting into
/// `head` is opting into that possibility knowingly.
#[derive(Debug, Default, Deserialize)]
pub struct ViewParams {
    #[serde(default)]
    finality: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

impl ViewParams {
    fn finality(&self) -> Finality {
        match self.finality.as_deref() {
            Some("head") => Finality::Head,
            _ => Finality::Finalized,
        }
    }

    /// Capped, so a caller cannot ask for the whole staker set by accident.
    fn limit(&self) -> usize {
        self.limit.unwrap_or(50).clamp(1, 500)
    }
}

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/pools/{address}", get(pool))
        .route("/pools/{address}/stakers", get(stakers))
        .route("/realms/{address}/proposals", get(proposals))
        .route("/treasuries/{address}", get(treasury))
        .with_state(state)
}

/// Typed rather than an ad-hoc JSON object, so the health shape is part of the
/// crate's contract and changes to it are visible at compile time.
#[derive(Debug, serde::Serialize)]
pub struct Health {
    pub finalized_slot: u64,
    pub pending_transactions: usize,
}

async fn health(State(state): State<Shared>) -> impl IntoResponse {
    let ingestor = state.read().expect("ingestor lock poisoned");
    Json(Health {
        finalized_slot: ingestor.cursor().slot,
        pending_transactions: ingestor.pending_count(),
    })
}

/// `None` from the read model becomes 404, which covers both "no such pool" and
/// "that is not an address". Distinguishing them would tell a caller which of
/// their two mistakes they made, at the cost of a second error path; the address
/// is in the URL either way.
fn respond<T: serde::Serialize>(found: Option<T>) -> axum::response::Response {
    match found {
        Some(body) => Json(body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn pool(
    State(state): State<Shared>,
    Path(address): Path<String>,
    Query(params): Query<ViewParams>,
) -> impl IntoResponse {
    let ingestor = state.read().expect("ingestor lock poisoned");
    respond(Api::new(&ingestor).pool(params.finality(), &address))
}

async fn stakers(
    State(state): State<Shared>,
    Path(address): Path<String>,
    Query(params): Query<ViewParams>,
) -> impl IntoResponse {
    let ingestor = state.read().expect("ingestor lock poisoned");
    respond(Api::new(&ingestor).stakers(params.finality(), &address, params.limit()))
}

async fn proposals(
    State(state): State<Shared>,
    Path(address): Path<String>,
    Query(params): Query<ViewParams>,
) -> impl IntoResponse {
    let ingestor = state.read().expect("ingestor lock poisoned");
    respond(Api::new(&ingestor).proposals(params.finality(), &address))
}

async fn treasury(
    State(state): State<Shared>,
    Path(address): Path<String>,
    Query(params): Query<ViewParams>,
) -> impl IntoResponse {
    let ingestor = state.read().expect("ingestor lock poisoned");
    respond(Api::new(&ingestor).treasury(params.finality(), &address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finality_defaults_to_the_view_that_cannot_be_revised() {
        assert_eq!(ViewParams::default().finality(), Finality::Finalized);
        assert_eq!(
            ViewParams {
                finality: Some("head".into()),
                limit: None
            }
            .finality(),
            Finality::Head
        );
        // Anything unrecognised falls back to the safe view rather than erroring:
        // a typo should not silently promote a caller to unfinalised data.
        assert_eq!(
            ViewParams {
                finality: Some("latest".into()),
                limit: None
            }
            .finality(),
            Finality::Finalized
        );
    }

    #[test]
    fn the_limit_is_clamped_at_both_ends() {
        let limit = |n: Option<usize>| {
            ViewParams {
                finality: None,
                limit: n,
            }
            .limit()
        };
        assert_eq!(limit(None), 50);
        assert_eq!(limit(Some(0)), 1, "zero would return an empty page");
        assert_eq!(limit(Some(10_000)), 500);
        assert_eq!(limit(Some(25)), 25);
    }
}
