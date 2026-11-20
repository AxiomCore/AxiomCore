//! `axiom extensions inspect --ownership` (X10b): the live ownership view of
//! a running development host's extensions.
//!
//! The data travels over the existing authenticated development Inspector
//! channel: a development host publishes `axiom-extension-ownership-
//! publication/v1` documents to `POST /api/v1/live/ownership` of the running
//! Inspector (`axiom inspect serve`), using the session token from
//! `.axiom/inspector/live-connection.json`; this command reads
//! `GET /api/v1/live/ownership` with the same token. Hosts refuse inspection
//! in production profiles unless it is explicitly enabled
//! (`axiom_extension_host::inspect::InspectionPolicy`).
//!
//! Exit codes (stable, for scripts): 0 report printed; 2 invalid arguments
//! or query; 3 no Inspector connection or it is unreachable; 4 the Inspector
//! refused the session token; 5 no host has published ownership data, or the
//! `--extension` alias is unknown.

use anyhow::{bail, Context, Result};
use axiom_lib::extension_ownership::*;
use axiom_lib::live_inspection::InspectorConnection;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    path::{Path, PathBuf},
    time::Duration,
};

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_UNAVAILABLE: i32 = 3;
pub const EXIT_UNAUTHENTICATED: i32 = 4;
pub const EXIT_NO_DATA: i32 = 5;

/// Hosts kept by the Inspector (latest publication each).
pub const MAX_PUBLISHING_HOSTS: usize = 16;
/// A publication older than this is dropped on read (its host stopped).
pub const PUBLICATION_TTL_MS: u128 = 30_000;

pub struct OwnershipOptions {
    pub json: bool,
    pub watch: bool,
    pub schema: bool,
    pub extension: Option<String>,
    pub scope: Option<u64>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub connection: Option<PathBuf>,
    pub interval_ms: u64,
    pub count: Option<u64>,
}

/// A failed read, mapped to an exit code.
#[derive(Debug)]
pub struct InspectFailure {
    pub code: i32,
    pub message: String,
}

impl InspectFailure {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Run the command; returns the exit code (messages already printed).
pub async fn handle_ownership(options: OwnershipOptions) -> i32 {
    if options.schema {
        print!("{OWNERSHIP_SCHEMA}");
        return EXIT_OK;
    }
    let query = OwnershipQuery {
        extension: options.extension.clone(),
        scope: options.scope,
        cursor: options.cursor.clone(),
        limit: options.limit,
    };
    if let Err(error) = query.page_limit() {
        eprintln!("axiom extensions inspect: {error}");
        return EXIT_USAGE;
    }
    if let Some(cursor) = &query.cursor {
        if parse_cursor(cursor).is_err() {
            eprintln!("axiom extensions inspect: malformed --cursor");
            return EXIT_USAGE;
        }
    }
    let connection = match options
        .connection
        .clone()
        .map(Ok)
        .unwrap_or_else(|| find_connection(Path::new(".")))
    {
        Ok(path) => match read_connection(&path) {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("axiom extensions inspect: {error:#}");
                return EXIT_UNAVAILABLE;
            }
        },
        Err(error) => {
            eprintln!("axiom extensions inspect: {error:#}");
            return EXIT_UNAVAILABLE;
        }
    };
    let mut polls = 0u64;
    loop {
        match fetch(&connection, &query).await {
            Ok(report) => {
                if options.watch && !options.json {
                    // Clear the screen and redraw.
                    print!("\x1b[2J\x1b[H");
                }
                if options.json {
                    if options.watch {
                        println!("{}", serde_json::to_string(&report).unwrap_or_default());
                    } else {
                        print!("{}", render_json(&report));
                    }
                } else {
                    print!("{}", render_tree(&report));
                }
            }
            Err(failure) => {
                eprintln!("axiom extensions inspect: {}", failure.message);
                if !options.watch || failure.code != EXIT_NO_DATA {
                    return failure.code;
                }
            }
        }
        polls += 1;
        if !options.watch || options.count.is_some_and(|count| polls >= count) {
            return EXIT_OK;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(options.interval_ms.max(100))) => {}
            _ = tokio::signal::ctrl_c() => return EXIT_OK,
        }
    }
}

/// `.axiom/inspector/live-connection.json` in `start` or an ancestor.
pub fn find_connection(start: &Path) -> Result<PathBuf> {
    let start = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut directory = start.as_path();
    loop {
        let candidate = directory.join(".axiom/inspector/live-connection.json");
        if candidate.is_file() {
            return Ok(candidate);
        }
        match directory.parent() {
            Some(parent) if parent != directory => directory = parent,
            _ => bail!("no running Inspector found (.axiom/inspector/live-connection.json); start one with `axiom inspect serve` in the application workspace"),
        }
    }
}

