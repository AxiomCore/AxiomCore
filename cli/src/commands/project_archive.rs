//! `axiom project archive`, `axiom project restore`, `axiom project archive-status` and
//! `axiom project list --archived` (V7 project archive, `/api/v1/projects/{slug}/archive`).
//!
//! Archive is destructive: it needs the typed confirmation `archive <project-slug>` on a
//! terminal, or `--yes` (which sends that same phrase). `main` asks (or, without a terminal
//! and without `--yes`, refuses with exit 2) before sign-in, the project lookup or any
//! request; the typed slug must then be the slug the project resolves to. The
//! archive POST follows the step-up rule of the per-database archive
//! ([`Api::post_action_reply`]); restore is not destructive and uses the session only.
//! Only known codes select fixed messages; server text and unknown values are cleaned.
use super::cloud_cli::{self, clean, items};
use super::resources_recovery::{Api, Reply};
use anyhow::{anyhow, bail, Result};
use reqwest::Method;
use serde_json::{json, Value};
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
    /// Standard input and standard error both: the prompt is drawn on standard error.
    fn is_terminal(&self) -> bool {
        super::auth::can_prompt()
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

/// Which command resolves the project: a restore prefers an archived project when two
/// projects match at the same step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purpose {
    Archive,
    Restore,
    Status,
}

/// The project slug from the argument (a slug, name or id) or the project linked to this
/// directory. An archived project is hidden from `GET /api/v1/projects`, so both that list
/// and `?archived=true` are read, merged (by id) and searched once: id, then exact slug, then
/// exact name, then case-insensitive slug and name. Two matches at the same step are refused,
/// except that a restore takes the only archived one among them.
pub async fn project_slug(api: &Api, given: Option<&str>, purpose: Purpose) -> Result<String> {
    let wanted = match given.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => crate::auth_store::get_project_id(&std::env::current_dir()?)?.ok_or_else(|| {
            anyhow!("no project: pass the project slug, name or id, or link this directory with `axiom project link`")
        })?,
    };
    let listed = cloud_cli::get(api, &["projects"]).await?;
    let archived = cloud_cli::call(api, Method::GET, &["projects"], &[("archived", "true".into())], None).await?;
    let archived_items = items(&archived, &["projects", "items"]);
    let id_of = |item: &Value| item.get("id").and_then(Value::as_str).map(str::to_string);
    let archived_ids: Vec<String> = archived_items.iter().filter_map(|i| id_of(i)).collect();
    // (project, is archived)
    let mut list: Vec<(&Value, bool)> = Vec::new();
    for item in items(&listed, &["projects", "items"]).into_iter().chain(archived_items.iter().copied()) {
        let id = id_of(item);
        if id.is_some() && list.iter().any(|(known, _)| id_of(known) == id) {
            continue;
        }
        let is_archived = id.as_ref().is_some_and(|id| archived_ids.contains(id));
        list.push((item, is_archived));
    }
    slug_of(choose(&list, &wanted, purpose)?, &wanted)
}

fn choose<'a>(list: &[(&'a Value, bool)], wanted: &str, purpose: Purpose) -> Result<&'a Value> {
    let value = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
    let steps = [("id", true), ("slug", true), ("name", true), ("slug", false), ("name", false)];
    for (key, exact) in steps {
        let found: Vec<(&'a Value, bool)> = list
            .iter()
            .copied()
            .filter(|(item, _)| value(item, key).is_some_and(|v| if exact { v == wanted } else { v.eq_ignore_ascii_case(wanted) }))
            .collect();
        match found.as_slice() {
            [] => continue,
            [(one, _)] => return Ok(one),
            many => {
                let archived: Vec<&&'a Value> = many.iter().filter(|(_, a)| *a).map(|(item, _)| item).collect();
                if purpose == Purpose::Restore {
                    if let [one] = archived.as_slice() {
                        return Ok(one);
                    }
                }
                let slugs: Vec<String> = many
                    .iter()
                    .take(5)
                    .map(|(item, a)| format!("{}{}", clean(&value(item, "slug").unwrap_or_default()), if *a { " (archived)" } else { "" }))
                    .collect();
                bail!("`{}` matches {} projects: {}. Pass the slug instead.", clean(wanted), many.len(), slugs.join("; "));
            }
        }
    }
    bail!("no project `{}` here, archived or not; see `axiom project list` and `axiom project list --archived`", clean(wanted))
}

