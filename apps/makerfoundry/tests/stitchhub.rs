//! The StitchHub client (MF-81, issues #1 and #53) against a stand-in hub that behaves as StitchHub
//! describes: the handshake (and `hello_first`); the outbox survives the hub being down and sends
//! in order; a refusal is final; a repeat is a duplicate, never sent twice; a version the hub no
//! longer takes stops sending; a message that expires unsent is reported; `order.placed` is
//! signed; messages from shops are applied once and in order, re-polling until the inbox is empty.
//! Needs `MF_TEST_DATABASE_URL` (each test makes its own schema), as `tests/accounts.rs` does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use makerfoundry::shop_orders::contract::{self, Envelope, Message, ProofAnswer};
use makerfoundry::shop_orders::hub::{self, Hub, stand_in};
use makerfoundry::shop_orders::sign::Signer;
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

/// What the test's "orders" were asked to apply: (type, conversation, seq, id).
type Applied = Arc<Mutex<Vec<(String, String, u64, String)>>>;

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
                .push((msg.kind().to_string(), env.conversation.clone(), env.seq, id));
            Ok(())
        }) as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
    }
}

async fn setup() -> Option<(PgPool, Hub, stand_in::Shared)> {
    let pool = test_db().await?;
    let (base, shared) = stand_in::start().await.unwrap();
    let signer = test_signer();
    shared.lock().unwrap().public_key = Some(signer.public_spki());
    Some((pool, Hub::new(&base, "mf-test").unwrap().with_signer(signer), shared))
}

/// A signing key made from a fixed seed (PKCS#8 v1, RFC 8410 § 7).
fn test_signer() -> Signer {
    use base64::Engine;
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
    ];
    der.extend_from_slice(&[42u8; 32]);
    Signer::from_base64(&base64::engine::general_purpose::STANDARD.encode(der)).unwrap()
}

async fn sync(pool: &PgPool, hub: &Hub, apply: hub::Apply<'_>) -> Result<(), String> {
    hub::sync(pool, hub, None, apply).await
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
        ids.push(hub::enqueue(&mut tx, &msg, "mf-1", to, 7, &[]).await.unwrap());
        tx.commit().await.unwrap();
    }
    ids
}

