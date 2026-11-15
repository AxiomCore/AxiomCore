//! Mock-server tests for `axiom project archive`, `restore`, `archive-status` and
//! `list --archived`.
use super::*;
use crate::commands::cloud_cli::test_support::{harness, Harness};
use std::time::Duration;

const ARCHIVE: &str = "POST /api/v1/projects/shop/archive";
const RESTORE: &str = "POST /api/v1/projects/shop/restore";
const VIEW: &str = "GET /api/v1/projects/shop/archive";
const PROJECTS: &str = "GET /api/v1/projects";

fn fast() -> Poll {
    Poll { interval: Duration::from_millis(5), limit: Duration::from_secs(5) }
}

struct Answer {
    terminal: bool,
    text: &'static str,
    asked: Vec<String>,
}

impl Prompt for Answer {
    fn is_terminal(&self) -> bool {
        self.terminal
    }
    fn input(&mut self, prompt: &str) -> Result<String> {
        self.asked.push(prompt.to_string());
        Ok(self.text.to_string())
    }
}

fn headless() -> Answer {
    Answer { terminal: false, text: "", asked: vec![] }
}

fn typed(text: &'static str) -> Answer {
    Answer { terminal: true, text, asked: vec![] }
}

fn view(state: &str) -> Value {
    json!({
        "enabled": true,
        "state": state,
        "hidden": matches!(state, "archiving" | "archived"),
        "confirm": "archive shop",
        "requested_at": "2026-11-14T10:00:00Z",
        "completed_at": null,
        "databases": [
            {"resource_id": "6f1c2a8e-0d4b-4c39-9a51-6a2f3f0b9e11", "database_name": "production", "status": "ready", "operation_id": "", "operation_status": "", "failure_code": ""}
        ],
        "blockers": []
    })
}

async fn shop() -> Harness {
    let h = harness().await;
    h.mock.script(PROJECTS, 200, json!([{"id": "11111111-1111-4111-8111-111111111111", "slug": "shop", "name": "Shop"}]));
    h
}

fn exit_code(error: &anyhow::Error) -> Option<i32> {
    error.downcast_ref::<crate::commands::auth::Exit>().map(|e| e.code)
}

fn script_step_up(h: &Harness) {
    h.mock.script(
        "POST /auth/device/start",
        200,
        json!({"device_code": "dc-1", "user_code": "ABCD-EFGH", "verification_uri": "https://example.test/device", "interval": 0, "expires_in": 600}),
    );
    h.mock.script("POST /auth/device/poll", 200, json!({"access_token": "step-up-access", "expires_in": 900, "scope": "resources:recovery", "token_type": "Bearer"}));
}

#[test]
fn headless_archive_without_yes_is_refused_before_any_request() {
    let mut prompt = headless();
    let error = confirm(Some("shop"), &mut prompt).unwrap_err();
    assert_eq!(exit_code(&error), Some(2));
    assert_eq!(error.to_string(), "pass --yes to confirm without a prompt");
    assert!(prompt.asked.is_empty());
}

#[tokio::test]
async fn archive_with_yes_sends_the_confirm_phrase_and_a_fresh_key_per_invocation() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 202, view("archiving"));
    archive(&h.api, Some("Shop"), &Confirmed::Yes, false, false, fast()).await.unwrap();
    archive(&h.api, Some("shop"), &Confirmed::Yes, false, true, fast()).await.unwrap();
    let sent = h.mock.seen(ARCHIVE);
    assert_eq!(sent.len(), 2);
    for request in &sent {
        assert_eq!(request.body, json!({"confirm": "archive shop"}));
        assert_eq!(request.bearer.as_deref(), Some("access"));
        uuid::Uuid::parse_str(request.key.as_deref().unwrap()).unwrap();
    }
    assert_ne!(sent[0].key, sent[1].key, "every invocation has its own Idempotency-Key");
    assert!(h.mock.seen(VIEW).is_empty(), "--yes reads nothing before the request");
}

