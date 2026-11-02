//! A test deploy's own name on the hub (`MF_STITCHHUB_SERVICE`, StitchHub's message of
//! 2026-11-01): what it signs, sends and takes is in that name, never `service:makerfoundry`.
//! Its own test binary, since the name is set once for the process.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use makerfoundry::shop_orders::contract::{self, Envelope};
use makerfoundry::shop_orders::sign;
use serde_json::json;

fn status_to(to: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "0192f0c4-7a1b-7c2d-8e3f-000000000001", "type": "order.status", "version": 1,
        "conversation": "mf-8f3a1", "seq": 1, "from": "shop:mainst", "to": to,
        "created_at": "2026-11-02T15:04:05Z", "expires_at": "2099-11-09T15:04:05Z",
        "body": { "status": "received" }
    }))
    .unwrap()
}

#[test]
fn a_test_deploy_signs_sends_and_takes_in_its_own_name() {
    assert_eq!(contract::me(), "service:makerfoundry");
    assert!(contract::set_me("not a service").is_err());
    contract::set_me("service:makerfoundry-test").unwrap();
    assert_eq!(contract::me(), "service:makerfoundry-test");
    assert!(contract::set_me("service:other").is_err(), "set once");

    // the signed text's sender line is the test name
    let env = Envelope {
        id: "0192f0c4-7a1b-7c2d-8e3f-0123456789ab".into(),
        kind: "order.placed".into(),
        version: 1,
        conversation: "mf-8f3a1".into(),
        seq: 1,
        from: None,
        to: "shop:mainst".into(),
        created_at: "2026-11-02T15:04:05Z".into(),
        expires_at: None,
        body: json!({}),
        sig: None,
    };
    assert!(sign::signed_text(&env).starts_with("SBHUB1\nservice:makerfoundry-test\nshop:mainst\n"));
    assert_eq!(env.sender(), "service:makerfoundry-test");

    // messages for the test name are taken; ones for production's name aren't this deploy's
    assert!(contract::parse(&status_to("service:makerfoundry-test")).is_ok());
    assert!(contract::parse(&status_to("service:makerfoundry")).is_err());
    assert!(contract::parse(&status_to("makerfoundry")).is_err());
}
