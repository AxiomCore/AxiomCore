//! Database access grants (runtime-data/v1): `axiom project resources grants`, `grant` and
//! `revoke`, the CLI side of the dashboard's **Database access** section.
//!
//! The requests and bodies are the dashboard's (`project-resources.js`): the grant list and
//! writes go to `…/resources/{id}/database-grants`, workloads come from the resource
//! environment's `secret-workloads`. Grants are Owner-only metadata; no credential is read or
//! printed. Validation (requirement name, an enabled database-access workload of the resource's
//! environment with a plan, an expiry of 7, 30 or 90 days) runs before any write.
//!
//! Errors: these routes answer `{"error":{"code"}}` with one fixed status per code
//! (`migrationErrorStatus` in the backend). The shared resource transport ([`Api`]) reports
//! only the HTTP status for that shape, so the code is recovered from the status (or taken
//! from the transport's message when it already names a known code) and mapped to the
//! dashboard's `grantCodes`/`grantActions` messages. Server text is never echoed.
//!
//! Idempotency: every write sends an `Idempotency-Key` (`--operation-id`, or a new UUID that is
//! printed). The backend stores a digest of the key's request (requirement, workload, plan,
//! consumer and `expires_at` to the microsecond) and returns the stored grant when the same key
//! comes with the same body; the same key with a different body is `AXRESOURCE_CONFLICT`. As
//! `expires_at` is computed from the clock, a retry must also pass the first attempt's
//! `--expires-at`; an unconfirmed grant prints both values. A second grant cannot arise in any
//! case: a workload holds at most one active grant per requirement on a database.
use super::resources_placement::{resolve, Select};
use super::resources_recovery::Api;
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Method;
use serde_json::{json, Value};

/// The data gateway's lease audience: only workloads registered for it can hold a grant.
pub const DATABASE_AUDIENCE: &str = "https://api.axiomcore.dev/runtime/v1/database-leases";
/// The expiry presets of the dashboard.
pub const EXPIRY_DAYS: [u32; 3] = [7, 30, 90];
const DAY_MS: i64 = 86_400_000;
/// The 90-day preset stays five minutes inside the API's limit so a clock slightly ahead of
/// the server is not refused.
const MARGIN_MS: i64 = 300_000;

const REQUIREMENT_HELP: &str = "Enter the requirement name: a lowercase letter, then up to 63 letters or digits.";
const WORKLOAD_HELP: &str = "Choose an enabled database-access workload in this environment.";
const EXPIRY_HELP: &str = "Choose an expiry of 7, 30 or 90 days.";
const WORKLOADS_FAILED: &str = "Workloads for this environment could not be loaded. Refresh and try again.";
const SIGN_IN: &str = "Sign in again to continue. Run `axiom login`.";

/// Terminal-safe text from a server value: no control characters, bounded.
fn clean(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(128).collect()
}
fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A `connectDatabase` requirement name: `^[a-z][A-Za-z0-9]{0,63}$`.
pub fn valid_requirement(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty() && bytes.len() <= 64 && bytes[0].is_ascii_lowercase() && bytes.iter().all(u8::is_ascii_alphanumeric)
}

fn is_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The dashboard's `grantExpiry`: `days` from `now`, at most 90 days less five minutes.
pub fn grant_expiry(days: u32, now_ms: i64) -> i64 {
    (now_ms + i64::from(days) * DAY_MS).min(now_ms + 90 * DAY_MS - MARGIN_MS)
}

/// Milliseconds since the epoch as the dashboard's `toISOString()`.
pub fn iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The `expires_at` to send: an explicit `--expires-at` (a retry) or `--expires-days` from now.
/// Both keep the API's rule: more than a minute from now and at most 90 days ahead.
pub fn expires_at(days: u32, explicit: Option<&str>, now: DateTime<Utc>) -> Result<String> {
    let now_ms = now.timestamp_millis();
    let Some(explicit) = explicit else {
        if !EXPIRY_DAYS.contains(&days) {
            bail!("{EXPIRY_HELP}");
        }
        return Ok(iso(grant_expiry(days, now_ms)));
    };
    let ms = DateTime::parse_from_rfc3339(explicit.trim())
        .map(|t| t.timestamp_millis())
        .map_err(|_| anyhow!("--expires-at must be an RFC 3339 time, for example 2026-12-01T09:30:00.000Z"))?;
    if ms <= now_ms + 60_000 || ms > now_ms + 90 * DAY_MS {
        bail!("--expires-at must be more than a minute from now and at most 90 days ahead. {EXPIRY_HELP}");
    }
    Ok(iso(ms))
}