#[tokio::test]
async fn archive_reauth_steps_up_and_retries_once_with_the_same_key() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 403, json!({"code": "AXRESOURCE_REAUTH_REQUIRED"}));
    h.mock.script(ARCHIVE, 202, view("archiving"));
    script_step_up(&h);
    archive(&h.api, Some("shop"), &Confirmed::Yes, false, false, fast()).await.unwrap();
    let sent = h.mock.seen(ARCHIVE);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].bearer.as_deref(), Some("access"));
    assert_eq!(sent[1].bearer.as_deref(), Some("step-up-access"));
    assert_eq!(sent[0].key, sent[1].key);
    assert_eq!(sent[0].body, sent[1].body);
    assert_eq!(h.mock.seen("POST /auth/device/start")[0].body, json!({"scope": "resources:recovery"}));
}

#[tokio::test]
async fn archive_reauth_after_step_up_fails_with_the_code_and_no_third_try() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 403, json!({"code": "AXRESOURCE_REAUTH_REQUIRED"}));
    script_step_up(&h);
    let error = archive(&h.api, Some("shop"), &Confirmed::Yes, false, false, fast()).await.unwrap_err().to_string();
    assert!(error.starts_with("AXRESOURCE_REAUTH_REQUIRED: This action needs a recent sign-in. Run `axiom login --step-up`"), "{error}");
    assert_eq!(h.mock.seen(ARCHIVE).len(), 2);
    assert_eq!(h.mock.seen("POST /auth/device/start").len(), 1);
}

#[tokio::test]
async fn blocked_archive_lists_each_database_and_reason_with_a_hint() {
    let h = shop().await;
    h.mock.script(
        ARCHIVE,
        409,
        json!({"code": "AXPROJECT_ARCHIVE_BLOCKED", "message": "SERVER_TEXT", "blockers": [
            {"resource_id": "r1", "database_name": "preview-1", "reason": "branch"},
            {"resource_id": "r2", "database_name": "legacy", "reason": "byo"},
            {"resource_id": "r3", "database_name": "production", "reason": "busy"},
            {"resource_id": "r4", "database_name": "staging", "reason": "state"},
            {"resource_id": "r5", "database_name": "eu\u{1b}[31m", "reason": "unqualified"},
            {"resource_id": "r6", "database_name": "other", "reason": "future_reason"}
        ]}),
    );
    let error = archive(&h.api, Some("shop"), &Confirmed::Yes, false, false, fast()).await.unwrap_err().to_string();
    let lines: Vec<&str> = error.lines().collect();
    assert_eq!(lines[0], "AXPROJECT_ARCHIVE_BLOCKED: `shop` cannot be archived now. Nothing was changed. Blocked by:");
    assert!(lines[1].starts_with("  - preview-1: a branch database. Delete the branch first (`axiom branch delete`)"), "{error}");
    assert!(lines[2].starts_with("  - legacy: a database connected from your own provider account (bring your own)"), "{error}");
    assert!(lines[3].starts_with("  - production: another operation is running on it. Wait for it to finish (`axiom project resources --project shop`)"), "{error}");
    assert!(lines[4].starts_with("  - staging: its status does not allow an archive now"), "{error}");
    assert!(lines[5].starts_with("  - eu[31m: not yet qualified for project archive"), "control characters removed: {error}");
    assert!(lines[6].starts_with("  - other: not archivable now (future_reason)."), "{error}");
    assert!(error.ends_with("(HTTP 409)"));
    assert!(!error.contains("SERVER_TEXT"));
}

