//! `axiom contract releases`: the hosted contract releases of a Cloud project, the same
//! endpoints the dashboard's contract pages use. List and inspect releases, read the semantic
//! analysis the release worker persisted (manifest, verification, semantic view, routes and
//! models), compare a release with an earlier one (diff summary and impact), queue analysis of
//! older releases, import a release from an HTTPS URL and download a release artifact.
//!
//! `axiom contract verify/diff/resolve` work on local lock files; this module only talks to
//! Axiom Cloud through the CLI session (`axiom login`). The artifact download is binary, so it
//! uses the existing `CloudClient::pull_contract_content` (refreshed on a 401) and checks the
//! bytes against the SHA-256 the release record carries before anything is written.
use super::cloud_cli;
use super::resources_recovery::Api;
use anyhow::{anyhow, bail, Context, Result};
use axiom_cloud::CloudClient;
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Human output shows at most this many item lines; `--json` always has everything.
const MAX_LINES: usize = 50;

#[derive(Args, Debug)]
pub struct ReleasesArgs {
    /// Cloud project: slug, name or id (defaults to the project linked to this directory)
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Machine-readable output: the server's JSON reply
    #[arg(long, global = true)]
    pub json: bool,
    /// Show release source URLs in full. By default their user name, password, query values
    /// and fragment are replaced with REDACTED, because an imported URL can carry credentials
    /// or a signed-URL token
    #[arg(long, global = true)]
    pub show_source_url: bool,
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Every hosted release of the project: version, signing status, creation time, size
    List,
    /// One release: id, version, SHA-256, signing key, source and status
    Show {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// The semantic manifest the release worker extracted (counts and hashes; `--json` for the document)
    Manifest {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// The verification record of a release: status, signature check, verifier, failure reason
    Verification {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// Compatibility summary of a release against an earlier one: counts, breaking flag, first changes
    ///
    /// The server always returns a bounded summary with the first ordered changes; the complete,
    /// cursor-paged evidence is in the native contract review (pull requests).
    Diff {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
        /// An earlier release version, or `baseline` to compare with nothing (the default, as on
        /// the dashboard: an earlier release is never chosen implicitly)
        #[arg(long)]
        against: Option<String>,
    },
    /// Impact of a release against an earlier one: affected routes, observed consumers, domain paths
    Impact {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
        /// An earlier release version, or `baseline` (the default, as on the dashboard)
        #[arg(long)]
        against: Option<String>,
    },
    /// The compact semantic view of a release (service, endpoints, domain) for agents and tools
    SemanticView {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// One page of the routes or models of a release
    SemanticItems {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
        /// Which collection: `routes` or `models`
        #[arg(long, value_parser = ["routes", "models"])]
        kind: String,
        /// Start of the page (the `next_cursor` of the previous page)
        #[arg(long)]
        cursor: Option<u64>,
        /// Items per page, 1 to 100
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Queue semantic analysis of a release signed before analysis existed (release managers)
    Backfill {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// The endpoints declared by a release artifact
    Endpoints {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// The entities (models) declared by a release artifact
    Entities {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
    },
    /// Import a contract artifact from an HTTPS URL as a new signed release (release managers)
    ///
    /// A URL that carries credentials or a signed token should not be passed with --url (argv
    /// is visible to other local processes and shell history): use --url-stdin or --url-file,
    /// or give no URL option on a terminal to be asked for it without echo.
    Import {
        /// Public HTTPS URL of the `.axiom` artifact (port 443, no redirects, at most 20 MB)
        #[arg(long, conflicts_with_all = ["url_stdin", "url_file"])]
        url: Option<String>,
        /// Read the URL from standard input (on a terminal, the hidden prompt is used instead)
        #[arg(long, conflicts_with = "url_file")]
        url_stdin: bool,
        /// Read the URL from this file
        #[arg(long)]
        url_file: Option<PathBuf>,
        /// Release version; default: the version inside the artifact
        #[arg(long)]
        version: Option<String>,
        /// Rebuild the artifact with VERSION instead of refusing a mismatching embedded version
        #[arg(long)]
        rebuild: bool,
    },
    /// Download a release artifact to a file and check its SHA-256 against the release record
    ///
    /// A release record without a valid SHA-256 is refused before anything is downloaded, unless
    /// --allow-unverified is passed; a digest that does not match is always refused.
    Download {
        /// Release version, or `latest` for the active release (a contract id is not accepted)
        version: String,
        /// File to write; default: `VERSION.axiom` in the current directory
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Replace FILE if it exists
        #[arg(long)]
        force: bool,
        /// Write the artifact even when the release record has no valid SHA-256 to check it
        /// against (the bytes are then unverified; a mismatching digest is still refused)
        #[arg(long)]
        allow_unverified: bool,
    },
}

pub async fn handle(args: &ReleasesArgs) -> Result<()> {
    let api = Api::from_profile()?;
    run(&api, args).await
}

pub async fn run(api: &Api, args: &ReleasesArgs) -> Result<()> {
    let slug = cloud_cli::project_slug(api, args.project.as_deref()).await?;
    let json = args.json;
    let output = |value: Value, human: fn(&Value) -> Vec<String>| cloud_cli::print(json, &shown(args.show_source_url, value), human);
    match &args.action {
        Action::List => {
            let listed = list_releases(api, &slug).await?;
            output(listed, render_list)
        }
        Action::Show { version } => {
            let value = release(api, &slug, version).await?;
            output(value, render_release)
        }
        Action::Manifest { version } => {
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "manifest"], &[], None).await.map_err(|e| not_ready(e, version))?;
            output(value, render_manifest)
        }
        Action::Verification { version } => {
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "verification"], &[], None).await.map_err(|e| not_ready(e, version))?;
            output(value, render_verification)
        }
        Action::Diff { version, against } => {
            let query = comparison_query(against.as_deref(), true);
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "diff"], &query, None).await.map_err(|e| comparison_not_ready(e, version, &query[0].1))?;
            output(value, render_diff)
        }
        Action::Impact { version, against } => {
            let query = comparison_query(against.as_deref(), false);
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "impact"], &query, None).await.map_err(|e| comparison_not_ready(e, version, &query[0].1))?;
            output(value, render_impact)
        }
        Action::SemanticView { version } => {
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "semantic-view"], &[], None).await.map_err(|e| not_ready(e, version))?;
            output(value, render_semantic_view)
        }
        Action::SemanticItems { version, kind, cursor, limit } => {
            let query = items_query(kind, *cursor, *limit);
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "semantic-items"], &query, None).await.map_err(|e| not_ready(e, version))?;
            output(value, render_semantic_items)
        }
        Action::Backfill { version } => {
            let value = cloud_cli::call(api, Method::POST, &["projects", &slug, "contracts", version, "semantic-backfill"], &[], None).await?;
            output(value, render_backfill)
        }
        Action::Endpoints { version } => {
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "endpoints"], &[], None).await?;
            output(value, render_endpoints)
        }
        Action::Entities { version } => {
            let value = cloud_cli::call(api, Method::GET, &["projects", &slug, "contracts", version, "entities"], &[], None).await?;
            output(value, render_entities)
        }
        Action::Import { url, url_stdin, url_file, version, rebuild } => {
            let url = import_url(url.as_deref(), *url_stdin, url_file.as_deref())?;
            let body = import_body(&url, version.as_deref(), *rebuild);
            let value = cloud_cli::call(api, Method::POST, &["projects", &slug, "contracts", "import"], &[], Some(&body)).await.map_err(import_error)?;
            output(value, render_import)
        }
        Action::Download { version, output, force, allow_unverified } => {
            let options = DownloadOptions { force: *force, allow_unverified: *allow_unverified, json };
            download(api, crate::auth_store::authenticated_cloud_client, &slug, version, output.as_deref(), options).await
        }
    }
}