fn read_connection(path: &Path) -> Result<InspectorConnection> {
    let connection: InspectorConnection = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))?;
    connection.validate()?;
    let loopback = connection.endpoint.starts_with("http://127.0.0.1:")
        || connection.endpoint.starts_with("http://[::1]:");
    if !loopback {
        bail!("the Inspector connection is not a loopback endpoint");
    }
    Ok(connection)
}

/// The query string for `GET /api/v1/live/ownership`.
pub fn query_string(query: &OwnershipQuery) -> String {
    let mut parts = Vec::new();
    if let Some(extension) = &query.extension {
        parts.push(format!("extension={}", urlencode(extension)));
    }
    if let Some(scope) = query.scope {
        parts.push(format!("scope={scope}"));
    }
    if let Some(cursor) = &query.cursor {
        parts.push(format!("cursor={}", urlencode(cursor)));
    }
    if let Some(limit) = query.limit {
        parts.push(format!("limit={limit}"));
    }
    parts.join("&")
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn fetch(
    connection: &InspectorConnection,
    query: &OwnershipQuery,
) -> Result<OwnershipReport, InspectFailure> {
    let url = format!(
        "{}/api/v1/live/ownership?{}",
        connection.endpoint,
        query_string(query)
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| InspectFailure::new(EXIT_UNAVAILABLE, format!("HTTP client: {e}")))?;
    let response = client
        .get(&url)
        .header("x-axiom-inspector-token", &connection.token)
        .send()
        .await
        .map_err(|_| {
            InspectFailure::new(
                EXIT_UNAVAILABLE,
                format!("the Inspector at {} is not reachable", connection.endpoint),
            )
        })?;
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .map_err(|_| InspectFailure::new(EXIT_UNAVAILABLE, "the Inspector response was cut off"))?;
    let message = || {
        serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| format!("HTTP {status}"))
    };
    match status {
        200 => {
            let report: OwnershipReport = serde_json::from_slice(&body).map_err(|e| {
                InspectFailure::new(EXIT_UNAVAILABLE, format!("malformed ownership report: {e}"))
            })?;
            report.validate().map_err(|e| {
                InspectFailure::new(EXIT_UNAVAILABLE, format!("invalid ownership report: {e}"))
            })?;
            Ok(report)
        }
        401 | 403 => Err(InspectFailure::new(EXIT_UNAUTHENTICATED, message())),
        400 => Err(InspectFailure::new(EXIT_USAGE, message())),
        404 => Err(InspectFailure::new(EXIT_NO_DATA, message())),
        _ => Err(InspectFailure::new(EXIT_UNAVAILABLE, message())),
    }
}

/// Stable JSON: the report as the contract serialises it (struct field
/// order, ascending ids), pretty-printed, newline-terminated.
pub fn render_json(report: &OwnershipReport) -> String {
    let mut text = serde_json::to_string_pretty(report).unwrap_or_default();
    text.push('\n');
    text
}

fn kind(kind: OwnershipScopeKind) -> &'static str {
    match kind {
        OwnershipScopeKind::App => "app",
        OwnershipScopeKind::Extension => "extension",
        OwnershipScopeKind::Instance => "instance",
        OwnershipScopeKind::Invocation => "invocation",
        OwnershipScopeKind::Round => "round",
    }
}

fn state(state: ScopeState) -> &'static str {
    match state {
        ScopeState::Open => "open",
        ScopeState::Closing => "closing",
        ScopeState::Closed => "closed",
    }
}

fn duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{}.{}s", ms / 1_000, (ms % 1_000) / 100)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1_000)
    }
}