/// A status message from the shop, as the hub would deliver it.
fn from_shop(id: &str, seq: u64, status: &str) -> Value {
    json!({ "id": id, "type": "order.status", "version": 1, "conversation": "mf-1", "seq": seq,
            "from": "shop:mainst", "to": "service:makerfoundry", "delivery_seq": seq,
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
    assert!(sync(&pool, &hub, &apply).await.is_err());
    let unsent: i64 = sqlx::query_scalar("SELECT count(*) FROM mf_hub_outbox WHERE sent_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unsent, 3);
    // back up: all three, in their order, each once
    shared.lock().unwrap().down = false;
    sync(&pool, &hub, &apply).await.unwrap();
    sync(&pool, &hub, &apply).await.unwrap();
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
    sync(&pool, &hub, &apply).await.unwrap();
    let (attempts, later): (i32, bool) = sqlx::query_as("SELECT attempts, next_try_at > now() FROM mf_hub_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((attempts, later), (1, true));
    // not due yet: the next round leaves it, even with the hub back
    shared.lock().unwrap().sends_down = false;
    sync(&pool, &hub, &apply).await.unwrap();
    assert!(shared.lock().unwrap().received.is_empty());
    sqlx::query("UPDATE mf_hub_outbox SET next_try_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    sync(&pool, &hub, &apply).await.unwrap();
    assert_eq!(shared.lock().unwrap().received.len(), 1);
    // a refusal: kept with why, never tried again
    send_answers(&pool, "shop:refuses", 1).await;
    sync(&pool, &hub, &apply).await.unwrap();
    sync(&pool, &hub, &apply).await.unwrap();
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
    let err = sync(&pool, &hub, &apply).await.unwrap_err();
    assert!(err.contains("update"), "{err}");
    assert_eq!(hub::state(&pool, "too_old").await.as_deref(), Some("yes"));
    assert!(shared.lock().unwrap().received.is_empty());
    shared.lock().unwrap().min_makerfoundry = "0.0.1".into();
    sync(&pool, &hub, &apply).await.unwrap();
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
    sync(&pool, &hub, &apply).await.unwrap();
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
    sync(&pool, &hub, &apply).await.unwrap();
    assert!(applied.lock().unwrap().is_empty(), "held for 1");
    assert!(
        shared.lock().unwrap().queues[contract::me()].is_empty(),
        "held ones are acknowledged"
    );
    stand_in::deliver(&shared, from_shop(&uuid(1), 1, "proof_sent"));
    sync(&pool, &hub, &apply).await.unwrap();
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
    sync(&pool, &hub, &apply).await.unwrap();
    assert_eq!(applied.lock().unwrap().len(), 3);
    assert!(
        shared.lock().unwrap().queues[contract::me()].is_empty(),
        "all acknowledged"
    );
    // the next in order still applies
    stand_in::deliver(&shared, from_shop(&uuid(4), 4, "ready"));
    sync(&pool, &hub, &apply).await.unwrap();
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
    sync(&pool, &hub, &apply).await.unwrap();
    stand_in::deliver(&shared, catalog);
    sync(&pool, &hub, &apply).await.unwrap();
    let catalogs = applied.lock().unwrap().iter().filter(|a| a.0 == "catalog").count();
    assert_eq!(catalogs, 1);
}

#[tokio::test]
async fn the_handshake_says_what_it_reads_and_is_redone_when_the_hub_asks() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    sync(&pool, &hub, &apply).await.unwrap();
    {
        let s = shared.lock().unwrap();
        assert!(s.said_hello);
        for t in [
            "catalog",
            "order.status",
            "proof.sent",
            "order.cancel",
            "message.expired",
        ] {
            assert_eq!(s.reads[t], json!([1]), "{t}");
        }
    }
    // the hub forgot the handshake (a restart): `hello_first`, then it's said again and the
    // message comes
    shared.lock().unwrap().said_hello = false;
    stand_in::deliver(&shared, from_shop(&uuid(1), 1, "proof_sent"));
    sync(&pool, &hub, &apply).await.unwrap();
    assert!(shared.lock().unwrap().said_hello);
    assert_eq!(applied.lock().unwrap().len(), 1);
    // the shops the hub lists
    shared.lock().unwrap().shops = vec![
        json!({"id": "shop:mainst", "name": "Main Street Threadworks", "ready": true}),
        json!({"id": "nonsense", "name": "x", "ready": true}),
    ];
    let shops = hub.shops().await.unwrap();
    assert_eq!(shops.len(), 1);
    assert_eq!((shops[0].id.as_str(), shops[0].ready), ("shop:mainst", true));
}

#[tokio::test]
async fn the_inbox_is_polled_until_empty() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    // the hub releases one message per conversation at a time: five in one round all the same
    for n in 1..=5u8 {
        stand_in::deliver(&shared, from_shop(&uuid(n), u64::from(n), "in_production"));
    }
    sync(&pool, &hub, &apply).await.unwrap();
    let seqs: Vec<u64> = applied.lock().unwrap().iter().map(|a| a.2).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5]);
    assert!(shared.lock().unwrap().queues[contract::me()].is_empty());
}

#[tokio::test]
async fn a_repeat_is_a_duplicate_and_a_reused_id_counts_as_delivered() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    send_answers(&pool, "shop:mainst", 1).await;
    sync(&pool, &hub, &apply).await.unwrap();
    let sent = shared.lock().unwrap().received[0].clone();
    // the same again (an answer lost on the way back): the hub says it had it
    assert!(hub.send(&sent).await.unwrap());
    // the same id with other content: the hub has that id already, so it's not sent again
    let mut changed = sent.clone();
    changed["created_at"] = json!(contract::now_utc());
    changed["body"]["note"] = json!("other");
    assert!(hub.send(&changed).await.unwrap());
    // a new id in a seq already taken: refused for good
    let mut taken = sent.clone();
    taken["id"] = json!(uuid(77));
    let err = hub.send(&taken).await.unwrap_err();
    assert!(format!("{err:?}").contains("seq_taken"), "{err:?}");
    assert_eq!(shared.lock().unwrap().received.len(), 1);
}

/// The example `order.placed`, its art put in the hub for `conversation`.
fn placed_message(shared: &stand_in::Shared, conversation: &str) -> Message {
    let mut v: Value = serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/contract/shop-orders/valid/order.placed.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let art = b"art bytes";
    let sha = stand_in::put_file(shared, conversation, art);
    v["body"]["lines"][0]["decorations"][0]["art"]["sha256"] = json!(sha);
    v["body"]["lines"][0]["decorations"][0]["art"]["size"] = json!(art.len());
    contract::parse(&serde_json::to_vec(&v).unwrap()).unwrap().1
}