/// `GET /api/v1/projects/{project}/contracts`.
async fn list_releases(api: &Api, slug: &str) -> Result<Value> {
    cloud_cli::call(api, Method::GET, &["projects", slug, "contracts"], &[], None).await
}

/// `GET /api/v1/projects/{project}/contracts/{version}`.
async fn release(api: &Api, slug: &str, version: &str) -> Result<Value> {
    cloud_cli::call(api, Method::GET, &["projects", slug, "contracts", version], &[], None).await
}

/// The semantic reads answer 409 until the release worker has produced the analysis.
fn not_ready(error: anyhow::Error, version: &str) -> anyhow::Error {
    if error.to_string().contains("(HTTP 409)") {
        let v = cloud_cli::clean(version);
        error.context(format!("The analysis of release {v} is not ready yet. The release worker produces it shortly after signing; for a release signed before analysis existed, run `axiom contract releases backfill {v}`."))
    } else {
        error
    }
}

/// [`not_ready`] for diff and impact, where either release may lack its analysis.
fn comparison_not_ready(error: anyhow::Error, version: &str, against: &str) -> anyhow::Error {
    if error.to_string().contains("(HTTP 409)") {
        let (v, a) = (cloud_cli::clean(version), cloud_cli::clean(against));
        let source = if a == "baseline" { String::new() } else { format!(" or of {a}") };
        error.context(format!("The analysis of release {v}{source} is not ready yet. For a release signed before analysis existed, run `axiom contract releases backfill <VERSION>` for it, or compare with `--against baseline`."))
    } else {
        error
    }
}

/// The `against` query of diff and impact (and `summary=true` for diff, like the dashboard).
/// Without `--against` the comparison is with `baseline`, as on the dashboard, which never
/// picks an earlier (possibly unverified or unanalysed) release implicitly.
fn comparison_query(against: Option<&str>, summary: bool) -> Vec<(&'static str, String)> {
    let against = against.map(str::trim).filter(|a| !a.is_empty()).unwrap_or("baseline");
    let mut query = vec![("against", against.to_string())];
    if summary {
        query.push(("summary", "true".into()));
    }
    query
}

/// An import can outlast the 20 s request timeout while the server fetches, signs and verifies;
/// the server derives its idempotency from the artifact, so a retry is safe.
fn import_error(error: anyhow::Error) -> anyhow::Error {
    if error.to_string().contains("request failed") {
        error.context("The import may still complete on the server. Check `axiom contract releases list` before retrying; a retry of the same artifact is safe.")
    } else {
        error
    }
}

fn items_query(kind: &str, cursor: Option<u64>, limit: u32) -> Vec<(&'static str, String)> {
    let mut query = vec![("kind", kind.to_string()), ("limit", limit.to_string())];
    if let Some(cursor) = cursor {
        query.push(("cursor", cursor.to_string()));
    }
    query
}

/// Longest import URL read from standard input or a file.
const MAX_URL_INPUT: u64 = 8 * 1024;

/// The import URL from `--url`, `--url-stdin`, `--url-file`, or a prompt without echo on a
/// terminal, so a URL with credentials never has to be an argument.
fn import_url(url: Option<&str>, stdin: bool, file: Option<&Path>) -> Result<Zeroizing<String>> {
    use std::io::{IsTerminal, Read};
    let mut raw = Zeroizing::new(String::new());
    if let Some(url) = url {
        raw.push_str(url);
    } else if stdin && !std::io::stdin().is_terminal() {
        std::io::stdin().take(MAX_URL_INPUT).read_to_string(&mut raw).context("read the artifact URL from standard input")?;
    } else if let Some(path) = file {
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?.take(MAX_URL_INPUT).read_to_string(&mut raw).with_context(|| format!("read {}", path.display()))?;
    } else if std::io::stdin().is_terminal() {
        raw = Zeroizing::new(dialoguer::Password::new().with_prompt("HTTPS URL of the .axiom artifact").interact()?);
    } else {
        bail!("no artifact URL: pass --url, --url-stdin or --url-file <path>");
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("the artifact URL is empty");
    }
    Ok(Zeroizing::new(trimmed.to_string()))
}