/// `Active`, `Expired` or `Revoked`, as the dashboard's `grantState`.
pub fn grant_state(grant: &Value, now: DateTime<Utc>) -> &'static str {
    if grant.get("revoked_at").is_some_and(|v| !v.is_null()) {
        return "Revoked";
    }
    match DateTime::parse_from_rfc3339(field(grant, "expires_at")) {
        Ok(expires) if expires > now => "Active",
        _ => "Expired",
    }
}

/// Enabled, registered for the database lease audience, in the resource's environment, with a
/// plan: the dashboard's `eligibleWorkloads`.
pub fn eligible(workload: &Value, environment_id: &str) -> bool {
    let environment = field(workload, "environment_id");
    field(workload, "status") == "active"
        && field(workload, "audience") == DATABASE_AUDIENCE
        && (environment.is_empty() || environment == environment_id)
        && is_digest(field(workload, "plan_sha256"))
}

fn ineligible_reason(workload: &Value, environment_id: &str) -> &'static str {
    let environment = field(workload, "environment_id");
    if field(workload, "status") != "active" {
        "is disabled"
    } else if field(workload, "audience") != DATABASE_AUDIENCE {
        "is not registered for database access"
    } else if !environment.is_empty() && environment != environment_id {
        "belongs to another environment"
    } else {
        "has no compiled backend plan"
    }
}

fn label(workload: &Value) -> String {
    format!("{} ({})", clean(field(workload, "subject")), clean(field(workload, "id")))
}

/// The eligible workload `wanted` names, by ID or by Google workload subject.
pub fn find_workload<'a>(items: &'a [Value], environment_id: &str, environment: &str, wanted: &str) -> Result<&'a Value> {
    let wanted = wanted.trim();
    let named = |w: &&Value| field(w, "id").eq_ignore_ascii_case(wanted) || field(w, "subject") == wanted;
    let matches: Vec<&Value> = items.iter().filter(named).filter(|w| eligible(w, environment_id)).collect();
    match matches.as_slice() {
        [one] => return Ok(one),
        [_, _, ..] => bail!(
            "Several enabled workloads have the subject {}: {}. Pass the workload ID.",
            clean(wanted),
            matches.iter().map(|w| clean(field(w, "id"))).collect::<Vec<_>>().join(", ")
        ),
        [] => {}
    }
    let mut message = WORKLOAD_HELP.to_string();
    if let Some(found) = items.iter().find(named) {
        message.push_str(&format!(" Workload {} {}.", label(found), ineligible_reason(found, environment_id)));
    } else {
        message.push_str(&format!(" No workload {} is registered in {}.", clean(wanted), clean(environment)));
    }
    let choices: Vec<String> = items.iter().filter(|w| eligible(w, environment_id)).map(label).collect();
    if choices.is_empty() {
        message.push_str(&format!(" No enabled workload in {} is registered for database access.", clean(environment)));
    } else {
        message.push_str(&format!(" Enabled workloads in {} registered for database access: {}.", clean(environment), choices.join(", ")));
    }
    bail!(message)
}

/// The revocation bound of the resource's engine (the dashboard's `revokeBound`).
pub fn revoke_bound(resource: &Value) -> &'static str {
    let engine = [field(resource, "database"), field(resource, "engine")].into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    match engine {
        "postgres" | "postgresql" => "The workload can't get new credentials. Its open connections end within about 10 minutes.",
        "sqlite" | "libsql" => "The workload can't get new credentials. A credential it already holds keeps working until it expires, at most 15 minutes.",
        _ => "The workload can't get new credentials.",
    }
}

