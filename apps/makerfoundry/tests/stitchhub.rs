//! The StitchHub client (MF-81, issue #1) against a stand-in hub: the outbox survives the hub being
//! down and sends in order; a refusal is final; a version the hub no longer takes stops sending; a
//! message that expires unsent is reported; messages from shops are applied once and in order.
//! Needs `MF_TEST_DATABASE_URL` (each test makes its own schema), as `tests/accounts.rs` does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use makerfoundry::shop_orders::contract::{self, Envelope, Message, ProofAnswer};
use makerfoundry::shop_orders::hub::{self, Hub, stand_in};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

async fn test_db() -> Option<PgPool> {
    let url = std::env::var("MF_TEST_DATABASE_URL").ok()?;
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let schema = format!("t{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let admin = PgPool::connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let opts = url
        .parse::<PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .unwrap();
    makerfoundry::db::MIGRATOR.run(&pool).await.unwrap();
    Some(pool)
}

/// What the test's "orders" were asked to apply: (type, order, seq, id).
type Applied = Arc<Mutex<Vec<(String, Option<String>, u64, String)>>>;

fn recorder(
    applied: &Applied,
) -> impl Fn(Envelope, Message) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Sync + use<> {
    let applied = applied.clone();
    move |env: Envelope, msg: Message| {
        let applied = applied.clone();
        Box::pin(async move {
            let id = match &msg {
                Message::MessageExpired(e) => e.id.clone(),
                _ => env.id.clone(),
            };
            applied
                .lock()
                .unwrap()
                .push((msg.kind().to_string(), env.order.clone(), env.seq, id));
            Ok(())
        }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
    }
}

async fn setup() -> Option<(PgPool, Hub, stand_in::Shared)> {
    let pool = test_db().await?;
    let (base, shared) = stand_in::start().await.unwrap();
    Some((pool, Hub::new(&base, "mf-test").unwrap(), shared))
}

async fn send_answers(pool: &PgPool, to: &str, n: u32) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 1..=n {
        let mut tx = pool.begin().await.unwrap();
        let msg = Message::ProofAnswer(ProofAnswer {
            proof: i,
            answer: "approved".into(),
            note: String::new(),
        });
        ids.push(hub::enqueue(&mut tx, &msg, Some("mf-1"), to, 7).await.unwrap());
        tx.commit().await.unwrap();
    }
    ids
}

/// A status message from the shop, as the hub would deliver it.
fn from_shop(id: &str, seq: u64, status: &str) -> Value {
    json!({ "id": id, "type": "order.status", "version": 1, "order": "mf-1", "seq": seq,
            "from": "shop:mainst", "to": "makerfoundry",
            "created_at": "2026-11-02T15:04:05Z", "expires_at": "2099-11-09T15:04:05Z",
            "body": { "status": status } })
}

fn uuid(n: u8) -> String {
    format!("0192f0c4-7a1b-7c2d-8e3f-0000000000{n:02x}")
}

#[test]
fn it_is_off_without_its_settings() {
    assert!(Hub::new("", "key").is_none());
    assert!(Hub::new("https://hub.example", " ").is_none());
    assert!(Hub::new("ftp://hub.example", "key").is_none());
    assert_eq!(
        format!("{:?}", Hub::new("https://hub.example", "secret").unwrap()),
        "Hub"
    );
}

#[tokio::test]
async fn the_outbox_waits_for_the_hub_and_sends_in_order() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    let ids = send_answers(&pool, "shop:mainst", 3).await;
    // the hub is down: nothing goes, nothing is lost
    shared.lock().unwrap().down = true;
    assert!(hub::sync(&pool, &hub, &apply).await.is_err());
    let unsent: i64 = sqlx::query_scalar("SELECT count(*) FROM mf_hub_outbox WHERE sent_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unsent, 3);
    // back up: all three, in their order, each once
    shared.lock().unwrap().down = false;
    hub::sync(&pool, &hub, &apply).await.unwrap();
    hub::sync(&pool, &hub, &apply).await.unwrap();
    let got: Vec<(String, u64)> = shared
        .lock()
        .unwrap()
        .received
        .iter()
        .map(|m| (m["id"].as_str().unwrap().to_string(), m["seq"].as_u64().unwrap()))
        .collect();
    assert_eq!(got, ids.iter().cloned().zip(1..=3).collect::<Vec<_>>());
    // what went out reads as a valid message
    for m in &shared.lock().unwrap().received {
        contract::parse(&serde_json::to_vec(m).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn a_failed_send_waits_longer_each_time_and_a_refusal_is_final() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    send_answers(&pool, "shop:mainst", 1).await;
    shared.lock().unwrap().sends_down = true;
    hub::sync(&pool, &hub, &apply).await.unwrap();
    let (attempts, later): (i32, bool) = sqlx::query_as("SELECT attempts, next_try_at > now() FROM mf_hub_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((attempts, later), (1, true));
    // not due yet: the next round leaves it, even with the hub back
    shared.lock().unwrap().sends_down = false;
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert!(shared.lock().unwrap().received.is_empty());
    sqlx::query("UPDATE mf_hub_outbox SET next_try_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert_eq!(shared.lock().unwrap().received.len(), 1);
    // a refusal: kept with why, never tried again
    send_answers(&pool, "shop:refuses", 1).await;
    hub::sync(&pool, &hub, &apply).await.unwrap();
    hub::sync(&pool, &hub, &apply).await.unwrap();
    let (failed, attempts): (Option<String>, i32) =
        sqlx::query_as("SELECT failed, attempts FROM mf_hub_outbox WHERE to_party = 'shop:refuses'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(failed.unwrap().contains("422"));
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn a_version_the_hub_no_longer_takes_stops_sending() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    send_answers(&pool, "shop:mainst", 1).await;
    shared.lock().unwrap().min_makerfoundry = "99.0.0".into();
    let err = hub::sync(&pool, &hub, &apply).await.unwrap_err();
    assert!(err.contains("update"), "{err}");
    assert_eq!(hub::state(&pool, "too_old").await.as_deref(), Some("yes"));
    assert!(shared.lock().unwrap().received.is_empty());
    shared.lock().unwrap().min_makerfoundry = "0.0.1".into();
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert_eq!(hub::state(&pool, "too_old").await.as_deref(), Some("no"));
    assert_eq!(shared.lock().unwrap().received.len(), 1);
}

#[tokio::test]
async fn a_message_that_expires_unsent_is_reported_as_undelivered() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    let ids = send_answers(&pool, "shop:mainst", 1).await;
    sqlx::query("UPDATE mf_hub_outbox SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert!(shared.lock().unwrap().received.is_empty(), "never sent once expired");
    let got = applied.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].0.as_str(), got[0].3.as_str()),
        ("message.expired", ids[0].as_str())
    );
}

#[tokio::test]
async fn messages_from_shops_are_applied_once_and_in_order() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    // 2 and 3 come before 1
    stand_in::deliver(&shared, from_shop(&uuid(2), 2, "approved"));
    stand_in::deliver(&shared, from_shop(&uuid(3), 3, "in_production"));
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert!(applied.lock().unwrap().is_empty(), "held for 1");
    assert!(
        shared.lock().unwrap().queues["makerfoundry"].is_empty(),
        "held ones are acknowledged"
    );
    stand_in::deliver(&shared, from_shop(&uuid(1), 1, "proof_sent"));
    hub::sync(&pool, &hub, &apply).await.unwrap();
    let seqs: Vec<u64> = applied.lock().unwrap().iter().map(|a| a.2).collect();
    assert_eq!(seqs, [1, 2, 3]);
    // the same message again, and a repeat of seq 3 under a new id: neither applied again
    stand_in::deliver(&shared, from_shop(&uuid(1), 1, "proof_sent"));
    stand_in::deliver(&shared, from_shop(&uuid(9), 3, "in_production"));
    // one that can't be read, and one for somebody else
    stand_in::deliver(
        &shared,
        json!({ "id": uuid(10), "type": "order.status", "body": "nonsense" }),
    );
    let mut other = from_shop(&uuid(11), 4, "ready");
    other["to"] = "shop:other".into();
    stand_in::deliver(&shared, other);
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert_eq!(applied.lock().unwrap().len(), 3);
    assert!(
        shared.lock().unwrap().queues["makerfoundry"].is_empty(),
        "all acknowledged"
    );
    // the next in order still applies
    stand_in::deliver(&shared, from_shop(&uuid(4), 4, "ready"));
    hub::sync(&pool, &hub, &apply).await.unwrap();
    assert_eq!(applied.lock().unwrap().last().unwrap().2, 4);

    // a message about no one order (a catalogue) delivered twice: applied once, by its id
    let catalog: Value = serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/contract/shop-orders/valid/catalog.json"
        ))
        .unwrap(),
    )
    .unwrap();
    stand_in::deliver(&shared, catalog.clone());
    hub::sync(&pool, &hub, &apply).await.unwrap();
    stand_in::deliver(&shared, catalog);
    hub::sync(&pool, &hub, &apply).await.unwrap();
    let catalogs = applied.lock().unwrap().iter().filter(|a| a.0 == "catalog").count();
    assert_eq!(catalogs, 1);
}