fn reason(reason: OwnershipCloseReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The human view: one block per extension, the open tree indented by
/// parent (children in ascending id order), then the recently closed
/// scopes. Deterministic for a given report.
pub fn render_tree(report: &OwnershipReport) -> String {
    let mut out = String::new();
    let mode = match report.mode {
        InspectionMode::Development => "development",
        InspectionMode::Production => "production (explicitly enabled)",
    };
    let _ = writeln!(
        out,
        "Extension ownership · {mode} · {} extension(s) · {} scope(s)",
        report.extensions.len(),
        report.page.returned
    );
    if report.extensions.is_empty() {
        let _ = writeln!(out, "  (no extension matched)");
    }
    for ext in &report.extensions {
        let id = &ext.identity;
        let digest = id.module_sha256.get(..12).unwrap_or(&id.module_sha256);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} · {}@{} · instance {} · abi {} · {} {} · {}/{} · module {digest}",
            ext.alias,
            id.extension,
            id.extension_version,
            ext.instance_id,
            id.abi_version,
            id.engine,
            id.engine_version,
            id.profile,
            id.target
        );
        let _ = writeln!(
            out,
            "  open {} · closed {} · graph {}",
            ext.open_scopes, ext.closed_scopes, ext.extension_node
        );
        let listed: HashMap<u64, &ScopeView> = ext.scopes.iter().map(|s| (s.id, s)).collect();
        let mut children: BTreeMap<Option<u64>, Vec<u64>> = BTreeMap::new();
        for scope in &ext.scopes {
            // A scope whose parent is on another page is shown at the top.
            let parent = scope.parent.filter(|p| listed.contains_key(p));
            children.entry(parent).or_default().push(scope.id);
        }
        fn walk(
            out: &mut String,
            id: u64,
            depth: usize,
            listed: &HashMap<u64, &ScopeView>,
            children: &BTreeMap<Option<u64>, Vec<u64>>,
        ) {
            let scope = listed[&id];
            let mut line = format!(
                "{}#{} {}",
                "  ".repeat(depth + 1),
                scope.id,
                kind(scope.kind)
            );
            if scope.origin == OwnershipScopeOrigin::HostCapability {
                line.push_str(" (host)");
            }
            if let Some(round) = scope.round {
                let _ = write!(line, " {round}");
            }
            let _ = write!(
                line,
                " · {} · {}",
                state(scope.state),
                duration(scope.age_ms)
            );
            let h = scope.handles;
            if h.total > 0 {
                let _ = write!(
                    line,
                    " · handles {} (streams {}, subscriptions {}, secrets {})",
                    h.total, h.streams, h.subscriptions, h.secrets
                );
            }
            if scope.pending_host_operations > 0 {
                let _ = write!(line, " · pending {}", scope.pending_host_operations);
            }
            if scope.suspended_rounds > 0 {
                let _ = write!(line, " · suspended {}", scope.suspended_rounds);
            }
            if scope.pending_finalizers > 0 {
                let _ = write!(line, " · finalizers {}", scope.pending_finalizers);
            }
            if let Some(b) = scope.budgets {
                let _ = write!(
                    line,
                    " · budgets calls {} read {} write {} rounds {}",
                    b.max_host_calls, b.max_read_bytes, b.max_write_bytes, b.max_rounds
                );
            }
            if let Some(u) = scope.usage {
                let _ = write!(
                    line,
                    " · rounds {}/{} · calls {} · read {} · write {}",
                    u.rounds, u.max_rounds, u.host_calls, u.read_bytes, u.write_bytes
                );
                if let Some(fuel) = u.fuel_used {
                    let _ = write!(line, " · fuel {fuel}");
                }
                if let Some(left) = u.deadline_remaining_ms {
                    let _ = write!(line, " · deadline in {}", duration(left));
                }
            }
            if let Some(graph) = &scope.graph {
                if let Some(export) = &graph.export {
                    let _ = write!(line, " · {export}");
                }
                if !graph.effect_sites.is_empty() {
                    let sites: Vec<String> =
                        graph.effect_sites.iter().map(u32::to_string).collect();
                    let _ = write!(line, " · sites {}", sites.join(","));
                }
            }
            if scope.child_count as usize > scope.children.len() {
                let _ = write!(
                    line,
                    " · {} children ({} listed)",
                    scope.child_count,
                    scope.children.len()
                );
            }
            out.push_str(&line);
            out.push('\n');
            if let Some(kids) = children.get(&Some(id)) {
                for kid in kids {
                    walk(out, *kid, depth + 1, listed, children);
                }
            }
        }
        if let Some(roots) = children.get(&None) {
            for root in roots {
                walk(&mut out, *root, 0, &listed, &children);
            }
        }
        if !ext.recently_closed.is_empty() {
            let _ = writeln!(out, "  recently closed:");
            for closed in &ext.recently_closed {
                let r = closed.released;
                let _ = writeln!(
                    out,
                    "    #{} {} · {}{} · released {} · finalizers {}{}",
                    closed.id,
                    kind(closed.kind),
                    reason(closed.reason),
                    if closed.cascaded { " (cascaded)" } else { "" },
                    r.total,
                    closed.finalizers_run,
                    if closed.finalizer_panics > 0 {
                        format!(" ({} panicked)", closed.finalizer_panics)
                    } else {
                        String::new()
                    }
                );
            }
        }
    }
    if let Some(cursor) = &report.page.next_cursor {
        let _ = writeln!(out, "\nmore: --cursor {cursor}");
    }
    out
}

// ------------------------------------------------- Inspector server side

/// The Inspector's store of host publications.
#[derive(Default)]
pub struct OwnershipStore {
    publications: BTreeMap<String, (OwnershipPublication, u128)>,
}