/// Replaces what can carry a secret in a URL: user name and password, every query value and
/// the fragment. A value that is not an absolute URL with a host (an opaque `scheme:rest`
/// value has no separable credentials) is hidden completely.
fn redact_url(raw: &str) -> String {
    const HIDDEN: &str = "REDACTED";
    let mut url = match url::Url::parse(raw.trim()) {
        Ok(url) if url.has_host() && !url.cannot_be_a_base() => url,
        _ => return format!("{HIDDEN} (not a URL; --show-source-url shows it)"),
    };
    if !url.username().is_empty() || url.password().is_some() {
        // Both fail only for URLs that cannot have credentials, which then have none to hide.
        let _ = url.set_password(None);
        let _ = url.set_username(HIDDEN);
    }
    if url.query().is_some() {
        let names: Vec<String> = url.query_pairs().map(|(name, _)| name.into_owned()).collect();
        if names.is_empty() {
            url.set_query(Some(HIDDEN));
        } else {
            url.query_pairs_mut().clear().extend_pairs(names.iter().map(|name| (name.as_str(), HIDDEN)));
        }
    }
    if url.fragment().is_some() {
        url.set_fragment(Some(HIDDEN));
    }
    url.to_string()
}

/// `value` as it may be shown: every non-empty `source_url`, at any depth, redacted unless
/// `--show-source-url`.
fn shown(show_source_url: bool, mut value: Value) -> Value {
    fn walk(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, item) in map.iter_mut() {
                    match item {
                        Value::String(url) if key == "source_url" && !url.is_empty() => *url = redact_url(url),
                        _ => walk(item),
                    }
                }
            }
            Value::Array(list) => list.iter_mut().for_each(walk),
            _ => {}
        }
    }
    if !show_source_url {
        walk(&mut value);
    }
    value
}

/// The dashboard's import body: `version` is empty to keep the artifact's own version.
fn import_body(url: &str, version: Option<&str>, rebuild: bool) -> Value {
    json!({"url": url.trim(), "version": version.map(str::trim).unwrap_or(""), "rebuild": rebuild})
}

/// A version that is safe as one URL path segment of the (non-encoding) artifact pull.
fn checked_version(version: &str) -> Result<&str> {
    let ok = !version.is_empty() && version.len() <= 128 && !version.starts_with('.') && version.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'));
    if ok {
        Ok(version)
    } else {
        bail!("release version `{}` has characters a download cannot use", cloud_cli::clean(version))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DownloadOptions {
    /// Replace an existing output file.
    force: bool,
    /// Write the bytes even when the release record has no valid SHA-256.
    allow_unverified: bool,
    json: bool,
}

/// The SHA-256 of a release record, lower-cased, when it is 64 hex digits; otherwise `None`.
fn expected_sha256(record: &Value) -> Option<String> {
    let sha = record.get("sha256").and_then(Value::as_str)?.trim();
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then(|| sha.to_ascii_lowercase())
}

/// Downloads the artifact of `version` with the existing Cloud client, checks it against the
/// SHA-256 of the release record and writes it to a new file (or replaces it with `--force`).
/// Without a valid SHA-256 in the record nothing is downloaded unless `--allow-unverified`.
/// The client is created only after the record is read, so it starts from the session the
/// [`Api`] may just have refreshed (refresh tokens are single-use).
async fn download(api: &Api, client: impl FnOnce() -> Result<CloudClient>, slug: &str, version: &str, output: Option<&Path>, options: DownloadOptions) -> Result<()> {
    let DownloadOptions { force, allow_unverified, json } = options;
    let record = release(api, slug, version).await?;
    let resolved = record.get("version").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(version);
    let resolved = checked_version(resolved)?.to_string();
    let expected = expected_sha256(&record);
    if expected.is_none() && !allow_unverified {
        bail!("the release record of {resolved} has no valid SHA-256, so the download cannot be verified; nothing was downloaded. Pass --allow-unverified to write it anyway.");
    }
    let path = output.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(format!("{resolved}.axiom")));
    if !force && path.exists() {
        bail!("{} exists; pass --force to replace it", path.display());
    }
    let client = client()?;
    let bytes = client.pull_contract_content(slug, &resolved).await.map_err(|e| anyhow!("could not download release {resolved}: {}", cloud_cli::clean(&e.to_string())))?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    if let Some(expected) = expected.as_deref().filter(|e| *e != sha256) {
        bail!("the downloaded artifact of release {resolved} has SHA-256 {sha256}, but the release record says {expected}; nothing was written");
    }
    write_artifact(&path, &bytes, force)?;
    let checked = expected.is_some();
    let summary = json!({"version": resolved, "path": path.display().to_string(), "size_bytes": bytes.len(), "sha256": sha256, "sha256_checked": checked});
    cloud_cli::print(json, &summary, |_| {
        let mut lines = vec![format!("Downloaded release {resolved} to {} ({} bytes, SHA-256 {sha256}).", path.display(), bytes.len())];
        if !checked {
            lines.push("Unverified (--allow-unverified): the release record has no valid SHA-256, so the file was not checked against it.".into());
        }
        lines
    })
}

/// Writes a new file, or with `force` replaces `path` atomically: the bytes go to a temporary
/// file in the same directory that is renamed over it, so a failed write keeps the old file.
fn write_artifact(path: &Path, bytes: &[u8], force: bool) -> Result<()> {
    if !force {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).with_context(|| format!("could not create {} (pass --force to replace an existing file)", path.display()))?;
        return file.write_all(bytes).with_context(|| format!("could not write {}", path.display()));
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir).with_context(|| format!("could not create a temporary file in {}", dir.display()))?;
    temp.write_all(bytes).with_context(|| format!("could not write {}", path.display()))?;
    temp.persist(path).map_err(|e| anyhow!("could not replace {}: {}", path.display(), e.error))?;
    Ok(())
}

/// At most [`MAX_LINES`] lines, then a pointer to `--json`.
fn bounded(lines: Vec<String>) -> Vec<String> {
    let total = lines.len();
    let mut shown: Vec<String> = lines.into_iter().take(MAX_LINES).collect();
    if total > MAX_LINES {
        shown.push(format!("… {} more; use --json for all of them", total - MAX_LINES));
    }
    shown
}

fn yes_no(value: &Value, pointer: &str) -> &'static str {
    if value.pointer(pointer).and_then(Value::as_bool).unwrap_or(false) {
        "yes"
    } else {
        "no"
    }
}

fn count(value: &Value, pointer: &str) -> usize {
    value.pointer(pointer).and_then(Value::as_array).map_or(0, Vec::len)
}

