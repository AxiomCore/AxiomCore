use super::*;
use crate::commands::cloud_mock::Mock;
use crate::commands::step_up::StepUpStore;
use ed25519_dalek::{Signer, SigningKey};

const RESOURCE: &str = "6f1c2a8e-0d4b-4c39-9a51-6a2f3f0b9e11";
const MOVE: &str = "7a2d3b9f-1e5c-4d4a-8b62-7b3f4a1c0f22";
const TOKEN: &str = "SYNTHETIC_TURSO_TOKEN_CANARY";

fn input() -> ConnectInput {
    ConnectInput { provider: "turso".into(), organization: "acme".into(), group: "default".into(), database: "shop-db".into(), fence_mode: "expiry".into(), acknowledge_split_store: true, ..Default::default() }
}

async fn fixture() -> (Mock, Api, tempfile::TempDir) {
    let mock = Mock::default();
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let api = Api::new(&base, "session-access".into(), "session-refresh".into(), false, StepUpStore::at(dir.path().join("step-up.json"))).unwrap().fast();
    mock.script("GET /api/v1/projects/demo/resources", 200, json!({"resources": [{"id": RESOURCE, "environment": "production", "generation": 4}], "operations": []}));
    (mock, api, dir)
}

#[test]
fn connect_body_needs_acknowledgements_and_valid_names() {
    let body = connect_body(&input(), TOKEN).unwrap();
    assert_eq!(body["acknowledge"], json!({"split_store": true}));
    assert_eq!(body["fence_mode"], "expiry");
    assert!(body.get("reclaim").is_none());
    let mut no_ack = input();
    no_ack.acknowledge_split_store = false;
    let error = connect_body(&no_ack, TOKEN).unwrap_err().to_string();
    assert!(error.contains("--acknowledge-split-store") && !error.contains(TOKEN), "{error}");
    let mut rotate = input();
    rotate.fence_mode = "rotate".into();
    assert!(connect_body(&rotate, TOKEN).is_err());
    rotate.acknowledge_rotate = true;
    assert_eq!(connect_body(&rotate, TOKEN).unwrap()["acknowledge"]["rotate_invalidates_my_tokens"], true);
    let mut bad = input();
    bad.database = "Shop DB".into();
    assert!(connect_body(&bad, TOKEN).is_err());
    let mut reclaim = input();
    reclaim.reclaim = Some("reclaim other".into());
    assert!(connect_body(&reclaim, TOKEN).is_err());
    reclaim.reclaim = Some("reclaim shop-db".into());
    assert_eq!(connect_body(&reclaim, TOKEN).unwrap()["reclaim"], "reclaim shop-db");
    assert!(connect_body(&input(), "two tokens").is_err());
    assert!(connect_body(&input(), "").is_err());
}

#[tokio::test]
async fn connect_puts_the_token_once_and_never_shows_it() {
    let (mock, api, _dir) = fixture().await;
    let route = format!("PUT /api/v1/projects/demo/resources/{RESOURCE}/connection");
    mock.script(&route, 202, json!({"connection": {"provider": "turso", "organization": "acme", "group": "default", "database": "shop-db", "status": "verifying", "token": {"set": true, "expires_at": null, "value": TOKEN}}, "operation": {"id": "op"}}));
    let select = Select { environment: Some("production".into()), resource: None };
    let value = connect(&api, "demo", &select, &input(), TOKEN, None).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body["token"], TOKEN);
    assert!(uuid::Uuid::parse_str(seen[0].key.as_deref().unwrap()).is_ok());
    let shown = safe_connection(value);
    let rendered = format!("{} {}", shown, render_connection(&shown));
    assert!(!rendered.contains(TOKEN), "{rendered}");
    assert_eq!(shown["connection"]["token"]["set"], true);
    assert!(render_connection(&shown).contains("expires never"));
}

#[tokio::test]
async fn connect_refusals_show_known_codes_only() {
    let (mock, api, _dir) = fixture().await;
    let route = format!("PUT /api/v1/projects/demo/resources/{RESOURCE}/connection");
    mock.script(&route, 422, json!({"code": "AXRESOURCE_BYO_TOKEN_EXPIRING", "reason": "SYNTHETIC_PRIVATE_CANARY"}));
    let select = Select { resource: Some(RESOURCE.into()), environment: None };
    let error = connect(&api, "demo", &select, &input(), TOKEN, None).await.unwrap_err().to_string();
    assert!(error.starts_with("AXRESOURCE_BYO_TOKEN_EXPIRING"), "{error}");
    assert!(!error.contains("CANARY") && !error.contains(TOKEN));
}