impl OwnershipStore {
    /// Accept a validated publication (replacing the host's previous one).
    pub fn publish(&mut self, publication: OwnershipPublication, now_ms: u128) -> Result<()> {
        publication.validate()?;
        self.publications
            .retain(|_, (_, at)| now_ms.saturating_sub(*at) <= PUBLICATION_TTL_MS);
        if !self.publications.contains_key(&publication.host_id)
            && self.publications.len() >= MAX_PUBLISHING_HOSTS
        {
            // Evict the stalest host.
            if let Some(oldest) = self
                .publications
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone())
            {
                self.publications.remove(&oldest);
            }
        }
        self.publications
            .insert(publication.host_id.clone(), (publication, now_ms));
        Ok(())
    }

    /// The merged, filtered page for a query: `Ok(None)` when no live host
    /// has published.
    pub fn read(&self, query: &OwnershipQuery, now_ms: u128) -> Result<Option<OwnershipReport>> {
        let live: Vec<&OwnershipPublication> = self
            .publications
            .values()
            .filter(|(_, at)| now_ms.saturating_sub(*at) <= PUBLICATION_TTL_MS)
            .map(|(p, _)| p)
            .collect();
        if live.is_empty() {
            return Ok(None);
        }
        let merged = merge_publications(live, u64::try_from(now_ms).unwrap_or(u64::MAX));
        merged.select(query).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> OwnershipReport {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/extension-ownership/report.json"
        ))
        .unwrap()
    }

    #[test]
    fn human_output_matches_the_golden() {
        let golden = include_str!("../../tests/fixtures/extension-ownership/report.txt");
        let rendered = render_tree(&fixture());
        if std::env::var_os("AXIOM_BLESS_OWNERSHIP_GOLDEN").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/extension-ownership/report.txt"
                ),
                &rendered,
            )
            .unwrap();
        }
        assert_eq!(rendered, golden);
        // Deterministic: rendering twice is byte-identical.
        assert_eq!(render_tree(&fixture()), rendered);
    }

    #[test]
    fn json_output_matches_the_golden() {
        let golden = include_str!("../../tests/fixtures/extension-ownership/report.pretty.json");
        let rendered = render_json(&fixture());
        if std::env::var_os("AXIOM_BLESS_OWNERSHIP_GOLDEN").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/extension-ownership/report.pretty.json"
                ),
                &rendered,
            )
            .unwrap();
        }
        assert_eq!(rendered, golden);
        let back: OwnershipReport = serde_json::from_str(&rendered).unwrap();
        assert_eq!(back, fixture());
    }

    #[test]
    fn store_merges_pages_expires_and_bounds_hosts() {
        let mut store = OwnershipStore::default();
        assert!(store.read(&OwnershipQuery::default(), 0).unwrap().is_none());
        let publication = |host: &str| OwnershipPublication {
            format: OWNERSHIP_PUBLICATION_FORMAT.into(),
            host_id: host.into(),
            report: fixture(),
        };
        store.publish(publication("dev-a"), 1_000).unwrap();
        store.publish(publication("dev-b"), 1_000).unwrap();
        let page = store
            .read(
                &OwnershipQuery {
                    limit: Some(3),
                    ..Default::default()
                },
                1_500,
            )
            .unwrap()
            .unwrap();
        assert_eq!(page.page.returned, 3);
        assert!(page.page.next_cursor.is_some());
        for i in 0..40 {
            store
                .publish(publication(&format!("dev-{i}")), 2_000 + i)
                .unwrap();
        }
        assert!(store.publications.len() <= MAX_PUBLISHING_HOSTS);
        assert!(
            store
                .read(&OwnershipQuery::default(), 1_000_000)
                .unwrap()
                .is_none(),
            "stale publications expire"
        );
        let mut bad = publication("dev-x");
        bad.report.extensions[0].identity.engine = "secret value with spaces".into();
        assert!(store.publish(bad, 3_000).is_err());
    }

    #[test]
    fn query_strings_are_encoded() {
        let query = OwnershipQuery {
            extension: Some("pricing".into()),
            scope: Some(7),
            cursor: Some("0.12".into()),
            limit: Some(5),
        };
        assert_eq!(
            query_string(&query),
            "extension=pricing&scope=7&cursor=0.12&limit=5"
        );
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
    }

    #[test]
    fn connection_is_found_upwards() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".axiom/inspector")).unwrap();
        std::fs::create_dir_all(root.path().join("a/b")).unwrap();
        std::fs::write(
            root.path().join(".axiom/inspector/live-connection.json"),
            "{}",
        )
        .unwrap();
        let found = find_connection(&root.path().join("a/b")).unwrap();
        assert!(found.ends_with(".axiom/inspector/live-connection.json"));
        assert!(
            read_connection(&found).is_err(),
            "an invalid connection is refused"
        );
    }
}