#[tokio::test]
async fn known_refusals_have_fixed_words() {
    let cases = [
        (ARCHIVE, 409, "AXPROJECT_ARCHIVE_DISABLED", "AXPROJECT_ARCHIVE_DISABLED: Project archive is not enabled for `shop`. Nothing was changed. (HTTP 409)"),
        (ARCHIVE, 403, "AXRESOURCE_DENIED", "AXRESOURCE_DENIED: Only a project Owner can archive or restore `shop`. (HTTP 403)"),
        (ARCHIVE, 409, "AXRESOURCE_CONFLICT", "AXRESOURCE_CONFLICT: A restore of `shop` is running, or this request's Idempotency-Key was already used for a different request. Check `axiom project archive-status shop` and retry. (HTTP 409)"),
        (RESTORE, 409, "AXPROJECT_NOT_ARCHIVED", "AXPROJECT_NOT_ARCHIVED: `shop` has not been archived, so there is nothing to restore. (HTTP 409)"),
        (RESTORE, 409, "AXRESOURCE_CONFLICT", "AXRESOURCE_CONFLICT: An archive of `shop` is still running, or another operation is running on one of its archived databases. Wait until it finishes (`axiom project archive-status shop` shows each database's operation), then retry the restore. (HTTP 409)"),
        (RESTORE, 403, "AXRESOURCE_DENIED", "AXRESOURCE_DENIED: Only a project Owner can archive or restore `shop`. (HTTP 403)"),
        (RESTORE, 409, "AXPROJECT_ARCHIVE_DISABLED", "AXPROJECT_ARCHIVE_DISABLED: Project archive is not enabled for `shop`. Nothing was changed. (HTTP 409)"),
    ];
    for (route, status, code, expected) in cases {
        let h = shop().await;
        h.mock.script(route, status, json!({"code": code}));
        let error = if route == ARCHIVE {
            archive(&h.api, Some("shop"), &Confirmed::Yes, false, false, fast()).await
        } else {
            restore(&h.api, Some("shop"), false, false, fast()).await
        }
        .unwrap_err()
        .to_string();
        assert_eq!(error, expected);
        assert_eq!(h.mock.seen(route).len(), 1, "{code} is not retried");
    }
}

#[tokio::test]
async fn restore_posts_an_empty_body_with_a_key_and_the_session_only() {
    let h = shop().await;
    h.mock.script(RESTORE, 202, view("restoring"));
    restore(&h.api, Some("shop"), false, false, fast()).await.unwrap();
    let sent = h.mock.seen(RESTORE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body, json!({}));
    assert_eq!(sent[0].bearer.as_deref(), Some("access"));
    uuid::Uuid::parse_str(sent[0].key.as_deref().unwrap()).unwrap();
    assert!(h.mock.seen("POST /auth/device/start").is_empty(), "restore needs no step-up");
}

#[tokio::test]
async fn an_archived_project_is_found_in_the_archived_list() {
    let h = harness().await;
    // The plain list hides archived projects; the second answer is `?archived=true`.
    h.mock.script(PROJECTS, 200, json!([{"id": "a", "slug": "other", "name": "Other"}]));
    h.mock.script(PROJECTS, 200, json!([{"id": "11111111-1111-4111-8111-111111111111", "slug": "shop", "name": "Shop", "archive": {"state": "archived"}}]));
    h.mock.script(RESTORE, 202, view("restored"));
    restore(&h.api, Some("11111111-1111-4111-8111-111111111111"), false, false, fast()).await.unwrap();
    let lists = h.mock.seen(PROJECTS);
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0].query, None);
    assert_eq!(lists[1].query.as_deref(), Some("archived=true"));
    assert_eq!(h.mock.seen(RESTORE).len(), 1);
    // Both lists are always read, so an archived project's exact slug is never missed.
    let h = shop().await;
    h.mock.script(RESTORE, 202, view("restored"));
    restore(&h.api, Some("shop"), false, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen(PROJECTS).len(), 2);
}

#[tokio::test]
async fn archive_wait_polls_until_archived() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 202, view("archiving"));
    h.mock.script(VIEW, 200, view("archiving"));
    h.mock.script(VIEW, 200, view("archiving"));
    h.mock.script(VIEW, 200, view("archived"));
    archive(&h.api, Some("shop"), &Confirmed::Yes, true, true, fast()).await.unwrap();
    assert_eq!(h.mock.seen(VIEW).len(), 3);
    assert_eq!(h.mock.seen(ARCHIVE).len(), 1, "polling never re-sends the archive");
}

