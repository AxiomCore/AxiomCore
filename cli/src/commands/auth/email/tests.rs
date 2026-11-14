use super::*;
use crate::commands::auth::Action;
use crate::commands::cloud_mock::Mock;
use axiom_cloud::CloudClient;
use std::io::IsTerminal;

const ENV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const CANARY: &str = "SYNTHETIC_SMTP_PASSWORD_CANARY";

fn args() -> AuthArgs {
    AuthArgs { project: Some("demo".into()), org: None, environment: ENV.into(), json: true, action: Action::Status }
}

async fn fixture() -> (Mock, Target) {
    let mock = Mock::default();
    let base = mock.start().await;
    let target = Target { client: CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap(), slug: "demo".into(), environment: ENV.into() };
    (mock, target)
}

fn admin_route(method: &str, rest: &str) -> String {
    format!("{method} /api/v1/projects/demo/auth/environments/{ENV}/{rest}")
}

fn configured_smtp() -> Value {
    json!({"host": "smtp.example.net", "port": 587, "username": "mailer", "password_set": true, "from_address": "auth@example.net", "from_name": "Example",
        "revision": 4, "last_test": {"result": "ok", "at": "2026-11-01T00:00:00Z"}, "configured": true, "updated_at": "2026-11-01T00:00:00Z"})
}

fn smtp_set(port: Option<&str>) -> SmtpAction {
    SmtpAction::Set { host: None, port: port.map(str::to_string), username: None, from_address: None, from_name: None, clear_from_name: false, password_stdin: false, password_prompt: false }
}

#[tokio::test]
async fn email_usage_and_unsuppress_call_their_routes() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", "email/usage"), 200, json!({"period_start": "2026-11-01T00:00:00Z", "sent": 12, "budget": 1000, "recovery_reserve": 100, "smtp_sent": 0,
        "daily": {"general": {"used": 3, "limit": 200}}, "suppressions": {"environment": 1, "global_from_environment": 0}}));
    email_with(&args(), &target, &EmailAction::Usage).await.unwrap();
    let seen = mock.seen(&admin_route("GET", "email/usage"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].query, None);

    mock.script(&admin_route("POST", "email/suppressions/remove"), 200, json!({"removed": true}));
    // Removing a suppression is confirmed: without a terminal it needs --yes, and nothing is sent.
    if !std::io::stdin().is_terminal() {
        let unconfirmed = email_with(&args(), &target, &EmailAction::Unsuppress { email: "bounced@example.com".into(), yes: false }).await.unwrap_err();
        assert_eq!(unconfirmed.downcast_ref::<crate::commands::auth::Exit>().unwrap().code, 2);
        assert!(unconfirmed.to_string().contains("--yes"));
        assert!(mock.seen(&admin_route("POST", "email/suppressions/remove")).is_empty());
    }
    email_with(&args(), &target, &EmailAction::Unsuppress { email: "bounced@example.com".into(), yes: true }).await.unwrap();
    let seen = mock.seen(&admin_route("POST", "email/suppressions/remove"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({"email": "bounced@example.com"}));

    // Not an address: refused locally, nothing sent.
    assert!(email_with(&args(), &target, &EmailAction::Unsuppress { email: "not-an-address".into(), yes: true }).await.is_err());
    assert_eq!(mock.seen(&admin_route("POST", "email/suppressions/remove")).len(), 1);
}

#[tokio::test]
async fn smtp_get_shows_only_whether_the_password_is_set() {
    let (mock, target) = fixture().await;
    let mut reply = configured_smtp();
    reply["password"] = json!(CANARY);
    mock.script(&admin_route("GET", "smtp"), 200, reply.clone());
    smtp_with(&args(), &target, &SmtpAction::Get { reveal_email: false }, None).await.unwrap();
    assert_eq!(mock.seen(&admin_route("GET", "smtp")).len(), 1);
    // The output path: secrets are stripped, the flag stays, addresses are masked.
    let mut shown = reply;
    sanitize(&mut shown, false);
    assert!(!shown.to_string().contains(CANARY));
    assert_eq!(shown["password_set"], true);
    assert_eq!(shown["from_address"], "a***@e***.net");
    assert!(smtp_lines(&shown).join("\n").contains("password set"));
}