#[tokio::test]
async fn an_order_is_signed_and_the_hub_checks_it() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    let msg = placed_message(&shared, "mf-8f3a1");
    let mut tx = pool.begin().await.unwrap();
    hub::enqueue(&mut tx, &msg, "mf-8f3a1", "shop:mainst", 7, &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sync(&pool, &hub, &apply).await.unwrap();
    let got = shared.lock().unwrap().received.clone();
    assert_eq!(got.len(), 1, "the hub took it");
    let env: Envelope = serde_json::from_value(got[0].clone()).unwrap();
    assert_eq!(env.sig.as_ref().unwrap().v, 1);
    assert!(makerfoundry::shop_orders::sign::verify(
        &hub.public_key().unwrap(),
        &env
    ));
    assert!(got[0].get("from").is_none(), "no from: the hub knows who sent it");

    // without a signing key the order waits (never sent unsigned)
    let (base, shared2) = stand_in::start().await.unwrap();
    let unsigned = Hub::new(&base, "mf-test").unwrap();
    let msg = placed_message(&shared2, "mf-8f3a2");
    let mut tx = pool.begin().await.unwrap();
    hub::enqueue(&mut tx, &msg, "mf-8f3a2", "shop:mainst", 7, &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sync(&pool, &unsigned, &apply).await.unwrap();
    assert!(shared2.lock().unwrap().received.is_empty());
    let (sent, failed): (bool, Option<String>) =
        sqlx::query_as("SELECT sent_at IS NOT NULL, failed FROM mf_hub_outbox WHERE order_ref = 'mf-8f3a2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((sent, failed), (false, None), "it waits for the key");

    // signed by a key the hub doesn't know: 403, final
    sqlx::query("UPDATE mf_hub_outbox SET next_try_at = now() WHERE order_ref = 'mf-8f3a2'")
        .execute(&pool)
        .await
        .unwrap();
    shared2.lock().unwrap().public_key = Some(test_signer().public_spki());
    let mut der_seed = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
    ];
    der_seed.extend_from_slice(&[9u8; 32]);
    use base64::Engine;
    let other = Signer::from_base64(&base64::engine::general_purpose::STANDARD.encode(der_seed)).unwrap();
    sync(&pool, &unsigned.with_signer(other), &apply).await.unwrap();
    let failed: Option<String> = sqlx::query_scalar("SELECT failed FROM mf_hub_outbox WHERE order_ref = 'mf-8f3a2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(failed.unwrap().contains("403"));
}

#[tokio::test]
async fn an_old_message_is_restamped_before_it_goes() {
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let apply = recorder(&applied);
    send_answers(&pool, "shop:mainst", 1).await;
    // made an hour ago (the hub was down): the hub only takes created_at within 10 minutes
    sqlx::query(
        "UPDATE mf_hub_outbox SET created_at = now() - interval '1 hour',
            envelope = jsonb_set(envelope, '{created_at}', '\"2026-01-01T00:00:00Z\"')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sync(&pool, &hub, &apply).await.unwrap();
    let got = shared.lock().unwrap().received.clone();
    assert_eq!(got.len(), 1);
    assert_ne!(got[0]["created_at"], "2026-01-01T00:00:00Z");
}

#[tokio::test]
async fn a_message_that_couldnt_be_applied_comes_again_past_the_cursor() {
    // (the hub's `next` can pass a message handed out and not acknowledged: StitchHub's PR #45)
    let Some((pool, hub, shared)) = setup().await else {
        return;
    };
    let applied: Applied = Arc::default();
    let record = recorder(&applied);
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let is_busy = busy.clone();
    // while mf-1's order is busy, its messages can't be applied
    let flaky = move |env: Envelope, msg: Message| {
        if env.conversation == "mf-1" && is_busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Box::pin(async { Err("busy".to_string()) })
                as Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
        }
        record(env, msg)
    };
    // two conversations: mf-1's message fails, mf-2's is applied from the same page
    stand_in::deliver(&shared, from_shop(&uuid(1), 1, "proof_sent"));
    let mut other = from_shop(&uuid(2), 1, "proof_sent");
    other["conversation"] = "mf-2".into();
    stand_in::deliver(&shared, other);
    sync(&pool, &hub, &flaky).await.unwrap();
    assert_eq!(applied.lock().unwrap().len(), 1, "mf-2's");
    assert_eq!(
        shared.lock().unwrap().queues[contract::me()].len(),
        1,
        "mf-1's wasn't acknowledged"
    );
    // the next round: it comes again and is applied, and so is what follows it
    busy.store(false, std::sync::atomic::Ordering::SeqCst);
    stand_in::deliver(&shared, from_shop(&uuid(3), 2, "approved"));
    sync(&pool, &hub, &flaky).await.unwrap();
    let got: Vec<(String, u64)> = applied.lock().unwrap().iter().map(|a| (a.1.clone(), a.2)).collect();
    assert_eq!(got, [("mf-2".to_string(), 1), ("mf-1".into(), 1), ("mf-1".into(), 2)]);
    assert!(shared.lock().unwrap().queues[contract::me()].is_empty());
}