#[tokio::test]
async fn archive_wait_reports_the_databases_that_failed() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 202, view("archiving"));
    let mut failed = view("failed");
    failed["databases"] = json!([
        {"resource_id": "r1", "database_name": "production", "status": "ready", "operation_id": "o1", "operation_status": "failed", "failure_code": "AXRESOURCE_SNAPSHOT_UNVERIFIED"},
        {"resource_id": "r2", "database_name": "analytics", "status": "archived", "operation_id": "o2", "operation_status": "succeeded", "failure_code": ""}
    ]);
    h.mock.script(VIEW, 200, failed);
    let error = archive(&h.api, Some("shop"), &Confirmed::Yes, true, false, fast()).await.unwrap_err().to_string();
    assert!(error.starts_with("The archive of shop failed"), "{error}");
    assert!(error.contains("  - production (AXRESOURCE_SNAPSHOT_UNVERIFIED)"), "{error}");
    assert!(!error.contains("analytics"), "{error}");
    assert!(error.contains("`axiom project restore shop`"), "{error}");
}

#[tokio::test]
async fn restore_wait_polls_until_restored_and_times_out_with_a_hint() {
    let h = shop().await;
    h.mock.script(RESTORE, 202, view("restoring"));
    h.mock.script(VIEW, 200, view("restoring"));
    h.mock.script(VIEW, 200, view("restored"));
    restore(&h.api, Some("shop"), true, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen(VIEW).len(), 2);

    let h = shop().await;
    h.mock.script(RESTORE, 202, view("restoring"));
    h.mock.script(VIEW, 200, view("restoring"));
    let poll = Poll { interval: Duration::from_millis(5), limit: Duration::from_millis(30) };
    let error = restore(&h.api, Some("shop"), true, false, poll).await.unwrap_err().to_string();
    assert!(error.contains("is still restoring") && error.contains("`axiom project archive-status shop`"), "{error}");
}

#[tokio::test]
async fn an_immediate_terminal_state_needs_no_poll() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 202, view("archived"));
    archive(&h.api, Some("shop"), &Confirmed::Yes, true, false, fast()).await.unwrap();
    assert!(h.mock.seen(VIEW).is_empty());
}

#[test]
fn the_terminal_prompt_comes_first_and_needs_the_exact_phrase() {
    // `confirm` takes no `Api`: it cannot send anything.
    let mut wrong = typed("archive Shop");
    let error = confirm(Some("shop"), &mut wrong).unwrap_err();
    assert_eq!(exit_code(&error), Some(2));
    assert_eq!(error.to_string(), "the confirmation did not match; nothing was sent");
    assert_eq!(wrong.asked, vec!["Type `archive shop` to confirm"]);

    let mut right = typed(" archive shop ");
    assert_eq!(confirm(Some("shop"), &mut right).unwrap(), "shop");
    assert_eq!(right.asked, vec!["Type `archive shop` to confirm"]);
}

#[test]
fn the_prompt_is_refused_unasked_for_an_argument_that_is_not_a_slug() {
    for given in [None, Some(""), Some("Shop"), Some("my shop"), Some("shop\u{1b}[2J"), Some(" shop")] {
        let mut prompt = typed("archive shop");
        let error = confirm(given, &mut prompt).unwrap_err();
        assert_eq!(exit_code(&error), Some(2), "{given:?}");
        assert!(error.to_string().contains("slug"), "{given:?}: {error}");
        assert!(!error.to_string().contains('\u{1b}'), "{given:?}: {error}");
        assert!(prompt.asked.is_empty(), "{given:?}");
    }
}

#[tokio::test]
async fn a_typed_confirmation_archives_after_the_lookup() {
    let h = shop().await;
    h.mock.script(VIEW, 200, view("none"));
    h.mock.script(ARCHIVE, 202, view("archiving"));
    archive(&h.api, Some("shop"), &Confirmed::Typed("shop".into()), false, false, fast()).await.unwrap();
    let sent = h.mock.seen(ARCHIVE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body, json!({"confirm": "archive shop"}));
    assert_eq!(h.mock.seen(VIEW).len(), 1, "the project is read after the confirmation");
}