#[tokio::test]
async fn smtp_set_creates_with_the_password_and_revision_zero() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", "smtp"), 200, json!({"host": "", "port": 0, "username": "", "password_set": false, "from_address": "", "from_name": null, "revision": 0, "last_test": {"result": "untested"}, "configured": false}));
    mock.script(&admin_route("PUT", "smtp"), 200, json!({"host": "smtp.example.net", "port": 465, "username": "mailer", "password_set": true, "from_address": "auth@example.net", "from_name": "Example", "revision": 1, "last_test": {"result": "untested"}, "configured": true}));
    let action = SmtpAction::Set {
        host: Some(" smtp.example.net ".into()),
        port: Some("465".into()),
        username: Some("mailer".into()),
        from_address: Some("auth@example.net".into()),
        from_name: Some("Example".into()),
        clear_from_name: false,
        password_stdin: true,
        password_prompt: false,
    };
    // Without a password a new configuration is refused before the PUT.
    assert!(smtp_with(&args(), &target, &action, None).await.is_err());
    assert!(mock.seen(&admin_route("PUT", "smtp")).is_empty());

    smtp_with(&args(), &target, &action, Some(Zeroizing::new(CANARY.to_string()))).await.unwrap();
    let seen = mock.seen(&admin_route("PUT", "smtp"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({"host": "smtp.example.net", "port": 465, "username": "mailer", "from_address": "auth@example.net", "from_name": "Example", "revision": 0, "password": CANARY}));
}

#[tokio::test]
async fn smtp_set_keeps_current_fields_and_the_stored_password() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", "smtp"), 200, configured_smtp());
    mock.script(&admin_route("PUT", "smtp"), 200, configured_smtp());
    smtp_with(&args(), &target, &smtp_set(Some("2525")), None).await.unwrap();
    let seen = mock.seen(&admin_route("PUT", "smtp"));
    assert_eq!(seen[0].body, json!({"host": "smtp.example.net", "port": 2525, "username": "mailer", "from_address": "auth@example.net", "from_name": "Example", "revision": 4}));

    let mut clear = smtp_set(None);
    if let SmtpAction::Set { clear_from_name, .. } = &mut clear {
        *clear_from_name = true;
    }
    smtp_with(&args(), &target, &clear, None).await.unwrap();
    let seen = mock.seen(&admin_route("PUT", "smtp"));
    assert_eq!(seen[1].body["from_name"], Value::Null);
    assert_eq!(seen[1].body["port"], 587);
}

#[test]
fn smtp_bodies_check_the_backend_rules() {
    let current = configured_smtp();
    assert!(smtp_body(&current, Some("smtp example"), None, None, None, None, false).is_err());
    assert!(smtp_body(&current, None, Some("25"), None, None, None, false).is_err());
    assert!(smtp_body(&current, None, None, Some("a\nb"), None, None, false).is_err());
    assert!(smtp_body(&current, None, None, None, Some("nobody"), None, false).is_err());
    assert!(smtp_body(&current, None, None, None, None, Some("Visit example.com"), false).is_err());
    assert!(smtp_body(&current, None, None, None, None, Some("A <b>"), false).is_err());
    // A Reader's masked view never fills a new body.
    let masked = json!({"host": "smtp.example.net", "port": 587, "username": "***", "from_address": "***", "configured": true, "revision": 2});
    assert!(smtp_body(&masked, None, None, None, None, None, false).is_err());
    // Nothing configured: every field is needed.
    assert!(smtp_body(&json!({"configured": false}), Some("smtp.example.net"), None, Some("u"), None, None, false).is_err());
}