fn render_list(value: &Value) -> Vec<String> {
    let list = cloud_cli::items(value, &["contracts", "items"]);
    if list.is_empty() {
        return vec!["No releases yet. Push one with `axiom release` or import one with `axiom contract releases import`.".into()];
    }
    bounded(
        list.iter()
            .map(|c| format!("{}  {}  created {}  {} bytes  source {}", cloud_cli::text(c, "/version"), cloud_cli::text(c, "/status"), cloud_cli::text(c, "/created_at"), cloud_cli::text(c, "/size_bytes"), cloud_cli::text(c, "/source_type")))
            .collect(),
    )
}

fn render_release(c: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("Release {} ({})", cloud_cli::text(c, "/version"), cloud_cli::text(c, "/status")),
        format!("  id          {}", cloud_cli::text(c, "/id")),
        format!("  created     {}", cloud_cli::text(c, "/created_at")),
        format!("  size        {} bytes", cloud_cli::text(c, "/size_bytes")),
        format!("  sha256      {}", cloud_cli::text(c, "/sha256")),
        format!("  signing key {}", cloud_cli::text(c, "/key_id")),
        format!("  source      {}", cloud_cli::text(c, "/source_type")),
    ];
    if c.pointer("/source_url").and_then(Value::as_str).is_some_and(|u| !u.is_empty()) {
        lines.push(format!("  source url  {}", cloud_cli::text(c, "/source_url")));
    }
    lines.push("Signed means provenance and integrity only; see `verification` for the semantic check.".into());
    lines
}

fn render_import(v: &Value) -> Vec<String> {
    vec![format!("Imported release {} ({}); SHA-256 {}.", cloud_cli::text(v, "/version"), cloud_cli::text(v, "/status"), cloud_cli::text(v, "/sha256"))]
}

fn render_manifest(m: &Value) -> Vec<String> {
    vec![
        format!("Manifest of release {}", cloud_cli::text(m, "/version")),
        format!("  endpoints   {} ({} streaming)", cloud_cli::text(m, "/endpoint_count"), cloud_cli::text(m, "/streaming_endpoint_count")),
        format!("  entities    {}", cloud_cli::text(m, "/entity_count")),
        format!("  schema hash {}", cloud_cli::text(m, "/schema_hash")),
        format!("  manifest    {}", cloud_cli::text(m, "/manifest_hash")),
        format!("  artifact    {}", cloud_cli::text(m, "/artifact_sha256")),
        format!("  extracted   {} by {}", cloud_cli::text(m, "/extracted_at"), cloud_cli::text(m, "/extractor_version")),
        "Use --json for the full manifest document.".into(),
    ]
}

fn render_verification(v: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("Verification: {}", cloud_cli::text(v, "/status")),
        format!("  signature verified {}", yes_no(v, "/signature_verified")),
        format!("  verified at        {} by {}", cloud_cli::text(v, "/verified_at"), cloud_cli::text(v, "/verifier_version")),
        format!("  manifest hash      {}", cloud_cli::text(v, "/manifest_hash")),
    ];
    if v.pointer("/failure_code").and_then(Value::as_str).is_some_and(|f| !f.is_empty()) || v.pointer("/failure_message").and_then(Value::as_str).is_some_and(|f| !f.is_empty()) {
        lines.push(format!("  failure            {}: {}", cloud_cli::text(v, "/failure_code"), cloud_cli::text(v, "/failure_message")));
    }
    lines
}

fn render_diff(d: &Value) -> Vec<String> {
    let breaking = d.pointer("/breaking_changes").and_then(Value::as_i64).unwrap_or(0);
    let mut lines = vec![
        format!("Release {} against {}: {} ({})", cloud_cli::text(d, "/to_version"), cloud_cli::text(d, "/from_version"), cloud_cli::text(d, "/compatibility"), cloud_cli::text(d, "/status")),
        format!("  breaking {}  risky {}  compatible {}", breaking, cloud_cli::text(d, "/risky_changes"), cloud_cli::text(d, "/compatible_changes")),
        format!("  breaking change: {}", if breaking > 0 { "yes" } else { "no" }),
    ];
    if d.pointer("/failure_message").and_then(Value::as_str).is_some_and(|f| !f.is_empty()) {
        lines.push(format!("  failure: {}", cloud_cli::text(d, "/failure_message")));
    }
    let changes = cloud_cli::items(d, &["changes"]);
    if !changes.is_empty() {
        lines.push("Changes:".into());
        lines.extend(bounded(
            changes.iter().map(|c| format!("  {}  {}  {}  {}", cloud_cli::text(c, "/severity"), cloud_cli::text(c, "/operation"), cloud_cli::text(c, "/path"), cloud_cli::text(c, "/summary"))).collect(),
        ));
    }
    if d.pointer("/highlights_truncated").and_then(Value::as_bool).unwrap_or(false) {
        lines.push("Only the first changes are shown; the full evidence is in the contract review (pull requests).".into());
    }
    lines
}

fn render_impact(i: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("Impact ({}): {} affected routes, {} observed consumers, {} observed events", cloud_cli::text(i, "/scope"), count(i, "/affected_routes"), count(i, "/observed_consumers"), cloud_cli::text(i, "/observed_events")),
        format!("  domain: {} entities, {} relationships, {} invariants, {} endpoints", count(i, "/domain/entities"), count(i, "/domain/relationships"), count(i, "/domain/invariants"), count(i, "/domain/endpoints")),
    ];
    let routes: Vec<String> = cloud_cli::items(i, &["affected_routes"]).iter().filter_map(|r| r.as_str()).map(|r| format!("  route {}", cloud_cli::clean(r))).collect();
    lines.extend(bounded(routes));
    let consumers: Vec<String> =
        cloud_cli::items(i, &["observed_consumers"]).iter().map(|c| format!("  consumer {} ({}) on {}: {} events, last seen {}", cloud_cli::text(c, "/name"), cloud_cli::text(c, "/platform"), cloud_cli::text(c, "/contract_version"), cloud_cli::text(c, "/event_count"), cloud_cli::text(c, "/last_seen_at"))).collect();
    lines.extend(bounded(consumers));
    if count(i, "/observed_consumers") == 0 {
        lines.push("No consumers observed; this is absence of telemetry, not evidence the change is safe.".into());
    }
    lines
}