#[tokio::test]
async fn a_typed_slug_that_resolves_to_another_project_sends_no_write() {
    // `shop` is the name of the project whose slug is `shop-2`.
    let h = harness().await;
    h.mock.script(PROJECTS, 200, json!([{"id": "22222222-2222-4222-8222-222222222222", "slug": "shop-2", "name": "shop"}]));
    h.mock.script("POST /api/v1/projects/shop-2/archive", 202, view("archiving"));
    let error = archive(&h.api, Some("shop"), &Confirmed::Typed("shop".into()), false, false, fast()).await.unwrap_err().to_string();
    assert!(error.contains("you confirmed `archive shop`, but `shop` is the project `shop-2`. Nothing was archived."), "{error}");
    assert!(error.contains("`axiom project archive shop-2`"), "{error}");
    // A typed id resolves to its project's slug: refused the same way.
    let id = "22222222-2222-4222-8222-222222222222";
    let error = archive(&h.api, Some(id), &Confirmed::Typed(id.into()), false, false, fast()).await.unwrap_err().to_string();
    assert!(error.contains("is the project `shop-2`"), "{error}");
    assert!(h.mock.requests().iter().all(|r| !r.route.starts_with("POST")), "{:?}", h.mock.requests());
}

#[tokio::test]
async fn a_typed_confirmation_is_still_refused_when_the_archive_would_be() {
    let h = shop().await;
    let mut blocked = view("none");
    blocked["blockers"] = json!([{"resource_id": "r1", "database_name": "preview-1", "reason": "branch"}]);
    h.mock.script(VIEW, 200, blocked);
    let error = archive(&h.api, Some("shop"), &Confirmed::Typed("shop".into()), false, false, fast()).await.unwrap_err().to_string();
    assert!(error.starts_with("AXPROJECT_ARCHIVE_BLOCKED: `shop` cannot be archived now."), "{error}");
    assert!(error.contains("  - preview-1: a branch database"), "{error}");
    assert!(h.mock.seen(ARCHIVE).is_empty());

    let h = shop().await;
    h.mock.script(VIEW, 200, json!({"enabled": false, "state": "none", "databases": [], "blockers": []}));
    let error = archive(&h.api, Some("shop"), &Confirmed::Typed("shop".into()), false, false, fast()).await.unwrap_err().to_string();
    assert!(error.starts_with("AXPROJECT_ARCHIVE_DISABLED"), "{error}");
    assert!(h.mock.seen(ARCHIVE).is_empty());
}

#[test]
fn status_rendering_shows_state_databases_and_blockers() {
    let mut value = view("none");
    value["blockers"] = json!([{"resource_id": "r1", "database_name": "preview-1", "reason": "busy"}]);
    let lines = render_view("shop", &value);
    assert_eq!(lines[0], "Project shop: not archived");
    assert!(lines.contains(&"  production  ready".to_string()), "{lines:?}");
    assert!(lines.contains(&"An archive request now would be refused:".to_string()), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("  - preview-1: another operation is running on it")), "{lines:?}");

    let mut archived = view("archived");
    archived["completed_at"] = json!("2026-11-14T10:20:00Z");
    archived["databases"][0]["status"] = json!("archived");
    archived["databases"][0]["operation_status"] = json!("succeeded");
    let lines = render_view("shop", &archived);
    assert!(lines[0].starts_with("Project shop: archived"));
    assert_eq!(lines[1], "  requested 2026-11-14T10:00:00Z, completed 2026-11-14T10:20:00Z");
    assert!(lines[2].contains("`axiom project restore shop`"));
    assert!(lines.contains(&"  production  archived  (operation succeeded)".to_string()), "{lines:?}");

    assert_eq!(render_view("shop", &json!({"enabled": false})), vec!["Project archive is not enabled for `shop`."]);
}

#[tokio::test]
async fn archive_status_reads_the_view() {
    let h = shop().await;
    h.mock.script(VIEW, 200, view("archiving"));
    status(&h.api, Some("shop"), true).await.unwrap();
    assert_eq!(h.mock.seen(VIEW).len(), 1);
    let h = shop().await;
    h.mock.script(VIEW, 403, json!({"code": "AXRESOURCE_DENIED"}));
    let error = status(&h.api, Some("shop"), false).await.unwrap_err().to_string();
    assert_eq!(error, "AXRESOURCE_DENIED: Your role in `shop` does not allow reading its archive status. (HTTP 403)");
}