#[tokio::test]
async fn smtp_test_and_remove_call_their_routes() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("POST", "smtp/test"), 200, json!({"result": "ok"}));
    smtp_with(&args(), &target, &SmtpAction::Test { to: "owner@example.com".into() }, None).await.unwrap();
    assert_eq!(mock.seen(&admin_route("POST", "smtp/test"))[0].body, json!({"to": "owner@example.com"}));
    assert!(smtp_with(&args(), &target, &SmtpAction::Test { to: "nobody".into() }, None).await.is_err());
    assert_eq!(mock.seen(&admin_route("POST", "smtp/test")).len(), 1);

    mock.script(&admin_route("DELETE", "smtp"), 200, json!({"deleted": true}));
    if !std::io::stdin().is_terminal() {
        assert!(smtp_with(&args(), &target, &SmtpAction::Remove { yes: false }, None).await.is_err());
        assert!(mock.seen(&admin_route("DELETE", "smtp")).is_empty());
    }
    smtp_with(&args(), &target, &SmtpAction::Remove { yes: true }, None).await.unwrap();
    let seen = mock.seen(&admin_route("DELETE", "smtp"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, Value::Null);
}

#[tokio::test]
async fn templates_list_show_and_reset() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", "templates"), 200, json!({"templates": [{"kind": "email_code", "overridden": false, "subject": null, "revision": 0, "updated_at": null}]}));
    templates_with(&args(), &target, &TemplatesAction::List { reveal_text: false }).await.unwrap();
    assert_eq!(mock.seen(&admin_route("GET", "templates")).len(), 1);

    let view = json!({"kind": "magic_link", "overridden": true, "subject": "Sign in", "intro": null, "outro": null, "revision": 3, "updated_at": "2026-11-01T00:00:00Z",
        "defaults": {"subject": "Your sign-in link", "intro": "Use this link.", "outro": "Ignore it otherwise."}});
    mock.script(&admin_route("GET", "templates/magic_link"), 200, view.clone());
    templates_with(&args(), &target, &TemplatesAction::Show { kind: "magic_link".into(), reveal_text: false }).await.unwrap();
    assert_eq!(mock.seen(&admin_route("GET", "templates/magic_link")).len(), 1);
    assert_eq!(template_lines(&view, true)[2], "Intro: (default) Use this link.");
    let mut hidden = view.clone();
    hide_text(&mut hidden, Some("magic_link"));
    let lines = template_lines(&hidden, false);
    assert!(lines[1].starts_with("Subject: set, 7 bytes, no variables, sha256 "), "{lines:?}");
    assert!(lines[2].starts_with("Intro: default text (set, 14 bytes"), "{lines:?}");
    assert!(!lines.join("\n").contains("Sign in") && !lines.join("\n").contains("Use this link"), "{lines:?}");

    // An unknown kind is a usage error; nothing is sent.
    assert!(templates_with(&args(), &target, &TemplatesAction::Show { kind: "welcome".into(), reveal_text: false }).await.is_err());
    assert!(mock.seen(&admin_route("GET", "templates/welcome")).is_empty());

    mock.script(&admin_route("DELETE", "templates/invitation"), 200, json!({"deleted": true}));
    if !std::io::stdin().is_terminal() {
        assert!(templates_with(&args(), &target, &TemplatesAction::Reset { kind: "invitation".into(), yes: false }).await.is_err());
        assert!(mock.seen(&admin_route("DELETE", "templates/invitation")).is_empty());
    }
    templates_with(&args(), &target, &TemplatesAction::Reset { kind: "invitation".into(), yes: true }).await.unwrap();
    assert_eq!(mock.seen(&admin_route("DELETE", "templates/invitation")).len(), 1);
    assert_eq!(mock.seen(&admin_route("DELETE", "templates/invitation"))[0].body, Value::Null);
}