fn show_time(value: &str) -> String {
    match DateTime::parse_from_rfc3339(value) {
        Ok(time) => time.with_timezone(&Utc).format("%Y-%m-%d %H:%M UTC").to_string(),
        Err(_) if value.is_empty() => "-".into(),
        Err(_) => clean(value),
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Load,
    Create,
    Revoke,
}

/// The dashboard's `grantMessage(action)(code, status)`, and whether the code has a fixed message.
pub fn grant_message(step: Step, code: &str, status: Option<u16>) -> (String, bool) {
    if status == Some(401) && code.is_empty() {
        return (SIGN_IN.into(), false);
    }
    let local = match (step, code) {
        (Step::Load, "AXRESOURCE_DENIED") => Some("Your role does not allow you to view database access for this database."),
        (Step::Revoke, "AXRESOURCE_INVALID") => Some("The revocation was not accepted. Refresh and try again."),
        (Step::Revoke, "AXRESOURCE_CONFLICT") => Some("The grant changed. Refresh and review it."),
        _ => None,
    };
    let shared = match code {
        "AXRESOURCE_DENIED" => Some("Only project owners can manage database access, and only for databases in this project."),
        "AXRESOURCE_INVALID" => Some("The grant was not accepted. Use a valid requirement name, an enabled database-access workload in this environment and an expiry within 90 days."),
        "AXRESOURCE_CONFLICT" => Some("This workload already has an active grant for this requirement, or the request changed. Refresh and review the grants."),
        "AXRESOURCE_NOT_FOUND" => Some("The database or grant was not found. Refresh and try again."),
        "AXRESOURCE_NOT_ADMITTED" => Some("Database access is not available for this database yet."),
        "AXRESOURCE_CAPACITY" => Some("This database has reached its limit of active grants."),
        "AXRESOURCE_UNAVAILABLE" => Some("Database access is unavailable right now. Try again shortly."),
        _ => None,
    };
    if let Some(message) = local.or(shared) {
        return (message.into(), true);
    }
    let fallback = match step {
        Step::Load => "Database access could not be loaded. Refresh and try again.",
        Step::Create => "The grant could not be confirmed. Try again: a retry with the same details does not create a second grant.",
        Step::Revoke => "The revocation could not be confirmed. Try again.",
    };
    if code.is_empty() {
        (fallback.into(), false)
    } else {
        (format!("{fallback} Code: {code}."), false)
    }
}

fn code_shaped(value: &str) -> bool {
    (2..=64).contains(&value.len()) && value.as_bytes()[0].is_ascii_uppercase() && value.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// The code and HTTP status of a resource transport error. `None` when the error is not a
/// transport diagnostic (an expired session, an oversized response), which stays as it is.
pub fn failure(error: &anyhow::Error) -> Option<(String, Option<u16>)> {
    let text = error.to_string();
    let (head, _) = text.split_once(':')?;
    if !code_shaped(head) {
        return None;
    }
    let status = text.rfind("(HTTP ").and_then(|at| text[at + 6..].split(')').next()).and_then(|s| s.parse::<u16>().ok());
    let code = match (head, status) {
        // The fixed status of each code on these routes (backend `migrationErrorStatus`).
        ("AXRESOURCE_CLOUD", Some(status)) => match status {
            400 => "AXRESOURCE_INVALID",
            402 => "AXRESOURCE_PAYMENT_REQUIRED",
            403 => "AXRESOURCE_DENIED",
            404 => "AXRESOURCE_NOT_FOUND",
            409 => "AXRESOURCE_CONFLICT",
            429 => "AXRESOURCE_CAPACITY",
            503 => "AXRESOURCE_UNAVAILABLE",
            _ => "",
        },
        ("AXRESOURCE_RESPONSE_TOO_LARGE", _) => return None,
        // The request failed or its answer was lost: the outcome is unknown.
        ("AXRESOURCE_UNAVAILABLE", None) => "",
        (code, _) => code,
    };
    Some((code.to_string(), status))
}

fn grant_error(step: Step, error: anyhow::Error) -> anyhow::Error {
    let Some((code, status)) = failure(&error) else { return error };
    let (message, known) = grant_message(step, &code, status);
    let http = status.map(|s| format!(" (HTTP {s})")).unwrap_or_default();
    if known {
        anyhow!("{code}: {message}{http}")
    } else {
        anyhow!("{message}{http}")
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

fn resource_id(resource: &Value) -> Result<String> {
    resource.get("id").and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("AXRESOURCE_INVALID_RESPONSE: resource without id"))
}

/// The resource of `select`, refused when it is deleting or deleted (it holds no grants).
async fn database(api: &Api, project: &str, select: &Select) -> Result<(Value, String)> {
    let resource = resolve(api, project, select).await?;
    if ["deleting", "deleted"].contains(&field(&resource, "status")) {
        bail!("The {} database is being deleted or was deleted, so it holds no database access.", clean(field(&resource, "environment")));
    }
    let id = resource_id(&resource)?;
    Ok((resource, id))
}

/// The workloads of the resource's environment (`secret-workloads`).
async fn workloads(api: &Api, project: &str, resource: &Value) -> Result<Vec<Value>> {
    let environment = field(resource, "environment_id");
    if environment.is_empty() {
        bail!("{WORKLOADS_FAILED}");
    }
    let value = api.get(&["projects", project, "configuration-environments", environment, "secret-workloads"]).await.map_err(|error| match failure(&error) {
        Some((code, Some(401))) if code == "AXRESOURCE_CLOUD" || code.is_empty() => anyhow!(SIGN_IN),
        Some(_) => anyhow!(WORKLOADS_FAILED),
        None => error,
    })?;
    Ok(match value {
        Value::Array(items) => items,
        Value::Object(mut map) => match map.remove("workloads") {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    })
}

fn operation_key(operation_id: Option<&str>) -> Result<String> {
    let key = operation_id.map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&key).map_err(|_| anyhow!("--operation-id must be a UUID"))?;
    Ok(key)
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// The grant list as a table: requirement, workload subject (or ID), state, expiry and ID.
pub fn render_grants(grants: &[Value], items: &[Value], now: DateTime<Utc>) -> String {
    let mut rows = vec![["REQUIREMENT".to_string(), "WORKLOAD".into(), "STATE".into(), "EXPIRES".into(), "ID".into()]];
    for grant in grants {
        let workload_id = field(grant, "workload_id");
        let workload = items.iter().find(|w| !workload_id.is_empty() && field(w, "id") == workload_id);
        let name = match workload {
            Some(w) if field(w, "status") == "active" => clean(field(w, "subject")),
            Some(w) => format!("{} (disabled)", clean(field(w, "subject"))),
            None if workload_id.is_empty() => "Workload not recorded".into(),
            None => clean(workload_id),
        };
        rows.push([clean(field(grant, "requirement")), name, grant_state(grant, now).into(), show_time(field(grant, "expires_at")), clean(field(grant, "id"))]);
    }
    let widths: Vec<usize> = (0..5).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0)).collect();
    rows.iter()
        .map(|row| row.iter().enumerate().map(|(i, cell)| if i == 4 { cell.clone() } else { format!("{cell:<width$}", width = widths[i]) }).collect::<Vec<_>>().join("  "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `grants`: the resource's grants, newest first, labelled with the workload subjects.
pub async fn handle_grants(project: &str, select: &Select, json: bool) -> Result<()> {
    let api = Api::from_profile()?;
    let (resource, id) = database(&api, project, select).await?;
    let mut value = api.get(&["projects", project, "resources", &id, "database-grants"]).await.map_err(|e| grant_error(Step::Load, e))?;
    // Labels only: without the workload list a grant shows its workload ID.
    let items = match workloads(&api, project, &resource).await {
        Ok(items) => items,
        Err(error) => {
            eprintln!("{error} Workloads are shown by ID.");
            Vec::new()
        }
    };
    let now = Utc::now();
    let mut grants: Vec<Value> = value.get("grants").and_then(Value::as_array).cloned().unwrap_or_default();
    if json {
        for grant in &mut grants {
            let subject = items.iter().find(|w| field(w, "id") == field(grant, "workload_id")).map(|w| json!(field(w, "subject"))).unwrap_or(Value::Null);
            let state = grant_state(grant, now).to_ascii_lowercase();
            if let Some(map) = grant.as_object_mut() {
                map.insert("state".into(), json!(state));
                map.insert("workload_subject".into(), subject);
            }
        }
        value["grants"] = Value::Array(grants);
        return print_json(&value);
    }
    if grants.is_empty() {
        println!(
            "No database access granted in {}. A server workload needs a grant before it can get credentials for this database. \
             Register the workload's Google identity for database access on the Secrets page, then grant it access with \
             `axiom project resources --project {project} grant`.",
            clean(field(&resource, "environment"))
        );
        return Ok(());
    }
    println!("{}", render_grants(&grants, &items, now));
    Ok(())
}

/// Flags of `grant`.
#[derive(Clone, Debug, Default)]
pub struct GrantInput<'a> {
    pub requirement: &'a str,
    pub workload: &'a str,
    pub expires_days: u32,
    pub expires_at: Option<&'a str>,
}

/// `grant`: validated like the dashboard's `submitGrant` before the single write.
pub async fn handle_grant(project: &str, select: &Select, input: &GrantInput<'_>, operation_id: Option<&str>, json: bool) -> Result<()> {
    let requirement = input.requirement.trim();
    if !valid_requirement(requirement) {
        bail!("{REQUIREMENT_HELP}");
    }
    let expires_at = expires_at(input.expires_days, input.expires_at, Utc::now())?;
    let key = operation_key(operation_id)?;
    let api = Api::from_profile()?;
    let (resource, id) = database(&api, project, select).await?;
    let environment = field(&resource, "environment").to_string();
    let items = workloads(&api, project, &resource).await?;
    let workload = find_workload(&items, field(&resource, "environment_id"), &environment, input.workload)?;
    let subject = clean(field(workload, "subject"));
    let body = json!({
        "requirement": requirement,
        "purpose": "app",
        "workload_id": field(workload, "id"),
        "plan_sha256": field(workload, "plan_sha256"),
        "consumer": "server",
        "expires_at": expires_at,
    });
    eprintln!("Operation idempotency key: {key}");
    let value = match api.write(&Method::POST, &["projects", project, "resources", &id, "database-grants"], &body, Some(&key)).await {
        Ok(value) => value,
        Err(error) => {
            let unknown = failure(&error).is_some_and(|(code, status)| code.is_empty() || status.is_none_or(|s| s >= 500));
            let error = grant_error(Step::Create, error);
            if unknown {
                bail!("{error}\nTo retry without creating a second grant, run the command again with --operation-id {key} --expires-at {expires_at}.");
            }
            return Err(error);
        }
    };
    if json {
        return print_json(&value);
    }
    println!(
        "Database access granted to {subject} for {requirement} in {} until {}.",
        clean(&environment),
        show_time(&expires_at)
    );
    Ok(())
}

/// `revoke`: revoking a revoked grant returns it unchanged.
pub async fn handle_revoke(project: &str, select: &Select, grant: &str, operation_id: Option<&str>, json: bool) -> Result<()> {
    uuid::Uuid::parse_str(grant).map_err(|_| anyhow!("The grant ID must be a UUID; `axiom project resources --project {project} grants` lists them."))?;
    let key = operation_key(operation_id)?;
    let api = Api::from_profile()?;
    let (resource, id) = database(&api, project, select).await?;
    eprintln!("Operation idempotency key: {key}");
    let value = api
        .write(&Method::POST, &["projects", project, "resources", &id, "database-grants", grant, "revoke"], &json!({}), Some(&key))
        .await
        .map_err(|e| grant_error(Step::Revoke, e))?;
    if json {
        return print_json(&value);
    }
    let requirement = value.pointer("/grant/requirement").and_then(Value::as_str).filter(|r| !r.is_empty()).unwrap_or(grant);
    println!("Grant for {} revoked in {}. {}", clean(requirement), clean(field(&resource, "environment")), revoke_bound(&resource));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn workload(id: &str, subject: &str, status: &str, audience: &str) -> Value {
        json!({"id": id, "subject": subject, "status": status, "audience": audience, "plan_sha256": PLAN})
    }

    #[test]
    fn requirement_names_follow_the_dashboard_pattern() {
        for good in ["tasks", "t", "orderItems2", &format!("a{}", "b".repeat(63))] {
            assert!(valid_requirement(good), "{good}");
        }
        for bad in ["", "Tasks", "2tasks", "task-s", "task s", "tâche", &format!("a{}", "b".repeat(64))] {
            assert!(!valid_requirement(bad), "{bad}");
        }
    }

    #[test]
    fn expiry_matches_grant_expiry_and_the_api_window() {
        let now = DateTime::parse_from_rfc3339("2026-11-13T10:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(expires_at(7, None, now).unwrap(), "2026-11-20T10:00:00.000Z");
        assert_eq!(expires_at(30, None, now).unwrap(), "2026-12-13T10:00:00.000Z");
        assert_eq!(expires_at(90, None, now).unwrap(), "2027-02-11T09:55:00.000Z");
        assert!(expires_at(14, None, now).unwrap_err().to_string().contains(EXPIRY_HELP));
        assert_eq!(expires_at(14, Some("2026-12-01T12:00:00+02:00"), now).unwrap(), "2026-12-01T10:00:00.000Z");
        assert!(expires_at(30, Some("2026-11-13T10:00:30Z"), now).is_err());
        assert!(expires_at(30, Some("2027-02-11T10:00:01Z"), now).is_err());
        assert!(expires_at(30, Some("2027-02-11T10:00:00Z"), now).is_ok());
        assert!(expires_at(30, Some("tomorrow"), now).is_err());
    }

    #[test]
    fn states_and_eligibility_follow_the_dashboard() {
        let now = Utc::now();
        assert_eq!(grant_state(&json!({"expires_at": "2099-01-01T00:00:00Z", "revoked_at": null}), now), "Active");
        assert_eq!(grant_state(&json!({"expires_at": "2020-01-01T00:00:00Z"}), now), "Expired");
        assert_eq!(grant_state(&json!({"expires_at": "nonsense"}), now), "Expired");
        assert_eq!(grant_state(&json!({"expires_at": "2099-01-01T00:00:00Z", "revoked_at": "2026-01-01T00:00:00Z"}), now), "Revoked");
        let env = "e1";
        assert!(eligible(&workload("a", "s", "active", DATABASE_AUDIENCE), env));
        assert!(!eligible(&workload("a", "s", "disabled", DATABASE_AUDIENCE), env));
        assert!(!eligible(&workload("a", "s", "active", "https://example.test/secrets"), env));
        let mut other = workload("a", "s", "active", DATABASE_AUDIENCE);
        other["environment_id"] = json!("e2");
        assert!(!eligible(&other, env));
        let mut unplanned = workload("a", "s", "active", DATABASE_AUDIENCE);
        unplanned["plan_sha256"] = json!("ABC");
        assert!(!eligible(&unplanned, env));
    }

    #[test]
    fn workloads_are_found_by_id_or_subject() {
        let items = vec![
            workload("11111111-2222-4333-8444-555555555555", "sub-1", "active", DATABASE_AUDIENCE),
            workload("22222222-2222-4333-8444-555555555555", "sub-2", "disabled", DATABASE_AUDIENCE),
            workload("33333333-2222-4333-8444-555555555555", "dup", "active", DATABASE_AUDIENCE),
            workload("44444444-2222-4333-8444-555555555555", "dup", "active", DATABASE_AUDIENCE),
        ];
        assert_eq!(field(find_workload(&items, "e", "production", "sub-1").unwrap(), "id"), "11111111-2222-4333-8444-555555555555");
        assert_eq!(field(find_workload(&items, "e", "production", "11111111-2222-4333-8444-555555555555").unwrap(), "subject"), "sub-1");
        let disabled = find_workload(&items, "e", "production", "sub-2").unwrap_err().to_string();
        assert!(disabled.starts_with(WORKLOAD_HELP) && disabled.contains("is disabled") && disabled.contains("sub-1 (11111111"), "{disabled}");
        assert!(find_workload(&items, "e", "production", "dup").unwrap_err().to_string().contains("Pass the workload ID"));
        let none = find_workload(&[], "e", "production", "x").unwrap_err().to_string();
        assert!(none.contains("No enabled workload in production is registered for database access."), "{none}");
    }

    #[test]
    fn transport_errors_map_to_the_dashboard_messages() {
        let map = |step, text: &str| grant_error(step, anyhow!(text.to_string())).to_string();
        let cloud = |status| format!("AXRESOURCE_CLOUD: request denied or unavailable (HTTP {status}). Check access, admission, capacity and subscription status.");
        assert_eq!(map(Step::Create, &cloud(409)), "AXRESOURCE_CONFLICT: This workload already has an active grant for this requirement, or the request changed. Refresh and review the grants. (HTTP 409)");
        assert_eq!(map(Step::Revoke, &cloud(409)), "AXRESOURCE_CONFLICT: The grant changed. Refresh and review it. (HTTP 409)");
        assert_eq!(map(Step::Load, &cloud(403)), "AXRESOURCE_DENIED: Your role does not allow you to view database access for this database. (HTTP 403)");
        assert!(map(Step::Create, &cloud(403)).contains("Only project owners can manage database access"));
        assert!(map(Step::Create, &cloud(400)).contains("The grant was not accepted."));
        assert!(map(Step::Revoke, &cloud(400)).contains("The revocation was not accepted."));
        assert!(map(Step::Create, &cloud(429)).contains("limit of active grants"));
        assert_eq!(map(Step::Create, &cloud(402)), "The grant could not be confirmed. Try again: a retry with the same details does not create a second grant. Code: AXRESOURCE_PAYMENT_REQUIRED. (HTTP 402)");
        assert_eq!(map(Step::Load, &cloud(401)), format!("{SIGN_IN} (HTTP 401)"));
        assert_eq!(map(Step::Revoke, &cloud(500)), "The revocation could not be confirmed. Try again. (HTTP 500)");
        assert_eq!(
            map(Step::Create, "AXRESOURCE_UNAVAILABLE: request failed. Check your connection and retry."),
            "The grant could not be confirmed. Try again: a retry with the same details does not create a second grant."
        );
        assert!(map(Step::Create, "AXRESOURCE_NOT_ADMITTED: This database action is not available for the selected resource yet. (HTTP 409)").starts_with("AXRESOURCE_NOT_ADMITTED: Database access is not available"));
        let session = "Your AxiomCore session has expired. Run 'axiom login' and retry.";
        assert_eq!(map(Step::Load, session), session);
    }

    #[test]
    fn revoke_bounds_follow_the_engine() {
        assert!(revoke_bound(&json!({"database": "postgres"})).contains("10 minutes"));
        assert!(revoke_bound(&json!({"engine": "libsql"})).contains("15 minutes"));
        assert_eq!(revoke_bound(&json!({})), "The workload can't get new credentials.");
    }

    #[test]
    fn the_table_never_prints_control_characters() {
        let grants = vec![json!({"id": "g", "requirement": "ta\u{1b}[31msks", "workload_id": "w", "expires_at": "2099-01-01T00:00:00Z"})];
        let table = render_grants(&grants, &[workload("w", "sub\u{7}", "active", DATABASE_AUDIENCE)], Utc::now());
        assert!(!table.chars().any(|c| c.is_control() && c != '\n'), "{table:?}");
        assert!(table.contains("ta[31msks") && table.contains("sub ") && table.contains("2099-01-01 00:00 UTC"), "{table}");
    }
}
