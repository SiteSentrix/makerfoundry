//! Ordering physical goods from shops through StitchHub (issues #1 and #2): the owner's decisions
//! of 2026-10-31 are in `docs/stitchshop-ordering-audit.md`, the messages in
//! `docs/shop-orders-contract.md`. Switched off until `MF_STITCHHUB_URL` and `MF_STITCHHUB_KEY`
//! are set.

pub mod contract;
pub mod hub;
pub mod orders;
pub mod shops;

use std::future::Future;
use std::pin::Pin;

use sqlx::PgPool;

use crate::AppState;
use contract::{Envelope, Message};

/// Applies a checked message from a shop (or the hub), once and in order (`hub::sync`).
pub async fn apply(pool: PgPool, env: Envelope, msg: Message) -> Result<(), String> {
    match &msg {
        Message::Catalog(c) => shops::keep(&pool, &env.from, c).await,
        // (orders come with MF-63 and MF-64; until then nothing sends any)
        _ => Err(format!("{} isn't handled yet", msg.kind())),
    }
}

/// One round with StitchHub when it's switched on (the sweep, every minute).
pub async fn sync(state: &AppState) -> Result<(), String> {
    let (Some(hub), Some(pool)) = (state.stitchhub.as_ref(), state.db.as_ref()) else {
        return Ok(());
    };
    let p = pool.clone();
    let run = move |env: Envelope, msg: Message| {
        let p = p.clone();
        Box::pin(apply(p, env, msg)) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
    };
    hub::sync(pool, hub, &run).await
}