#[tokio::test]
async fn templates_set_merges_with_the_current_text_and_sends_its_revision() {
    let (mock, target) = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let outro = dir.path().join("outro.txt");
    std::fs::write(&outro, "Sent to {{recipient_email}}.\n").unwrap();
    let subject = dir.path().join("subject.txt");
    std::fs::write(&subject, "Welcome to {{app_name}}\n").unwrap();
    let empty = dir.path().join("empty.txt");
    std::fs::write(&empty, "").unwrap();
    let link = dir.path().join("link.txt");
    std::fs::write(&link, "Go to https://evil.example").unwrap();
    mock.script(&admin_route("GET", "templates/signup"), 200, json!({"kind": "signup", "overridden": true, "subject": "Old", "intro": "Old intro", "outro": null, "revision": 2, "defaults": {}}));
    mock.script(&admin_route("PUT", "templates/signup"), 200, json!({"kind": "signup", "overridden": true, "subject": "Welcome to {{app_name}}", "intro": "Old intro", "outro": "Sent to {{recipient_email}}.", "revision": 3, "defaults": {}}));
    let action = TemplatesAction::Set { kind: "signup".into(), subject_file: Some(subject), intro_file: None, outro_file: Some(outro.clone()), reveal_text: false };
    templates_with(&args(), &target, &action).await.unwrap();
    let seen = mock.seen(&admin_route("PUT", "templates/signup"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({"subject": "Welcome to {{app_name}}", "intro": "Old intro", "outro": "Sent to {{recipient_email}}.", "revision": 2}));

    // A first customization is created with revision 0; an empty subject means the default.
    mock.script(&admin_route("GET", "templates/email_code"), 200, json!({"kind": "email_code", "overridden": false, "subject": null, "intro": null, "outro": null, "revision": 0, "defaults": {}}));
    mock.script(&admin_route("PUT", "templates/email_code"), 200, json!({"kind": "email_code", "overridden": true, "revision": 1, "defaults": {}}));
    let first = TemplatesAction::Set { kind: "email_code".into(), subject_file: Some(empty), intro_file: None, outro_file: Some(outro), reveal_text: false };
    templates_with(&args(), &target, &first).await.unwrap();
    assert_eq!(mock.seen(&admin_route("PUT", "templates/email_code"))[0].body, json!({"subject": null, "intro": null, "outro": "Sent to {{recipient_email}}.", "revision": 0}));

    // Nothing to change, or text the backend refuses: no PUT.
    let nothing = TemplatesAction::Set { kind: "signup".into(), subject_file: None, intro_file: None, outro_file: None, reveal_text: false };
    assert!(templates_with(&args(), &target, &nothing).await.is_err());
    let link = TemplatesAction::Set { kind: "signup".into(), subject_file: Some(link), intro_file: None, outro_file: None, reveal_text: false };
    assert!(templates_with(&args(), &target, &link).await.is_err());
    assert_eq!(mock.seen(&admin_route("PUT", "templates/signup")).len(), 1);
}

#[tokio::test]
async fn templates_preview_sends_the_text_to_try_or_nothing() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("POST", "templates/email_code/preview"), 200, json!({"subject": "Your code", "text": "123456", "html": "<p>123456</p>"}));
    let saved = TemplatesAction::Preview { kind: "email_code".into(), subject_file: None, intro_file: None, outro_file: None, html: false, reveal_text: false };
    templates_with(&args(), &target, &saved).await.unwrap();
    assert_eq!(mock.seen(&admin_route("POST", "templates/email_code/preview"))[0].body, json!({}));
    assert!(mock.seen(&admin_route("GET", "templates/email_code")).is_empty());

    let dir = tempfile::tempdir().unwrap();
    let intro = dir.path().join("intro.txt");
    std::fs::write(&intro, "Hello from {{app_name}}.\n\nYour code expires in {{expires_minutes}} minutes.").unwrap();
    mock.script(&admin_route("GET", "templates/email_code"), 200, json!({"kind": "email_code", "overridden": true, "subject": "Code", "intro": "Old", "outro": "Bye", "revision": 5, "defaults": {}}));
    let trial = TemplatesAction::Preview { kind: "email_code".into(), subject_file: None, intro_file: Some(intro), outro_file: None, html: true, reveal_text: false };
    templates_with(&args(), &target, &trial).await.unwrap();
    assert_eq!(
        mock.seen(&admin_route("POST", "templates/email_code/preview"))[1].body,
        json!({"subject": "Code", "intro": "Hello from {{app_name}}.\n\nYour code expires in {{expires_minutes}} minutes.", "outro": "Bye"})
    );
}