/// A slug the API gave us is used verbatim in the URL and the confirmation phrase, so it is
/// refused, never cleaned, unless it is plain lowercase letters, digits and hyphens.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 128 && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn slug_of(project: &Value, wanted: &str) -> Result<String> {
    let slug = project
        .get("slug")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("the project list has no slug for `{}`", clean(wanted)))?;
    if !valid_slug(slug) {
        bail!("the project list gave `{}` a slug that is not lowercase letters, digits and hyphens; nothing was sent", clean(wanted));
    }
    Ok(slug.to_string())
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
            Kind::Restore => format!("An archive of `{slug}` is still running, or another operation is running on one of its archived databases. Wait until it finishes (`axiom project archive-status {slug}` shows each database's operation), then retry the restore."),
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
    let state = state(view);
    let enabled = view.get("enabled").and_then(Value::as_bool) != Some(false);
    if !enabled && state == "none" {
        return vec![format!("Project archive is not enabled for `{slug}`.")];
    }
    let mut out = vec![format!("Project {slug}: {}", state_words(state))];
    if !enabled {
        // Restore works with the flag off; a new archive does not.
        out.push(format!("  Project archive is not enabled for `{slug}`: it cannot be archived again, but it can be restored."));
    }
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

/// The databases an archive takes, once the project has been read.
fn database_line(view: &Value) -> Option<String> {
    let names: Vec<String> = view
        .get("databases")
        .and_then(Value::as_array)
        .map(|dbs| dbs.iter().filter_map(|db| field(db, "database_name")).map(clean).collect())
        .unwrap_or_default();
    (!names.is_empty()).then(|| format!("Archiving databases: {}", names.join(", ")))
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

/// At most this many status reads in a row may fail (5xx, 429 or no answer) during `--wait`.
const MAX_FAILED_READS: u32 = 10;

/// Poll `GET …/archive` until archived, restored or failed. Succeeds only on `want`. A 5xx,
/// 429 or transport failure is retried until the deadline (at most [`MAX_FAILED_READS`] in a
/// row); any other refusal stops at once. On failure the last view read is returned with the
/// error, so `--json` can still print it.
async fn wait_for(api: &Api, slug: &str, kind: Kind, mut view: Value, poll: Poll, human: bool) -> Result<Value, (Value, anyhow::Error)> {
    let want = if kind == Kind::Restore { "restored" } else { "archived" };
    let deadline = tokio::time::Instant::now() + poll.limit;
    let mut last = String::new();
    let mut failed_reads = 0u32;
    while !is_terminal_state(state(&view)) {
        if human && state(&view) != last {
            last = state(&view).to_string();
            eprintln!("{}", state_words(&last));
        }
        if tokio::time::Instant::now() >= deadline {
            let error = anyhow!(
                "{slug} is still {} after {} minutes. Follow it with `axiom project archive-status {slug}`.",
                clean(state(&view)),
                poll.limit.as_secs() / 60
            );
            return Err((view, error));
        }
        tokio::time::sleep(poll.interval).await;
        let transient = match api.request(&Method::GET, &["projects", slug, "archive"], &[], None, None).await {
            Ok(reply) if (200..300).contains(&reply.status) => {
                view = reply.body;
                failed_reads = 0;
                continue;
            }
            Ok(reply) if reply.status >= 500 || reply.status == 429 => refusal(Kind::Status, slug, &reply),
            Ok(reply) => {
                let error = refusal(Kind::Status, slug, &reply);
                return Err((view, error));
            }
            Err(error) => error,
        };
        failed_reads += 1;
        if human {
            eprintln!("Could not read the archive status ({transient}); retrying.");
        }
        if failed_reads >= MAX_FAILED_READS {
            let error = anyhow!("Stopped waiting: {MAX_FAILED_READS} status reads in a row failed; the last: {transient}. The {} may still be running; follow it with `axiom project archive-status {slug}`.", if kind == Kind::Restore { "restore" } else { "archive" });
            return Err((view, error));
        }
    }
    let end = state(&view).to_string();
    match end.as_str() {
        s if s == want => Ok(view),
        "failed" => {
            let error = failed_error(kind, slug, &view);
            Err((view, error))
        }
        other => {
            let error = anyhow!("{slug} ended {}, not {want}. See `axiom project archive-status {slug}`.", state_words(other));
            Err((view, error))
        }
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

/// How an archive was confirmed: `--yes`, or the phrase typed at the terminal for the
/// project argument exactly as given (see [`confirm`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirmed {
    Yes,
    Typed(String),
}

/// The phrase typed in `main`, before sign-in, registration or any request.
static TYPED: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Ask for the typed confirmation before anything is sent: no sign-in, no project lookup, no
/// request. The phrase is `archive <PROJECT>` with the argument exactly as given, so it must
/// already be a slug (lowercase letters, digits and hyphens); anything else, no argument
/// (a linked directory stores an id, not a slug) or a wrong answer is refused with exit 2.
/// Returns the confirmed slug; [`archive`] refuses if the project resolves to another slug.
pub fn confirm(given: Option<&str>, prompt: &mut impl Prompt) -> Result<String> {
    if !prompt.is_terminal() {
        return Err(exit2(super::auth::NEEDS_YES));
    }
    let Some(given) = given.filter(|g| !g.is_empty()) else {
        return Err(exit2(
            "name the project to archive by its slug: `axiom project archive <project-slug>` (see `axiom project list`); nothing was sent",
        ));
    };
    if !valid_slug(given) {
        return Err(exit2(format!(
            "`{}` is not a project slug (lowercase letters, digits and hyphens); the prompt asks for the slug, see `axiom project list`. Nothing was sent.",
            clean(given)
        )));
    }
    let phrase = confirm_phrase(given);
    eprintln!("Archiving `{given}` hides it from project lists at once. Each database gets an encrypted, verified snapshot; only after its snapshot verifies is the database deleted from its hosting provider.");
    eprintln!("Apps cannot use the project's databases until an Owner restores it with `axiom project restore {given}`, which brings them back from their snapshots with their data.");
    let answer = prompt.input(&format!("Type `{phrase}` to confirm"))?;
    if answer.trim() != phrase {
        return Err(exit2("the confirmation did not match; nothing was sent"));
    }
    Ok(given.to_string())
}

/// `main`: the typed confirmation, asked before the registration gate and any request. The
/// answer is kept for [`handle_archive`].
pub fn confirm_first(given: Option<&str>) -> Result<()> {
    let slug = confirm(given, &mut TerminalPrompt)?;
    let _ = TYPED.set(slug);
    Ok(())
}

/// `axiom project archive`. With [`Confirmed::Typed`] the prompt was already answered and
/// nothing was sent before it; the project is looked up only now, and refused without a
/// write if its slug is not the one typed, it is not enabled or it is blocked.
pub async fn archive(api: &Api, given: Option<&str>, confirmed: &Confirmed, wait: bool, json: bool, poll: Poll) -> Result<()> {
    let slug = project_slug(api, given, Purpose::Archive).await?;
    let phrase = confirm_phrase(&slug);
    if let Confirmed::Typed(typed) = confirmed {
        if typed != &slug {
            bail!(
                "you confirmed `archive {typed}`, but `{typed}` is the project `{slug}`. Nothing was archived. To archive it, run `axiom project archive {slug}`."
            );
        }
        // Refuse early, with words, what the server would refuse.
        let view = read_view(api, &slug).await?;
        if view.get("enabled").and_then(Value::as_bool) == Some(false) {
            bail!("AXPROJECT_ARCHIVE_DISABLED: Project archive is not enabled for `{slug}`. Nothing was changed.");
        }
        let blockers = blockers(&view);
        if !blockers.is_empty() && !matches!(state(&view), "archiving" | "archived") {
            bail!("AXPROJECT_ARCHIVE_BLOCKED: {}", blocked_message(&slug, &blockers));
        }
        if let Some(line) = database_line(&view) {
            eprintln!("{line}");
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
    let slug = project_slug(api, given, Purpose::Restore).await?;
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
    match wait_for(api, slug, kind, view, poll, !json).await {
        Ok(view) => {
            let text = accepted_text(kind, slug, &view);
            print(json, &view, text)
        }
        Err((view, error)) => {
            // `--json` always prints the last view read, also when the wait ends badly.
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            }
            Err(error)
        }
    }
}

/// `axiom project archive-status`.
pub async fn status(api: &Api, given: Option<&str>, json: bool) -> Result<()> {
    let slug = project_slug(api, given, Purpose::Status).await?;
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
    // `main` normally asked already, before anything else; keep the handler safe on its own:
    // the prompt (or the headless refusal) still comes before the profile and any request.
    let confirmed = match (yes, TYPED.get()) {
        (true, _) => Confirmed::Yes,
        (false, Some(typed)) => Confirmed::Typed(typed.clone()),
        (false, None) => Confirmed::Typed(confirm(project, &mut TerminalPrompt)?),
    };
    let api = Api::from_profile()?;
    archive(&api, project, &confirmed, wait, json, Poll::default()).await
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