#[tokio::test]
async fn list_archived_asks_for_archived_projects_only() {
    let h = harness().await;
    h.mock.script(PROJECTS, 200, json!([{"id": "1", "slug": "shop", "name": "Shop", "archive": {"state": "archived", "requested_at": "2026-11-14T10:00:00Z"}}]));
    let lines = list_archived(&h.api).await.unwrap();
    assert_eq!(lines[1], "  • Shop  shop  (archived, since 2026-11-14T10:00:00Z)");
    assert_eq!(h.mock.seen(PROJECTS)[0].query.as_deref(), Some("archived=true"));
    let h = harness().await;
    h.mock.script(PROJECTS, 200, json!([]));
    assert_eq!(list_archived(&h.api).await.unwrap(), vec!["No archived projects."]);
}

/// The plain list, then the `?archived=true` list.
async fn lists(plain: Value, archived: Value) -> Harness {
    let h = harness().await;
    h.mock.script(PROJECTS, 200, plain);
    h.mock.script(PROJECTS, 200, archived);
    h
}

#[tokio::test]
async fn an_archived_projects_exact_slug_beats_a_visible_projects_name() {
    // `shop` is archived; a new visible project is named "shop" with the slug shop-1.
    let plain = json!([{"id": "v", "slug": "shop-1", "name": "shop"}]);
    let archived = json!([{"id": "a", "slug": "shop", "name": "Shop", "archive": {"state": "archived"}}]);
    let h = lists(plain.clone(), archived.clone()).await;
    h.mock.script(RESTORE, 202, view("restoring"));
    restore(&h.api, Some("shop"), false, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen(RESTORE).len(), 1);
    assert!(h.mock.seen("POST /api/v1/projects/shop-1/restore").is_empty());

    let h = lists(plain, archived).await;
    h.mock.script(ARCHIVE, 202, view("archived"));
    archive(&h.api, Some("shop"), &Confirmed::Yes, false, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen(ARCHIVE).len(), 1);
    assert!(h.mock.seen("POST /api/v1/projects/shop-1/archive").is_empty());
}

#[tokio::test]
async fn a_tie_prefers_the_archived_project_for_restore_only() {
    let plain = json!([{"id": "v", "slug": "shop-1", "name": "Shop"}]);
    let archived = json!([{"id": "a", "slug": "shop-2", "name": "Shop"}]);
    let h = lists(plain.clone(), archived.clone()).await;
    h.mock.script("POST /api/v1/projects/shop-2/restore", 202, view("restoring"));
    restore(&h.api, Some("Shop"), false, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen("POST /api/v1/projects/shop-2/restore").len(), 1);

    let h = lists(plain, archived).await;
    let error = archive(&h.api, Some("Shop"), &Confirmed::Yes, false, false, fast()).await.unwrap_err().to_string();
    assert_eq!(error, "`Shop` matches 2 projects: shop-1; shop-2 (archived). Pass the slug instead.");
    assert!(h.mock.requests().iter().all(|r| r.route.starts_with("GET ")), "nothing is archived on a tie");

    // The same project in both lists (an API that ignores ?archived=true) is one project.
    let same = json!([{"id": "v", "slug": "shop", "name": "Shop"}]);
    let h = lists(same.clone(), same).await;
    h.mock.script(RESTORE, 202, view("restored"));
    restore(&h.api, Some("Shop"), false, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen(RESTORE).len(), 1);
}