fn render_semantic_view(s: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("Release {} of {}: {} endpoints, signature verified {}", cloud_cli::text(s, "/version"), cloud_cli::text(s, "/service_name"), count(s, "/endpoints"), yes_no(s, "/signature_verified")),
        format!("  domain: {} entities, {} relationships, {} invariants", count(s, "/domain/entities"), count(s, "/domain/relationships"), count(s, "/domain/invariants")),
    ];
    if s.pointer("/truncated").and_then(Value::as_bool).unwrap_or(false) {
        lines.push("  the server truncated this view".into());
    }
    lines.extend(bounded(cloud_cli::items(s, &["endpoints"]).iter().map(|e| format!("  {} {}  {}", cloud_cli::text(e, "/method"), cloud_cli::text(e, "/path"), cloud_cli::text(e, "/name"))).collect()));
    lines
}

fn render_semantic_items(p: &Value) -> Vec<String> {
    let kind = cloud_cli::text(p, "/kind");
    let items = cloud_cli::items(p, &["items"]);
    let mut lines = vec![format!("{} {} of {} in total", items.len(), kind, cloud_cli::text(p, "/total"))];
    for item in &items {
        lines.push(if kind == "models" {
            format!("  {}  {}  {} fields", cloud_cli::text(item, "/name"), cloud_cli::text(item, "/kind"), cloud_cli::text(item, "/field_count"))
        } else {
            format!("  {} {}  {}", cloud_cli::text(item, "/method"), cloud_cli::text(item, "/path"), cloud_cli::text(item, "/name"))
        });
    }
    if let Some(next) = p.get("next_cursor").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        lines.push(format!("Next page: --cursor {}", cloud_cli::clean(next)));
    }
    lines
}

fn render_backfill(b: &Value) -> Vec<String> {
    let status = match b.get("status").and_then(Value::as_str) {
        Some("queued") => "queued",
        Some("already_queued") => "already queued",
        _ => "requested",
    };
    vec![format!("Semantic analysis of release {} {status}. {}", cloud_cli::text(b, "/version"), cloud_cli::text(b, "/message"))]
}

fn render_endpoints(value: &Value) -> Vec<String> {
    let list = cloud_cli::items(value, &["endpoints", "items"]);
    if list.is_empty() {
        return vec!["No endpoints in this release.".into()];
    }
    bounded(
        list.iter()
            .map(|e| format!("{} {}  {}  auth {}  validation {}", cloud_cli::text(e, "/method"), cloud_cli::text(e, "/path"), cloud_cli::text(e, "/name"), yes_no(e, "/auth_required"), yes_no(e, "/validation")))
            .collect(),
    )
}