#[test]
fn disconnect_and_consent_need_the_typed_phrase() {
    assert!(disconnect_body(4, "shop-db", None).unwrap_err().to_string().contains("--confirm \"disconnect shop-db\""));
    assert!(disconnect_body(4, "shop-db", Some("disconnect other")).is_err());
    assert_eq!(disconnect_body(4, "shop-db", Some("disconnect shop-db")).unwrap(), json!({"generation": 4, "confirm": "disconnect shop-db"}));
    assert!(consent_body("region", None, true, "production", Some("x")).is_err());
    assert!(consent_body("region", Some("sydney"), false, "production", Some("move production to sydney")).is_err());
    assert_eq!(
        consent_body("region", Some("sydney"), true, "production", Some("move production to sydney")).unwrap(),
        json!({"kind": "region", "to_region": "sydney", "acknowledge_costs": true, "confirm": "move production to sydney"})
    );
    assert!(consent_body("dedicated", None, true, "production", Some("move production to dedicated")).is_ok());
    assert!(consent_body("dedicated", Some("sydney"), true, "production", Some("move production to dedicated")).is_err());
}

#[test]
fn convert_body_needs_both_acknowledgements_and_the_phrase() {
    let conversion = "8b3e4c0a-2f6d-4e5b-9c73-8c4a5b2d1e33";
    let mut ack = ConvertAcknowledge::default();
    assert!(convert_body(3, conversion, Some("convert shop to postgresql"), &ack).is_err());
    ack.sessions_end = true;
    ack.redeploy = true;
    assert!(convert_body(3, conversion, Some("convert shop"), &ack).is_err());
    assert!(convert_body(3, "nope", Some("convert shop to postgresql"), &ack).is_err());
    ack.external_writers = Some("none".into());
    let body = convert_body(3, conversion, Some("convert shop to postgresql"), &ack).unwrap();
    assert_eq!(body["acknowledge"], json!({"sessions_end": true, "redeploy": true, "external_writers": "none"}));
    assert_eq!(body["conversion_id"], conversion);
}

fn signed_placement(key: &SigningKey) -> (Value, Value) {
    let b64 = base64::engine::general_purpose::STANDARD;
    let body = json!({"move_id": MOVE, "project_id": "p", "resource_id": RESOURCE, "kind": "relocate",
        "before": {"generation": 3, "region_id": "singapore"}, "after": {"generation": 4, "region_id": "sydney"},
        "checks": {"rows": 12}, "retained_until": "2026-11-08T00:00:00Z", "recovery": "retained_fenced_previous"});
    let canonical = serde_json::to_vec(&body).unwrap();
    let placement = json!({"current": {}, "receipts": [{"body": body, "canonical": b64.encode(&canonical), "body_sha256": hex::encode(Sha256::digest(&canonical)),
        "signature": b64.encode(key.sign(&canonical).to_bytes()), "key_version": "1", "created_at": "2026-11-01T00:00:00Z"}]});
    let keys = json!({"keys": [{"version": "1", "algorithm": "EC_SIGN_ED25519", "public_key": b64.encode(key.verifying_key().to_bytes())}]});
    (placement, keys)
}

#[test]
fn receipts_verify_offline_and_tampering_is_refused() {
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let (placement, keys) = signed_placement(&key);
    let summary = verify_receipt(&placement, &keys, MOVE).unwrap();
    assert_eq!(summary["verified"], true);
    assert_eq!(summary["after"]["region_id"], "sydney");
    // The displayed body differs from the signed bytes.
    let mut shown = placement.clone();
    shown["receipts"][0]["body"]["after"]["region_id"] = json!("mumbai");
    assert!(verify_receipt(&shown, &keys, MOVE).is_err());
    // Another key.
    let (_, other) = signed_placement(&SigningKey::from_bytes(&[1u8; 32]));
    assert!(verify_receipt(&placement, &other, MOVE).unwrap_err().to_string().contains("signature does not hold"));
    // Unsigned receipts are reported, not accepted.
    let mut unsigned = placement.clone();
    unsigned["receipts"][0]["signature"] = Value::Null;
    assert!(verify_receipt(&unsigned, &keys, MOVE).unwrap_err().to_string().contains("not signed yet"));
    assert!(verify_receipt(&placement, &keys, "8b3e4c0a-2f6d-4e5b-9c73-8c4a5b2d1e33").is_err());
    assert!(render_placement(&placement).contains("signed"));
}

#[tokio::test]
async fn receipt_verify_fetches_placement_and_keys() {
    let (mock, api, _dir) = fixture().await;
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let (placement, keys) = signed_placement(&key);
    mock.script(&format!("GET /api/v1/projects/demo/resources/{RESOURCE}/placement"), 200, placement);
    mock.script("GET /api/v1/resources/receipt-keys", 200, keys);
    let action = ReceiptAction::Verify { move_id: MOVE.into(), select: Select { environment: Some("production".into()), resource: None }, file: None, keys: None, json: true };
    assert_eq!(receipt(&api, "demo", &action).await.unwrap()["verified"], true);
}

#[tokio::test]
async fn connection_check_and_wait() {
    let (mock, api, _dir) = fixture().await;
    mock.script("GET /api/v1/projects/demo/resources", 200, json!({"resources": [], "operations": [{"id": "op-1", "status": "failed", "failure_code": "AXRESOURCE_BYO_CLAIMED"}]}));
    let error = wait_operation(&api, "demo", "op-1", Duration::from_secs(2)).await.unwrap_err().to_string();
    assert!(error.contains("AXRESOURCE_BYO_CLAIMED") && error.contains("reclaim"), "{error}");
}