#[test]
fn template_text_follows_the_backend_rules() {
    assert_eq!(template_problem("signup", "the intro", "Welcome to {{app_name}}.", 2000), None);
    assert!(template_problem("signup", "the intro", "Your new address is {{new_email}}", 2000).is_some());
    assert_eq!(template_problem("email_change_notice", "the intro", "Your new address is {{new_email}}", 2000), None);
    assert!(template_problem("signup", "the intro", "{{ app_name }}", 2000).is_some());
    assert!(template_problem("signup", "the intro", "see www.example", 2000).is_some());
    assert!(template_problem("signup", "the intro", "mail us at help.example.com", 2000).is_some());
    assert!(template_problem("signup", "the intro", "evil{{expires_minutes}}.com", 2000).is_some());
    assert!(template_problem("signup", "the subject", "two\nlines", 600).is_some());
    assert!(template_problem("signup", "the intro", "bell\u{7}", 2000).is_some());
    assert!(template_problem("signup", "the subject", &"a".repeat(601), 600).is_some());
    assert_eq!(template_problem("signup", "the intro", "two\nlines\tok", 2000), None);
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.txt");
    std::fs::write(&big, "a".repeat(TEMPLATE_BODY_LIMIT + 1)).unwrap();
    assert!(read_template_file(&big).is_err());
    let empty = dir.path().join("empty.txt");
    std::fs::write(&empty, "\n").unwrap();
    assert_eq!(read_template_file(&empty).unwrap(), None);
    let long = dir.path().join("long.txt");
    std::fs::write(&long, "a".repeat(2001)).unwrap();
    assert!(template_body("signup", &json!({}), None, Some(&long), None).is_err());
}