#[tokio::test]
async fn a_slug_that_is_not_plain_is_refused_before_any_write() {
    for slug in ["shop/../other", "Shop", "sh\u{1b}[31mop", "shop?x=1", "shop op"] {
        let h = lists(json!([{"id": "x", "slug": slug, "name": "Evil"}]), json!([])).await;
        let error = archive(&h.api, Some("Evil"), &Confirmed::Yes, false, false, fast()).await.unwrap_err().to_string();
        assert!(error.contains("not lowercase letters, digits and hyphens; nothing was sent"), "{slug}: {error}");
        assert!(!error.contains('\u{1b}'));
        assert!(h.mock.requests().iter().all(|r| r.route.starts_with("GET /api/v1/projects") && !r.route.contains("archive")), "{slug}");
        let h = lists(json!([]), json!([{"id": "x", "slug": slug, "name": "Evil"}])).await;
        assert!(restore(&h.api, Some("Evil"), false, false, fast()).await.is_err());
        assert!(h.mock.requests().iter().all(|r| r.route == PROJECTS));
    }
    assert!(valid_slug("shop-1") && !valid_slug("") && !valid_slug("shop_1"));
}

#[tokio::test]
async fn wait_rides_out_server_errors_and_stops_on_a_refusal() {
    let h = shop().await;
    h.mock.script(ARCHIVE, 202, view("archiving"));
    h.mock.script(VIEW, 503, json!({"code": "AXRESOURCE_UNAVAILABLE"}));
    h.mock.script(VIEW, 502, Value::Null);
    h.mock.script(VIEW, 429, json!({}));
    h.mock.script(VIEW, 200, view("archiving"));
    h.mock.script(VIEW, 500, json!({}));
    h.mock.script(VIEW, 200, view("archived"));
    archive(&h.api, Some("shop"), &Confirmed::Yes, true, false, fast()).await.unwrap();
    assert_eq!(h.mock.seen(VIEW).len(), 6);

    let h = shop().await;
    h.mock.script(ARCHIVE, 202, view("archiving"));
    h.mock.script(VIEW, 403, json!({"code": "AXRESOURCE_DENIED"}));
    let error = archive(&h.api, Some("shop"), &Confirmed::Yes, true, false, fast()).await.unwrap_err().to_string();
    assert!(error.starts_with("AXRESOURCE_DENIED"), "{error}");
    assert_eq!(h.mock.seen(VIEW).len(), 1, "a 4xx stops the wait at once");

    let h = shop().await;
    h.mock.script(RESTORE, 202, view("restoring"));
    h.mock.script(VIEW, 503, json!({"code": "AXRESOURCE_UNAVAILABLE"}));
    let error = restore(&h.api, Some("shop"), true, false, fast()).await.unwrap_err().to_string();
    assert!(error.starts_with("Stopped waiting: 10 status reads in a row failed; the last: AXRESOURCE_UNAVAILABLE"), "{error}");
    assert!(error.contains("The restore may still be running"), "{error}");
    assert_eq!(h.mock.seen(VIEW).len(), 10);
}

#[tokio::test]
async fn a_failed_or_timed_out_wait_hands_back_the_last_view() {
    let h = shop().await;
    h.mock.script(VIEW, 200, view("restoring"));
    let poll = Poll { interval: Duration::from_millis(5), limit: Duration::from_millis(30) };
    let (last, error) = wait_for(&h.api, "shop", Kind::Restore, view("restoring"), poll, false).await.unwrap_err();
    assert_eq!(last["state"], "restoring");
    assert!(error.to_string().contains("is still restoring"));

    let h = shop().await;
    h.mock.script(VIEW, 200, view("failed"));
    let (last, error) = wait_for(&h.api, "shop", Kind::Archive, view("archiving"), fast(), false).await.unwrap_err();
    assert_eq!(last["state"], "failed");
    assert!(error.to_string().starts_with("The archive of shop failed"));
}

#[test]
fn status_of_an_archived_project_is_shown_when_archive_is_off() {
    let mut value = view("archived");
    value["enabled"] = json!(false);
    let lines = render_view("shop", &value);
    assert!(lines[0].starts_with("Project shop: archived"), "{lines:?}");
    assert_eq!(lines[1], "  Project archive is not enabled for `shop`: it cannot be archived again, but it can be restored.");
    assert!(lines.contains(&"  production  ready".to_string()), "{lines:?}");
}