fn render_entities(value: &Value) -> Vec<String> {
    let list = cloud_cli::items(value, &["entities", "items"]);
    if list.is_empty() {
        return vec!["No entities in this release.".into()];
    }
    bounded(list.iter().map(|e| format!("{}  {}  {} fields", cloud_cli::text(e, "/name"), cloud_cli::text(e, "/kind"), cloud_cli::text(e, "/field_count"))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cloud_cli::test_support::harness;
    use crate::commands::cloud_mock::Mock;
    use crate::commands::step_up::StepUpStore;

    const PROJECTS: &str = "GET /api/v1/projects";
    const LIST: &str = "GET /api/v1/projects/demo/contracts";

    fn args(action: Action) -> ReleasesArgs {
        ReleasesArgs { project: Some("Demo App".into()), json: false, show_source_url: false, action }
    }

    fn projects(mock: &Mock) {
        mock.script(PROJECTS, 200, json!([{"id": "11111111-1111-4111-8111-111111111111", "slug": "demo", "name": "Demo App"}]));
    }

    fn releases() -> Value {
        json!([
            {"id": "c3", "version": "1.2.0", "status": "signed", "created_at": "2026-11-03T10:00:00Z", "size_bytes": 30, "source_type": "upload"},
            {"id": "c1", "version": "1.0.0", "status": "signed", "created_at": "2026-11-01T10:00:00Z", "size_bytes": 10, "source_type": "upload"},
            {"id": "c2", "version": "1.1.0", "status": "unsigned", "created_at": "2026-11-02T10:00:00Z", "size_bytes": 20, "source_type": "url"}
        ])
    }

    #[tokio::test]
    async fn list_and_show_resolve_the_project_name_to_its_slug() {
        let h = harness().await;
        projects(&h.mock);
        h.mock.script(LIST, 200, releases());
        h.mock.script("GET /api/v1/projects/demo/contracts/latest", 200, releases()[0].clone());
        run(&h.api, &args(Action::List)).await.unwrap();
        run(&h.api, &args(Action::Show { version: "latest".into() })).await.unwrap();
        assert_eq!(h.mock.seen(PROJECTS).len(), 2);
        assert_eq!(h.mock.seen(LIST).len(), 1);
        assert_eq!(h.mock.seen(LIST)[0].query, None);
        assert_eq!(h.mock.seen(LIST)[0].bearer.as_deref(), Some("access"));
        assert_eq!(h.mock.seen("GET /api/v1/projects/demo/contracts/latest").len(), 1);
        let missing = ReleasesArgs { project: Some("nope".into()), json: false, show_source_url: false, action: Action::List };
        assert!(run(&h.api, &missing).await.unwrap_err().to_string().contains("no project `nope`"));
    }

    #[tokio::test]
    async fn semantic_reads_call_their_endpoints() {
        let h = harness().await;
        projects(&h.mock);
        for (route, action) in [
            ("GET /api/v1/projects/demo/contracts/1.2.0/manifest", Action::Manifest { version: "1.2.0".into() }),
            ("GET /api/v1/projects/demo/contracts/1.2.0/verification", Action::Verification { version: "1.2.0".into() }),
            ("GET /api/v1/projects/demo/contracts/1.2.0/semantic-view", Action::SemanticView { version: "1.2.0".into() }),
            ("GET /api/v1/projects/demo/contracts/1.2.0/endpoints", Action::Endpoints { version: "1.2.0".into() }),
            ("GET /api/v1/projects/demo/contracts/1.2.0/entities", Action::Entities { version: "1.2.0".into() }),
        ] {
            h.mock.script(route, 200, json!({}));
            run(&h.api, &args(action)).await.unwrap();
            let seen = h.mock.seen(route);
            assert_eq!(seen.len(), 1, "{route}");
            assert_eq!(seen[0].query, None, "{route}");
        }
    }

    #[tokio::test]
    async fn semantic_items_send_kind_limit_and_cursor() {
        let h = harness().await;
        projects(&h.mock);
        let route = "GET /api/v1/projects/demo/contracts/1.2.0/semantic-items";
        h.mock.script(route, 200, json!({"kind": "routes", "total": 1, "items": [{"name": "get_user", "method": "GET", "path": "/users/{id}"}]}));
        run(&h.api, &args(Action::SemanticItems { version: "1.2.0".into(), kind: "routes".into(), cursor: None, limit: 50 })).await.unwrap();
        run(&h.api, &args(Action::SemanticItems { version: "1.2.0".into(), kind: "models".into(), cursor: Some(100), limit: 25 })).await.unwrap();
        let seen = h.mock.seen(route);
        assert_eq!(seen[0].query.as_deref(), Some("kind=routes&limit=50"));
        assert_eq!(seen[1].query.as_deref(), Some("kind=models&limit=25&cursor=100"));
    }

    #[tokio::test]
    async fn diff_and_impact_compare_with_baseline_by_default_like_the_dashboard() {
        let h = harness().await;
        projects(&h.mock);
        let diff = "GET /api/v1/projects/demo/contracts/1.2.0/diff";
        let impact = "GET /api/v1/projects/demo/contracts/1.2.0/impact";
        h.mock.script(diff, 200, json!({"from_version": "baseline", "to_version": "1.2.0", "breaking_changes": 1}));
        h.mock.script(impact, 200, json!({"affected_routes": ["GET /users"]}));
        run(&h.api, &args(Action::Diff { version: "1.2.0".into(), against: None })).await.unwrap();
        run(&h.api, &args(Action::Diff { version: "1.2.0".into(), against: Some("1.0.0".into()) })).await.unwrap();
        run(&h.api, &args(Action::Impact { version: "1.2.0".into(), against: None })).await.unwrap();
        run(&h.api, &args(Action::Impact { version: "1.2.0".into(), against: Some("1.1.0".into()) })).await.unwrap();
        let seen = h.mock.seen(diff);
        assert_eq!(seen[0].query.as_deref(), Some("against=baseline&summary=true"));
        assert_eq!(seen[1].query.as_deref(), Some("against=1.0.0&summary=true"));
        assert_eq!(h.mock.seen(impact)[0].query.as_deref(), Some("against=baseline"));
        assert_eq!(h.mock.seen(impact)[1].query.as_deref(), Some("against=1.1.0"));
        assert!(seen.iter().chain(h.mock.seen(impact).iter()).all(|s| s.body == Value::Null));
        assert!(h.mock.seen(LIST).is_empty(), "no implicit predecessor lookup");
    }

    #[tokio::test]
    async fn not_ready_analysis_points_to_backfill() {
        let h = harness().await;
        projects(&h.mock);
        h.mock.script("GET /api/v1/projects/demo/contracts/0.9.0/manifest", 409, json!({"message": "contract manifest is not ready"}));
        let error = format!("{:#}", run(&h.api, &args(Action::Manifest { version: "0.9.0".into() })).await.unwrap_err());
        assert!(error.contains("axiom contract releases backfill 0.9.0") && error.contains("HTTP 409"), "{error}");
        h.mock.script("GET /api/v1/projects/demo/contracts/0.9.0/diff", 409, json!({"message": "source manifest is not ready"}));
        let error = format!("{:#}", run(&h.api, &args(Action::Diff { version: "0.9.0".into(), against: Some("0.8.0".into()) })).await.unwrap_err());
        assert!(error.contains("release 0.9.0 or of 0.8.0 is not ready") && error.contains("--against baseline"), "{error}");
    }

    #[tokio::test]
    async fn backfill_and_import_send_the_dashboard_bodies() {
        let h = harness().await;
        projects(&h.mock);
        let backfill = "POST /api/v1/projects/demo/contracts/1.0.0/semantic-backfill";
        h.mock.script(backfill, 202, json!({"status": "queued", "version": "1.0.0", "message": "queued"}));
        run(&h.api, &args(Action::Backfill { version: "1.0.0".into() })).await.unwrap();
        assert_eq!(h.mock.seen(backfill)[0].body, Value::Null, "no body, like the dashboard");

        let import = "POST /api/v1/projects/demo/contracts/import";
        h.mock.script(import, 201, json!({"version": "2.0.0", "status": "signed", "sha256": "ab"}));
        run(&h.api, &args(Action::Import { url: Some(" https://example.com/a.axiom ".into()), url_stdin: false, url_file: None, version: None, rebuild: false })).await.unwrap();
        run(&h.api, &args(Action::Import { url: Some("https://example.com/a.axiom".into()), url_stdin: false, url_file: None, version: Some("2.0.0".into()), rebuild: true })).await.unwrap();
        let seen = h.mock.seen(import);
        assert_eq!(seen[0].body, json!({"url": "https://example.com/a.axiom", "version": "", "rebuild": false}));
        assert_eq!(seen[1].body, json!({"url": "https://example.com/a.axiom", "version": "2.0.0", "rebuild": true}));
    }

    async fn download_fixture() -> (Mock, Api, String, tempfile::TempDir) {
        let mock = Mock::default();
        let base = mock.start().await;
        let dir = tempfile::tempdir().unwrap();
        let api = Api::new(&base, "access".into(), "refresh".into(), false, StepUpStore::at(dir.path().join("step-up.json"))).unwrap().fast();
        (mock, api, base, dir)
    }

    fn loopback(base: &str) -> impl FnOnce() -> Result<CloudClient> + '_ {
        move || CloudClient::for_loopback(base, "access".into(), "refresh".into())
    }

    const STRICT: DownloadOptions = DownloadOptions { force: false, allow_unverified: false, json: false };

    #[tokio::test]
    async fn download_checks_the_sha256_and_never_overwrites_without_force() {
        let (mock, api, base, dir) = download_fixture().await;
        let artifact = json!("AXIOM-ARTIFACT");
        let bytes = artifact.to_string().into_bytes();
        let sha = hex::encode(Sha256::digest(&bytes));
        mock.script("GET /api/v1/projects/demo/contracts/latest", 200, json!({"version": "1.2.0", "sha256": sha}));
        mock.script("GET /api/v1/projects/demo/contracts/1.2.0/pull", 200, artifact);
        let path = dir.path().join("release.axiom");
        download(&api, loopback(&base), "demo", "latest", Some(&path), STRICT).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let pulled = mock.seen("GET /api/v1/projects/demo/contracts/1.2.0/pull");
        assert_eq!(pulled.len(), 1);
        assert_eq!(pulled[0].bearer.as_deref(), Some("access"));

        std::fs::write(&path, b"keep").unwrap();
        assert!(download(&api, loopback(&base), "demo", "latest", Some(&path), STRICT).await.unwrap_err().to_string().contains("--force"));
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        download(&api, loopback(&base), "demo", "latest", Some(&path), DownloadOptions { force: true, json: true, ..STRICT }).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).filter(|n| n != "release.axiom" && n != "step-up.json").collect();
        assert!(leftovers.is_empty(), "the temporary file is renamed over the target: {leftovers:?}");
    }

    #[tokio::test]
    async fn a_mismatching_download_is_not_written() {
        let (mock, api, base, dir) = download_fixture().await;
        mock.script("GET /api/v1/projects/demo/contracts/1.2.0", 200, json!({"version": "1.2.0", "sha256": "00".repeat(32)}));
        mock.script("GET /api/v1/projects/demo/contracts/1.2.0/pull", 200, json!("tampered"));
        let path = dir.path().join("release.axiom");
        let error = download(&api, loopback(&base), "demo", "1.2.0", Some(&path), STRICT).await.unwrap_err().to_string();
        assert!(error.contains("nothing was written"), "{error}");
        assert!(!path.exists());
        mock.script("GET /api/v1/projects/demo/contracts/x", 200, json!({"version": "../x", "sha256": ""}));
        assert!(download(&api, loopback(&base), "demo", "x", Some(&path), STRICT).await.is_err());
        // --allow-unverified never skips a check the record makes possible.
        let error = download(&api, loopback(&base), "demo", "1.2.0", Some(&path), DownloadOptions { allow_unverified: true, ..STRICT }).await.unwrap_err().to_string();
        assert!(error.contains("nothing was written"), "{error}");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_release_without_a_valid_sha256_is_not_downloaded_unless_allowed() {
        let (mock, api, base, dir) = download_fixture().await;
        let pull = "GET /api/v1/projects/demo/contracts/1.2.0/pull";
        mock.script(pull, 200, json!("AXIOM-ARTIFACT"));
        let path = dir.path().join("release.axiom");
        // One record route per case: the mock repeats the last scripted reply of a route.
        let records = [json!(null), json!(""), json!("   "), json!("abc"), json!("zz".repeat(32)), json!("0".repeat(63)), json!(7), json!("absent")];
        for (i, sha) in records.iter().enumerate() {
            let record = if sha == "absent" { json!({"version": "1.2.0"}) } else { json!({"version": "1.2.0", "sha256": sha}) };
            mock.script(&format!("GET /api/v1/projects/demo/contracts/r{i}"), 200, record);
            let error = download(&api, loopback(&base), "demo", &format!("r{i}"), Some(&path), STRICT).await.unwrap_err().to_string();
            assert!(error.contains("no valid SHA-256") && error.contains("--allow-unverified") && error.contains("nothing was downloaded"), "{sha}: {error}");
            assert!(!path.exists(), "{sha}");
        }
        assert!(mock.seen(pull).is_empty(), "refused before the artifact is requested");

        download(&api, loopback(&base), "demo", "r0", Some(&path), DownloadOptions { allow_unverified: true, ..STRICT }).await.unwrap();
        let bytes = json!("AXIOM-ARTIFACT").to_string().into_bytes();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(mock.seen(pull).len(), 1);
        // An upper-case digest in the record is still a valid one.
        mock.script("GET /api/v1/projects/demo/contracts/upper", 200, json!({"version": "1.2.0", "sha256": hex::encode_upper(Sha256::digest(&bytes))}));
        download(&api, loopback(&base), "demo", "upper", Some(&path), DownloadOptions { force: true, ..STRICT }).await.unwrap();
    }

    const SECRET_URL: &str = "https://deploy-bot:PASSWORD_CANARY@artifacts.example.com/r/a.axiom?X-Amz-Credential=CRED_CANARY&X-Amz-Signature=SIG_CANARY&token=TOKEN_CANARY#FRAG_CANARY";
    const CANARIES: [&str; 6] = ["deploy-bot", "PASSWORD_CANARY", "CRED_CANARY", "SIG_CANARY", "TOKEN_CANARY", "FRAG_CANARY"];

    fn assert_no_canary(text: &str) {
        for canary in CANARIES {
            assert!(!text.contains(canary), "{canary} leaked: {text}");
        }
    }

    #[test]
    fn source_urls_are_redacted_in_human_and_json_output_unless_shown() {
        let redacted = redact_url(SECRET_URL);
        assert_no_canary(&redacted);
        assert!(redacted.starts_with("https://REDACTED@artifacts.example.com/r/a.axiom?"), "{redacted}");
        assert!(redacted.contains("X-Amz-Signature=REDACTED") && redacted.contains("token=REDACTED"), "{redacted}");
        assert_eq!(redact_url("https://example.com/a.axiom"), "https://example.com/a.axiom", "nothing secret, nothing changed");
        assert_eq!(redact_url("https://example.com/a.axiom?sig"), "https://example.com/a.axiom?sig=REDACTED");
        assert_no_canary(&redact_url("deploy-bot:PASSWORD_CANARY@not a url"));
        assert_no_canary(&redact_url("mailto:deploy-bot:PASSWORD_CANARY@example.com"));
        assert_no_canary(&redact_url("https://deploy-bot@example.com/a"));

        let release = json!({"version": "1.1.0", "status": "signed", "source_type": "url", "source_url": SECRET_URL});
        // show, import (--json prints the whole reply) and list (an array of releases, or wrapped).
        for reply in [release.clone(), json!([release.clone()]), json!({"contracts": [release.clone()], "nested": {"source_url": SECRET_URL}})] {
            let shown_value = shown(false, reply.clone());
            assert_no_canary(&serde_json::to_string_pretty(&shown_value).unwrap());
            assert_eq!(shown(true, reply.clone()), reply, "--show-source-url shows the reply unchanged");
        }
        let human = render_release(&shown(false, release.clone())).join("\n");
        assert_no_canary(&human);
        assert!(human.contains("source url  https://REDACTED@artifacts.example.com"), "{human}");
        assert!(render_release(&shown(true, release)).join("\n").contains("PASSWORD_CANARY"));
        assert_eq!(shown(false, json!({"source_url": ""})), json!({"source_url": ""}));
    }

    #[tokio::test]
    async fn import_reads_the_url_from_a_file_or_stdin_instead_of_argv() {
        let h = harness().await;
        projects(&h.mock);
        let import = "POST /api/v1/projects/demo/contracts/import";
        h.mock.script(import, 201, json!({"version": "2.0.0", "status": "signed", "sha256": "ab", "source_url": SECRET_URL}));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("url.txt");
        std::fs::write(&file, format!("{SECRET_URL}\n")).unwrap();
        run(&h.api, &args(Action::Import { url: None, url_stdin: false, url_file: Some(file), version: None, rebuild: false })).await.unwrap();
        assert_eq!(h.mock.seen(import)[0].body, json!({"url": SECRET_URL, "version": "", "rebuild": false}));
        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, " \n").unwrap();
        assert!(run(&h.api, &args(Action::Import { url: None, url_stdin: false, url_file: Some(empty), version: None, rebuild: false })).await.unwrap_err().to_string().contains("empty"));
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            assert!(import_url(None, false, None).unwrap_err().to_string().contains("--url-file"));
        }

        use clap::Parser;
        assert!(Cli::try_parse_from(["releases", "import", "--url", "https://x/a.axiom", "--url-file", "u.txt"]).is_err());
        assert!(Cli::try_parse_from(["releases", "import", "--url-stdin", "--url-file", "u.txt"]).is_err());
        assert!(Cli::try_parse_from(["releases", "import", "https://x/a.axiom"]).is_err(), "no positional URL");
        let parsed = Cli::try_parse_from(["releases", "import", "--url-stdin", "--show-source-url"]).unwrap().releases;
        assert!(parsed.show_source_url && matches!(parsed.action, Action::Import { url: None, url_stdin: true, .. }));
        assert!(matches!(Cli::try_parse_from(["releases", "download", "1.0.0", "--allow-unverified"]).unwrap().releases.action, Action::Download { allow_unverified: true, force: false, .. }));
    }

    #[test]
    fn human_output_is_bounded_and_summarises() {
        let many: Vec<Value> = (0..60).map(|i| json!({"version": format!("0.{i}.0"), "status": "signed"})).collect();
        let lines = render_list(&Value::Array(many));
        assert_eq!(lines.len(), MAX_LINES + 1);
        assert!(lines.last().unwrap().contains("10 more; use --json"));
        assert!(render_list(&json!([]))[0].starts_with("No releases"));
        let diff = render_diff(&json!({"from_version": "1.1.0", "to_version": "1.2.0", "compatibility": "breaking", "status": "ready", "breaking_changes": 2, "risky_changes": 1, "compatible_changes": 0, "changes": [{"severity": "breaking", "operation": "remove", "path": "/users", "summary": "route removed"}], "highlights_truncated": true}));
        assert_eq!(diff[0], "Release 1.2.0 against 1.1.0: breaking (ready)");
        assert!(diff.iter().any(|l| l == "  breaking change: yes"));
        assert!(diff.iter().any(|l| l.contains("route removed")));
        let verification = render_verification(&json!({"status": "failed", "signature_verified": false, "failure_code": "AXV1", "failure_message": "bad\u{1b}[31m"}));
        assert!(verification.iter().any(|l| l.contains("AXV1: bad[31m")));
        assert!(render_semantic_items(&json!({"kind": "models", "total": 3, "next_cursor": "2", "items": [{"name": "User", "kind": "struct", "field_count": 4}]})).iter().any(|l| l == "Next page: --cursor 2"));
        assert!(render_backfill(&json!({"status": "already_queued", "version": "1.0.0", "message": "m"}))[0].contains("already queued"));
    }

    #[derive(clap::Parser, Debug)]
    struct Cli {
        #[command(flatten)]
        releases: ReleasesArgs,
    }

    #[test]
    fn the_command_tree_parses() {
        use clap::{CommandFactory, Parser};
        Cli::command().debug_assert();
        let parsed = Cli::try_parse_from(["releases", "diff", "1.2.0", "--against", "baseline", "--project", "demo", "--json"]).unwrap().releases;
        assert!(parsed.json && parsed.project.as_deref() == Some("demo"));
        assert!(matches!(parsed.action, Action::Diff { ref against, .. } if against.as_deref() == Some("baseline")));
        assert!(Cli::try_parse_from(["releases", "semantic-items", "1.2.0", "--kind", "views"]).is_err());
        assert!(Cli::try_parse_from(["releases", "semantic-items", "1.2.0", "--kind", "routes", "--limit", "101"]).is_err());
        assert!(matches!(Cli::try_parse_from(["releases", "download", "latest", "-o", "a.axiom", "--force"]).unwrap().releases.action, Action::Download { force: true, .. }));
    }

    #[test]
    fn import_body_matches_the_dashboard() {
        assert_eq!(import_body("https://x/a.axiom", None, false), json!({"url": "https://x/a.axiom", "version": "", "rebuild": false}));
        assert_eq!(import_body("https://x/a.axiom", Some(" 1.0.0 "), true), json!({"url": "https://x/a.axiom", "version": "1.0.0", "rebuild": true}));
        assert!(checked_version("1.0.0+build.1").is_ok());
        assert!(checked_version("../etc").is_err() && checked_version("a/b").is_err() && checked_version("").is_err());
    }
}