#[test]
fn hidden_template_text_keeps_only_metadata() {
    let text = "Hi {{recipient_email}}, {{app_name}} {{recipient_email}} axr1_SYNTHETIC\nCANARY0123456789";
    let summary = text_summary(&json!(text), None);
    assert_eq!(summary["set"], true);
    assert_eq!(summary["bytes"], text.len());
    assert_eq!(summary["variables"], json!(["{{recipient_email}}", "{{app_name}}"]));
    assert_eq!(summary["sha256"], hex::encode(sha2::Sha256::digest(text.as_bytes())));
    assert_eq!(text_summary(&json!(""), None), json!({"set": false}));
    assert_eq!(text_summary(&Value::Null, None), json!({"set": false}));

    let mut view = json!({"kind": "email_change_confirm", "overridden": true, "revision": 3, "updated_at": "2026-11-01T00:00:00Z", "subject": text, "intro": null,
        "defaults": {"outro": "token ghp_SYNTHETIC0123456789abcdefCANARY"}, "templates": [{"subject": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sig"}], "future_field": "Zanzibar"});
    hide_text(&mut view, None);
    let printed = view.to_string();
    for canary in ["axr1_", "CANARY", "ghp_", "eyJ", "Zanzibar", "Hi "] {
        assert!(!printed.contains(canary), "{canary} in {printed}");
    }
    assert_eq!(view["kind"], "email_change_confirm");
    assert_eq!(view["revision"], 3);
    assert_eq!(view["overridden"], true);
    assert_eq!(view["updated_at"], "2026-11-01T00:00:00Z");
    assert_eq!(view["intro"], json!({"set": false}));
    assert_eq!(view["subject"], summary);
    assert_eq!(view["defaults"]["outro"]["set"], true);
    assert_eq!(view["future_field"], Value::Null);
    assert_eq!(view["unknown_fields"], 1);
}

#[tokio::test]
async fn two_template_fields_cannot_both_read_stdin() {
    let (mock, target) = fixture().await;
    // Refused before any request.
    let both = TemplatesAction::Set { kind: "signup".into(), subject_file: Some("-".into()), intro_file: Some("-".into()), outro_file: None, reveal_text: false };
    assert!(templates_with(&args(), &target, &both).await.is_err());
    let trial = TemplatesAction::Preview { kind: "signup".into(), subject_file: None, intro_file: Some("-".into()), outro_file: Some("-".into()), html: false, reveal_text: false };
    assert!(templates_with(&args(), &target, &trial).await.is_err());
    assert!(mock.seen(&admin_route("GET", "templates/signup")).is_empty());
    assert!(mock.seen(&admin_route("PUT", "templates/signup")).is_empty());
}

#[test]
fn template_subjects_are_never_argv() {
    use clap::Parser;
    #[derive(clap::Parser)]
    struct Harness {
        #[command(subcommand)]
        action: TemplatesAction,
    }
    assert!(Harness::try_parse_from(["t", "set", "signup", "--subject", "Welcome"]).is_err());
    assert!(Harness::try_parse_from(["t", "preview", "signup", "--subject", "Welcome"]).is_err());
    assert!(Harness::try_parse_from(["t", "set", "signup", "--subject-file", "-"]).is_ok());
    assert!(Harness::try_parse_from(["t", "show", "signup", "--reveal-text"]).is_ok());
    #[derive(clap::Parser)]
    struct EmailHarness {
        #[command(subcommand)]
        action: EmailAction,
    }
    assert!(EmailHarness::try_parse_from(["t", "unsuppress", "a@example.com", "--yes"]).is_ok());
}

#[test]
fn unsupported_variables_are_counted_never_named() {
    // P1 re-verification round 3: the hex canary as an unsupported variable.
    let probe = "{{abcdefabcdefabcdefabcdefabcdefab}}";
    let problem = template_problem("signup", "the subject", &format!("Hi {probe} {{{{zzz}}}} {{{{app_name}}}}"), 600).unwrap();
    assert!(problem.starts_with("the subject uses 2 unsupported placeholders; use {{app_name}}"), "{problem}");
    assert!(!problem.contains("abcdef") && !problem.contains("zzz"), "{problem}");
    let problem = template_problem("signup", "the intro", "Your new address is {{new_email}}", 2000).unwrap();
    assert!(problem.contains("1 unsupported placeholder;"), "{problem}");
    // Placeholders the backend pattern does not match are counted too.
    let problem = template_problem("signup", "the outro", "{{ Secret }} {{Foo}} {{app-name}} {{app_name}}", 2000).unwrap();
    assert!(problem.starts_with("the outro uses 3 unsupported placeholders;"), "{problem}");
    assert!(!problem.contains("Secret") && !problem.contains("Foo") && !problem.contains("app-name"), "{problem}");
    let problem = template_problem("signup", "the outro", "Secret}} {{app_name}}", 2000).unwrap();
    assert!(problem.starts_with("the outro has an unclosed or unknown {{variable}}") && !problem.contains("Secret"), "{problem}");

    let summary = text_summary(&json!(format!("{probe} {{{{app_name}}}} {{{{ ABC }}}} {{{{app_name}}}} {{{{new_email}}}}")), None);
    assert_eq!(summary["variables"], json!(["{{app_name}}", "{{new_email}}"]));
    assert_eq!(summary["unsupported_placeholders"], 2);
    assert!(!summary.to_string().contains("abcdef"), "{summary}");
    assert!(describe(&summary).contains("variables {{app_name}} {{new_email}}, 2 unsupported placeholders, sha256 "), "{}", describe(&summary));
    assert_eq!(text_summary(&json!("plain"), None)["unsupported_placeholders"], 0);
    assert!(!describe(&text_summary(&json!("plain"), None)).contains("unsupported"));
    // With the kind known, the summary agrees with the checks: {{new_email}} is only for email_change_notice.
    let signup = text_summary(&json!("Now {{new_email}}"), Some("signup"));
    assert_eq!((&signup["variables"], &signup["unsupported_placeholders"]), (&json!([]), &json!(1)));
    let notice = text_summary(&json!("Now {{new_email}}"), Some("email_change_notice"));
    assert_eq!((&notice["variables"], &notice["unsupported_placeholders"]), (&json!(["{{new_email}}"]), &json!(0)));
    let mut list = json!({"templates": [{"kind": "signup", "subject": "Now {{new_email}}"}, {"kind": "email_change_notice", "subject": "Now {{new_email}}"}]});
    hide_text(&mut list, None);
    assert_eq!(list["templates"][0]["subject"]["unsupported_placeholders"], 1);
    assert_eq!(list["templates"][1]["subject"]["unsupported_placeholders"], 0);
}

#[test]
fn only_documented_metadata_positions_with_valid_values_are_shown() {
    let canary = "AbcDefghiJklMnopQrStUvWxYz_-abcDEF";
    let mut view = json!({"kind": "signup", "updated_at": "2026-11-01T00:00:00.123456Z", "revision": 2, "overridden": true, "subject": {"kind": canary},
        "new_field": {"kind": canary, "updated_at": canary, "created_at": canary, "name": canary, "id": canary}, canary: {canary: canary},
        "defaults": {"kind": "signup", "updated_at": "2026-11-01T00:00:00Z", "intro": [canary, {canary: 1}]},
        "templates": [{"kind": "magic_link", "updated_at": "2026-11-01T00:00:00+01:00", "subject": canary, "nested": [{"kind": "signup"}]}]});
    hide_text(&mut view, Some("signup"));
    assert!(!view.to_string().contains(canary), "{view}");
    assert_eq!(view["kind"], "signup");
    assert_eq!(view["updated_at"], "2026-11-01T00:00:00.123456Z");
    assert_eq!(view["templates"][0]["kind"], "magic_link");
    assert_eq!(view["templates"][0]["updated_at"], "2026-11-01T00:00:00+01:00");
    assert_eq!(view["templates"][0]["subject"]["set"], true);
    // Unknown keys, wherever they are and whatever they are called, are only counted.
    assert_eq!(view["unknown_fields"], 2);
    assert_eq!(view["defaults"]["unknown_fields"], 2);
    assert_eq!(view["templates"][0]["unknown_fields"], 1);
    assert_eq!(view["subject"], json!({"unknown_fields": 1}));
    assert_eq!(view["defaults"]["intro"][0]["set"], true);
    assert_eq!(view["defaults"]["intro"][1], json!({"unknown_fields": 1}));
    // A documented position with an unexpected value is text too.
    for (key, value) in [("kind", canary), ("kind", "Signup"), ("updated_at", canary), ("updated_at", "2026-11-01T00:00:00Z secret"), ("updated_at", "")] {
        let mut probe = json!({ key: value, "templates": [{ key: value }] });
        hide_text(&mut probe, None);
        assert!(probe[key].is_object() && probe["templates"][0][key].is_object(), "{key}={value:?}: {probe}");
    }
}

/// Every path to a string in `value` (JSON pointers).
fn string_paths(value: &Value, at: String, out: &mut Vec<String>) {
    match value {
        Value::String(_) => out.push(at),
        Value::Object(map) => map.iter().for_each(|(k, v)| string_paths(v, format!("{at}/{k}"), out)),
        Value::Array(items) => items.iter().enumerate().for_each(|(i, v)| string_paths(v, format!("{at}/{i}"), out)),
        _ => {}
    }
}

/// Every path to an object in `value` (JSON pointers; the root is "").
fn object_paths(value: &Value, at: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            out.push(at.clone());
            map.iter().for_each(|(k, v)| object_paths(v, format!("{at}/{k}"), out));
        }
        Value::Array(items) => items.iter().enumerate().for_each(|(i, v)| object_paths(v, format!("{at}/{i}"), out)),
        _ => {}
    }
}

/// Samples of the templates responses, each with unknown nested fields.
fn fuzz_samples() -> [Value; 3] {
    let future = json!({"kind": "signup", "created_at": "2026-11-01T00:00:00Z", "name": "n", "id": "i", "list": ["s", {"updated_at": "2026-11-01T00:00:00Z", "kind": "magic_link", "deep": [["x"]]}]});
    [
        json!({"templates": [{"kind": "signup", "overridden": true, "subject": "Welcome", "revision": 2, "updated_at": "2026-11-01T00:00:00Z", "future": future}], "note": "n"}),
        json!({"kind": "signup", "overridden": true, "subject": "Welcome", "intro": "Hi", "outro": "Bye", "revision": 2, "updated_at": "2026-11-01T00:00:00Z",
            "defaults": {"subject": "a", "intro": "b", "outro": "c", "kind": "signup"}, "new_field": future, "name": "j", "id": "k"}),
        json!({"subject": "Welcome", "text": "Hi", "html": "<p>Hi</p>", "kind": "signup", "meta": future}),
    ]
}

/// Each sample with `marker` put in every string, and added as a key (with a string
/// and an object value) to every object.
fn fuzz_probes(marker: &str) -> Vec<(String, Value)> {
    let mut probes = Vec::new();
    for sample in fuzz_samples() {
        let mut paths = Vec::new();
        string_paths(&sample, String::new(), &mut paths);
        for path in paths {
            let mut probe = sample.clone();
            *probe.pointer_mut(&path).unwrap() = marker.into();
            probes.push((format!("{path} = {marker:?}"), probe));
        }
        let mut objects = Vec::new();
        object_paths(&sample, String::new(), &mut objects);
        for path in objects {
            for value in [json!("v"), json!({marker: marker})] {
                let mut probe = sample.clone();
                probe.pointer_mut(&path).unwrap()[marker] = value;
                probes.push((format!("{path}/{marker:?} key"), probe));
            }
        }
    }
    probes
}

#[test]
fn a_marker_in_any_string_position_is_never_in_default_output_or_errors() {
    let markers = ["QzMarker0123456789Zq", "{{qzmarkerzq}}", "Hi {{qzmarkerzq}} QzMarker0123456789Zq", "{{ QzMarker }}", "{{qzmarkerzq", "see qzmarkerzq.example.com"];
    let mut checked = 0;
    for marker in markers {
        for (label, probe) in fuzz_probes(marker) {
            // The marker reaches the response (positive control).
            assert!(probe.to_string().to_lowercase().contains("qzmarker"), "{label}");
            for kind in [None, Some("signup")] {
                let mut hidden = probe.clone();
                crate::commands::auth::sanitize(&mut hidden, true);
                hide_text(&mut hidden, kind);
                let mut printed = vec![serde_json::to_string_pretty(&hidden).unwrap()];
                printed.extend(template_lines(&hidden, false));
                printed.extend(list_lines(&hidden, false));
                printed.extend(preview_lines(&hidden, false, false));
                printed.extend(preview_lines(&hidden, true, false));
                // The text a `set` or `preview` would send from this response is checked first.
                for kind in TEMPLATE_KINDS {
                    if let Err(error) = template_body(kind, &probe, None, None, None) {
                        printed.push(format!("{error:#}"));
                    }
                }
                let printed = printed.join("\n").to_lowercase();
                assert!(!printed.contains("qzmarker"), "{label}: {printed}");
                checked += 1;
            }
        }
    }
    assert!(checked > 500, "{checked}");
    // Every field of the text checks, with the marker in every form.
    for marker in markers {
        for (field, max) in [("the subject", 600), ("the intro", 2000), ("the outro", 2000)] {
            for value in [marker.to_string(), format!("{marker}\n{marker}"), format!("{marker}\u{7}"), marker.repeat(200)] {
                if let Some(problem) = template_problem("signup", field, &value, max) {
                    assert!(!problem.to_lowercase().contains("qzmarker"), "{field} {value:?}: {problem}");
                }
            }
        }
    }
}
