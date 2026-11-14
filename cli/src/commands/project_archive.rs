//! `axiom project archive`, `axiom project restore`, `axiom project archive-status` and
//! `axiom project list --archived` (V7 project archive, `/api/v1/projects/{slug}/archive`).
//!
//! Archive is destructive: it needs the typed confirmation `archive <project-slug>` on a
//! terminal, or `--yes` (which sends that same phrase). Without a terminal and without
//! `--yes`, `main` refuses before sign-in, the project lookup or any request (exit 2). The
//! archive POST follows the step-up rule of the per-database archive
//! ([`Api::post_action_reply`]); restore is not destructive and uses the session only.
//! Only known codes select fixed messages; server text and unknown values are cleaned.
use super::cloud_cli::{self, clean, items, pick};
use super::resources_recovery::{Api, Reply};
use anyhow::{anyhow, bail, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::time::Duration;

/// `--wait` polls every 3 s for at most an hour: an archive snapshots and verifies every
/// database before it deletes it, and a restore rebuilds each one from its snapshot.
#[derive(Clone, Copy, Debug)]
pub struct Poll {
    pub interval: Duration,
    pub limit: Duration,
}

impl Default for Poll {
    fn default() -> Self {
        Self { interval: Duration::from_secs(3), limit: Duration::from_secs(60 * 60) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Archive,
    Restore,
    Status,
}

fn exit2(message: impl Into<String>) -> anyhow::Error {
    super::auth::Exit { code: 2, message: message.into() }.into()
}

/// The typed confirmation, behind a trait so tests can answer it.
pub trait Prompt {
    fn is_terminal(&self) -> bool;
    fn input(&mut self, prompt: &str) -> Result<String>;
}

pub struct TerminalPrompt;

impl Prompt for TerminalPrompt {
    fn is_terminal(&self) -> bool {
        std::io::stdin().is_terminal()
    }
    fn input(&mut self, prompt: &str) -> Result<String> {
        Ok(dialoguer::Input::<String>::new().with_prompt(prompt).allow_empty(true).interact_text()?)
    }
}

/// The exact phrase the archive request carries.
pub fn confirm_phrase(slug: &str) -> String {
    format!("archive {slug}")
}

// ---------------------------------------------------------------------------
// Project resolution
// ---------------------------------------------------------------------------

/// The project slug from the argument (a slug, name or id) or the project linked to this
/// directory. An archived project is hidden from `GET /api/v1/projects`, so a value that
/// matches none of the listed projects is looked up again among the archived ones.
pub async fn project_slug(api: &Api, given: Option<&str>) -> Result<String> {
    let wanted = match given.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => crate::auth_store::get_project_id(&std::env::current_dir()?)?.ok_or_else(|| {
            anyhow!("no project: pass the project slug, name or id, or link this directory with `axiom project link`")
        })?,
    };
    let listed = cloud_cli::get(api, &["projects"]).await?;
    let mut list = items(&listed, &["projects", "items"]);
    if let Ok(found) = pick(&list, &wanted, &["slug", "name"], "project") {
        return slug_of(found, &wanted);
    }
    let archived = cloud_cli::call(api, Method::GET, &["projects"], &[("archived", "true".into())], None).await?;
    for item in items(&archived, &["projects", "items"]) {
        let id = item.get("id").and_then(Value::as_str);
        if !list.iter().any(|known| id.is_some() && known.get("id").and_then(Value::as_str) == id) {
            list.push(item);
        }
    }
    slug_of(pick(&list, &wanted, &["slug", "name"], "project")?, &wanted)
}

fn slug_of(project: &Value, wanted: &str) -> Result<String> {
    project
        .get("slug")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("the project list has no slug for `{}`", clean(wanted)))
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn state(view: &Value) -> &str {
    field(view, "state").unwrap_or("none")
}

fn is_terminal_state(state: &str) -> bool {
    matches!(state, "archived" | "restored" | "failed")
}

fn state_words(state: &str) -> String {
    match state {
        "none" => "not archived".into(),
        "archiving" => "archiving: each database is being snapshotted and verified, then deleted from its provider".into(),
        "archived" => "archived: every database is kept as a verified snapshot and is not running".into(),
        "restoring" => "restoring: the databases are coming back from their snapshots".into(),
        "restored" => "restored: the databases serve their archived data again".into(),
        "failed" => "failed: at least one database could not be archived; it was not deleted and still serves".into(),
        other => clean(other),
    }
}

/// Why a database blocks an archive, with what to do about it.
pub fn blocker_reason(reason: &str, slug: &str) -> String {
    match reason {
        "byo" => "a database connected from your own provider account (bring your own); project archive covers only databases AxiomCore hosts. Remove it from the project first.".into(),
        "branch" => "a branch database. Delete the branch first (`axiom branch delete`).".into(),
        "busy" => format!("another operation is running on it. Wait for it to finish (`axiom project resources --project {slug}`), then retry."),
        "state" => format!("its status does not allow an archive now (for example it is suspended, archived on its own or needs repair). Check `axiom project resources --project {slug}`."),
        "unqualified" => "not yet qualified for project archive (it is enabled database by database). Contact support.".into(),
        other => format!("not archivable now ({}).", clean(other)),
    }
}

fn blocker_lines(blockers: &[Value], slug: &str) -> Vec<String> {
    blockers
        .iter()
        .map(|b| {
            let name = field(b, "database_name").or_else(|| field(b, "resource_id")).map(clean).unwrap_or_else(|| "a database".into());
            format!("  - {name}: {}", blocker_reason(field(b, "reason").unwrap_or(""), slug))
        })
        .collect()
}

fn blockers(view: &Value) -> Vec<Value> {
    view.get("blockers").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn blocked_message(slug: &str, blockers: &[Value]) -> String {
    let mut lines = vec![format!("`{slug}` cannot be archived now. Nothing was changed. Blocked by:")];
    if blockers.is_empty() {
        lines.push(format!("  (no details; see `axiom project archive-status {slug}`)"));
    }
    lines.extend(blocker_lines(blockers, slug));
    lines.join("\n")
}

fn reply_code(reply: &Reply) -> Option<&str> {
    reply.body.pointer("/error/code").or_else(|| reply.body.get("code")).and_then(Value::as_str)
}

/// Plain words for a refused archive, restore or status read.
fn refusal(kind: Kind, slug: &str, reply: &Reply) -> anyhow::Error {
    let Some(code) = reply_code(reply) else {
        return cloud_cli::error(reply);
    };
    let message = match code {
        "AXPROJECT_ARCHIVE_DISABLED" => format!("Project archive is not enabled for `{slug}`. Nothing was changed."),
        "AXPROJECT_ARCHIVE_BLOCKED" => blocked_message(slug, &blockers(&reply.body)),
        "AXPROJECT_NOT_ARCHIVED" => format!("`{slug}` has not been archived, so there is nothing to restore."),
        "AXRESOURCE_CONFLICT" => match kind {
            Kind::Restore => format!("An archive of `{slug}` is still running. Wait until it is archived or failed (`axiom project archive-status {slug}`), then retry the restore."),
            _ => format!("A restore of `{slug}` is running, or this request's Idempotency-Key was already used for a different request. Check `axiom project archive-status {slug}` and retry."),
        },
        "AXRESOURCE_DENIED" => match kind {
            Kind::Status => format!("Your role in `{slug}` does not allow reading its archive status."),
            _ => format!("Only a project Owner can archive or restore `{slug}`."),
        },
        "AXRESOURCE_REAUTH_REQUIRED" => "This action needs a recent sign-in. Run `axiom login --step-up` and retry.".into(),
        "AXRESOURCE_INVALID" => match kind {
            Kind::Archive => "The request was refused: the confirmation text, Idempotency-Key or body was not accepted. Nothing was changed.".into(),
            _ => "The request was refused as invalid. Nothing was changed.".into(),
        },
        "AXRESOURCE_UNAVAILABLE" => "The service is temporarily unavailable. Retry in a minute.".into(),
        _ => return cloud_cli::error(reply),
    };
    anyhow!("{code}: {message} (HTTP {})", reply.status)
}

/// Human rendering of `GET …/archive`. Every printed value is cleaned.
pub fn render_view(slug: &str, view: &Value) -> Vec<String> {
    if view.get("enabled").and_then(Value::as_bool) == Some(false) {
        return vec![format!("Project archive is not enabled for `{slug}`.")];
    }
    let state = state(view);
    let mut out = vec![format!("Project {slug}: {}", state_words(state))];
    let mut when = Vec::new();
    if let Some(at) = field(view, "requested_at") {
        when.push(format!("requested {}", clean(at)));
    }
    if let Some(at) = field(view, "completed_at") {
        when.push(format!("completed {}", clean(at)));
    }
    if !when.is_empty() {
        out.push(format!("  {}", when.join(", ")));
    }
    if view.get("hidden").and_then(Value::as_bool) == Some(true) {
        out.push(format!("  Hidden from `axiom project list`; see `axiom project list --archived`. Restore it with `axiom project restore {slug}`."));
    }
    let databases = view.get("databases").and_then(Value::as_array).cloned().unwrap_or_default();
    if !databases.is_empty() {
        out.push("Databases:".into());
        for db in &databases {
            let name = field(db, "database_name").map(clean).unwrap_or_else(|| "?".into());
            let status = field(db, "status").map(clean).unwrap_or_else(|| "?".into());
            let mut line = format!("  {name}  {status}");
            if let Some(op) = field(db, "operation_status") {
                line.push_str(&format!("  (operation {}", clean(op)));
                if let Some(code) = field(db, "failure_code") {
                    line.push_str(&format!(": {}", clean(code)));
                }
                line.push(')');
            }
            out.push(line);
        }
    }
    let blockers = blockers(view);
    if !blockers.is_empty() && !matches!(state, "archiving" | "archived") {
        out.push("An archive request now would be refused:".into());
        out.extend(blocker_lines(&blockers, slug));
    }
    out
}

fn consequences(slug: &str, view: &Value) -> Vec<String> {
    let mut out = vec![
        format!("Archiving `{slug}` hides it from project lists at once. Each database gets an encrypted, verified snapshot; only after its snapshot verifies is the database deleted from its hosting provider."),
        format!("Apps cannot use the project's databases until an Owner restores it with `axiom project restore {slug}`, which brings them back from their snapshots with their data."),
    ];
    let names: Vec<String> = view
        .get("databases")
        .and_then(Value::as_array)
        .map(|dbs| dbs.iter().filter_map(|db| field(db, "database_name")).map(clean).collect())
        .unwrap_or_default();
    if !names.is_empty() {
        out.push(format!("Databases: {}", names.join(", ")));
    }
    out
}

fn accepted_text(kind: Kind, slug: &str, view: &Value) -> Vec<String> {
    match (kind, state(view)) {
        (Kind::Archive, "archived") => vec![format!("Project {slug} is archived. Restore it with `axiom project restore {slug}`.")],
        (Kind::Archive, "archiving") => vec![format!(
            "Archive of {slug} started: the project is hidden from project lists and each database is being snapshotted. Check progress with `axiom project archive-status {slug}`, or add --wait."
        )],
        (Kind::Restore, "restored") => vec![format!("Project {slug} is restored: it is listed again and its databases serve their archived data.")],
        (Kind::Restore, "restoring") => vec![format!(
            "Restore of {slug} started: the project is listed again and its databases are coming back from their snapshots. Check progress with `axiom project archive-status {slug}`, or add --wait."
        )],
        _ => render_view(slug, view),
    }
}

fn failed_error(kind: Kind, slug: &str, view: &Value) -> anyhow::Error {
    let failed: Vec<String> = view
        .get("databases")
        .and_then(Value::as_array)
        .map(|dbs| {
            dbs.iter()
                .filter(|db| field(db, "operation_status") == Some("failed") || field(db, "failure_code").is_some())
                .map(|db| {
                    let name = field(db, "database_name").map(clean).unwrap_or_else(|| "?".into());
                    match field(db, "failure_code") {
                        Some(code) => format!("  - {name} ({})", clean(code)),
                        None => format!("  - {name}"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let mut lines = vec![match kind {
        Kind::Restore => format!("The restore of {slug} failed."),
        _ => format!("The archive of {slug} failed: a database whose snapshot could not be taken and verified was not deleted and still serves."),
    }];
    if !failed.is_empty() {
        lines.push("Databases that failed:".into());
        lines.extend(failed);
    }
    lines.push(format!(
        "The project is visible again. An Owner can retry with `axiom project archive {slug}` or bring back what was archived with `axiom project restore {slug}`."
    ));
    anyhow!("{}", lines.join("\n"))
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

async fn read_view(api: &Api, slug: &str) -> Result<Value> {
    let reply = api.request(&Method::GET, &["projects", slug, "archive"], &[], None, None).await?;
    if (200..300).contains(&reply.status) {
        return Ok(reply.body);
    }
    Err(refusal(Kind::Status, slug, &reply))
}

/// Poll `GET …/archive` until archived, restored or failed. Succeeds only on `want`.
async fn wait_for(api: &Api, slug: &str, kind: Kind, mut view: Value, poll: Poll, human: bool) -> Result<Value> {
    let want = if kind == Kind::Restore { "restored" } else { "archived" };
    let deadline = tokio::time::Instant::now() + poll.limit;
    let mut last = String::new();
    while !is_terminal_state(state(&view)) {
        if human && state(&view) != last {
            last = state(&view).to_string();
            eprintln!("{}", state_words(&last));
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "{slug} is still {} after {} minutes. Follow it with `axiom project archive-status {slug}`.",
                clean(state(&view)),
                poll.limit.as_secs() / 60
            );
        }
        tokio::time::sleep(poll.interval).await;
        view = read_view(api, slug).await?;
    }
    let end = state(&view).to_string();
    match end.as_str() {
        s if s == want => Ok(view),
        "failed" => Err(failed_error(kind, slug, &view)),
        other => bail!("{slug} ended {}, not {want}. See `axiom project archive-status {slug}`.", state_words(other)),
    }
}

fn print(json: bool, value: &Value, human: Vec<String>) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        for line in human {
            println!("{line}");
        }
    }
    Ok(())
}

/// `axiom project archive`.
pub async fn archive(api: &Api, given: Option<&str>, yes: bool, wait: bool, json: bool, poll: Poll, prompt: &mut impl Prompt) -> Result<()> {
    if !yes && !prompt.is_terminal() {
        return Err(exit2(super::auth::NEEDS_YES));
    }
    let slug = project_slug(api, given).await?;
    let phrase = confirm_phrase(&slug);
    if !yes {
        // Show what would happen, and refuse early what the server would refuse.
        let view = read_view(api, &slug).await?;
        if view.get("enabled").and_then(Value::as_bool) == Some(false) {
            bail!("AXPROJECT_ARCHIVE_DISABLED: Project archive is not enabled for `{slug}`. Nothing was changed.");
        }
        let blockers = blockers(&view);
        if !blockers.is_empty() && !matches!(state(&view), "archiving" | "archived") {
            bail!("AXPROJECT_ARCHIVE_BLOCKED: {}", blocked_message(&slug, &blockers));
        }
        for line in consequences(&slug, &view) {
            eprintln!("{line}");
        }
        let answer = prompt.input(&format!("Type `{phrase}` to confirm"))?;
        if answer.trim() != phrase {
            return Err(exit2("the confirmation did not match; nothing was changed"));
        }
    }
    let key = uuid::Uuid::new_v4().to_string();
    eprintln!("Archive idempotency key: {key}");
    let reply = api.post_action_reply(&["projects", &slug, "archive"], &json!({"confirm": phrase}), &key).await?;
    if !(200..300).contains(&reply.status) {
        return Err(refusal(Kind::Archive, &slug, &reply));
    }
    finish(api, &slug, Kind::Archive, reply.body, wait, json, poll).await
}

/// `axiom project restore`.
pub async fn restore(api: &Api, given: Option<&str>, wait: bool, json: bool, poll: Poll) -> Result<()> {
    let slug = project_slug(api, given).await?;
    let key = uuid::Uuid::new_v4().to_string();
    eprintln!("Restore idempotency key: {key}");
    let reply = api.request(&Method::POST, &["projects", &slug, "restore"], &[], Some(&json!({})), Some(&key)).await?;
    if !(200..300).contains(&reply.status) {
        return Err(refusal(Kind::Restore, &slug, &reply));
    }
    finish(api, &slug, Kind::Restore, reply.body, wait, json, poll).await
}

async fn finish(api: &Api, slug: &str, kind: Kind, view: Value, wait: bool, json: bool, poll: Poll) -> Result<()> {
    if !wait {
        let text = accepted_text(kind, slug, &view);
        return print(json, &view, text);
    }
    let view = wait_for(api, slug, kind, view, poll, !json).await?;
    let text = accepted_text(kind, slug, &view);
    print(json, &view, text)
}

/// `axiom project archive-status`.
pub async fn status(api: &Api, given: Option<&str>, json: bool) -> Result<()> {
    let slug = project_slug(api, given).await?;
    let view = read_view(api, &slug).await?;
    print(json, &view, render_view(&slug, &view))
}

/// `axiom project list --archived`: the caller's archived (hidden) projects.
pub async fn list_archived(api: &Api) -> Result<Vec<String>> {
    let listed = cloud_cli::call(api, Method::GET, &["projects"], &[("archived", "true".into())], None).await?;
    let projects = items(&listed, &["projects", "items"]);
    if projects.is_empty() {
        return Ok(vec!["No archived projects.".into()]);
    }
    let mut out = vec![" Archived projects:".to_string()];
    for p in projects {
        let name = field(p, "name").map(clean).unwrap_or_default();
        let slug = field(p, "slug").map(clean).unwrap_or_default();
        let archive = p.get("archive").cloned().unwrap_or(Value::Null);
        let state = field(&archive, "state").map(clean).unwrap_or_else(|| "archived".into());
        let since = field(&archive, "requested_at").map(|at| format!(", since {}", clean(at))).unwrap_or_default();
        out.push(format!("  • {name}  {slug}  ({state}{since})"));
    }
    out.push("Restore one with `axiom project restore <slug>`.".into());
    Ok(out)
}

// ---------------------------------------------------------------------------
// Entry points used by `main.rs`
// ---------------------------------------------------------------------------

pub async fn handle_archive(project: Option<&str>, yes: bool, wait: bool, json: bool) -> Result<()> {
    let mut prompt = TerminalPrompt;
    // `main` already refused this before anything else; keep the handler safe on its own.
    if !yes && !prompt.is_terminal() {
        return Err(exit2(super::auth::NEEDS_YES));
    }
    let api = Api::from_profile()?;
    archive(&api, project, yes, wait, json, Poll::default(), &mut prompt).await
}

pub async fn handle_restore(project: Option<&str>, wait: bool, json: bool) -> Result<()> {
    let api = Api::from_profile()?;
    restore(&api, project, wait, json, Poll::default()).await
}

pub async fn handle_status(project: Option<&str>, json: bool) -> Result<()> {
    let api = Api::from_profile()?;
    status(&api, project, json).await
}

pub async fn handle_list_archived() -> Result<()> {
    let api = Api::from_profile()?;
    for line in list_archived(&api).await? {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
