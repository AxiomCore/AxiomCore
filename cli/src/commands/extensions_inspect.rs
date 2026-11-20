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

use anyhow::{bail, Result};
use axiom_lib::extension_ownership::*;
use axiom_lib::live_inspection::InspectorConnection;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    /// The message is made terminal-safe here, once (w3-d2 Fix 3): it may
    /// quote the Inspector (error bodies, serde errors naming its fields).
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: terminal_safe(&message.into(), MAX_MESSAGE_CHARS),
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
        if parse_walk_cursor(cursor).is_err() {
            eprintln!("axiom extensions inspect: malformed --cursor");
            return EXIT_USAGE;
        }
    }
    let connection = match options.connection.as_deref() {
        Some(path) => axiom_lib::live_inspection::read_connection_file(path),
        None => discover_connection(Path::new(".")),
    };
    let connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!(
                "axiom extensions inspect: {}",
                terminal_safe(&format!("{error:#}"), MAX_MESSAGE_CHARS)
            );
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
                // Already escaped and bounded by InspectFailure::new.
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

/// The Inspector connection for the workspace containing `start`: the
/// nearest directory (from `start` upwards) that has a
/// `.axiom/inspector/live-connection.json`. Each candidate is checked by the
/// one validation in `axiom_lib::live_inspection::find_project_connection`
/// (no symlink, owner-only file, strictly loopback endpoint); an
/// untrustworthy candidate is an error, never skipped.
pub fn discover_connection(start: &Path) -> Result<InspectorConnection> {
    let start = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut directory = start.as_path();
    loop {
        if let Some(connection) = axiom_lib::live_inspection::find_project_connection(directory)? {
            return Ok(connection);
        }
        match directory.parent() {
            Some(parent) if parent != directory => directory = parent,
            _ => bail!("no running Inspector found (.axiom/inspector/live-connection.json); start one with `axiom inspect serve` in the application workspace"),
        }
    }
}

/// Longest server-supplied message printed (characters, after escaping).
pub const MAX_MESSAGE_CHARS: usize = 512;
/// Largest Inspector response body read: a report page is under 1 MiB.
pub const MAX_REPORT_BYTES: usize = 2 * 1024 * 1024;
/// Largest Inspector error body read.
pub const MAX_ERROR_BYTES: usize = 64 * 1024;

/// Text from outside the process (Inspector bodies, endpoints, paths), made
/// safe for a terminal: every control character (C0, DEL, C1), and the
/// bidirectional and zero-width formatting characters, are written as
/// `\u{..}` escapes; backslashes are doubled; the result is cut to `max`
/// characters (with `…`). No escape sequence can reach the terminal.
pub fn terminal_safe(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    for c in text.chars() {
        let piece = match c {
            '\\' => "\\\\".to_string(),
            c if c.is_control()
                || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}') =>
            {
                format!("\\u{{{:x}}}", c as u32)
            }
            c => c.to_string(),
        };
        let width = piece.chars().count();
        if count + width > max {
            out.push('…');
            return out;
        }
        count += width;
        out.push_str(&piece);
    }
    out
}

/// Read at most `max` bytes of a response body; `None` if it is larger.
async fn bounded_body(mut response: reqwest::Response, max: usize) -> Result<Option<Vec<u8>>, ()> {
    if response.content_length().is_some_and(|length| length > max as u64) {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if body.len() + chunk.len() > max {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
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
    // The parsed loopback address, never the file's string (w3-d2 Fix 2).
    let endpoint = connection
        .loopback_endpoint()
        .map_err(|e| InspectFailure::new(EXIT_UNAVAILABLE, e.to_string()))?;
    let url = endpoint.url(&format!("/api/v1/live/ownership?{}", query_string(query)));
    let client = axiom_lib::live_inspection::inspector_client_builder(Duration::from_secs(10))
        .build()
        .map_err(|_| InspectFailure::new(EXIT_UNAVAILABLE, "HTTP client unavailable"))?;
    let response = client
        .get(&url)
        .header("x-axiom-inspector-token", &connection.token)
        .send()
        .await
        .map_err(|_| {
            InspectFailure::new(
                EXIT_UNAVAILABLE,
                format!("the Inspector at {} is not reachable", endpoint.origin()),
            )
        })?;
    let status = response.status().as_u16();
    let limit = if status == 200 {
        MAX_REPORT_BYTES
    } else {
        MAX_ERROR_BYTES
    };
    let body = bounded_body(response, limit)
        .await
        .map_err(|_| InspectFailure::new(EXIT_UNAVAILABLE, "the Inspector response was cut off"))?
        .ok_or_else(|| {
            InspectFailure::new(EXIT_UNAVAILABLE, "the Inspector response exceeds its bound")
        })?;
    let message = || {
        let text = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| format!("HTTP {status}"));
        text
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
        let first = ext.scopes.first().map_or(0, |scope| scope.id);
        let mut orphans: Vec<u64> = Vec::new();
        for scope in &ext.scopes {
            match scope.parent {
                // An older, listed parent: shown under it.
                Some(p) if p < scope.id && listed.contains_key(&p) => {
                    children.entry(Some(p)).or_default().push(scope.id)
                }
                // A parent on an earlier page (before this page's first
                // scope), or the root: shown at the top.
                Some(p) if p < first => children.entry(None).or_default().push(scope.id),
                None => children.entry(None).or_default().push(scope.id),
                // w3-d2 Fix 6: a parent that is missing from its range or not
                // older: listed under "orphaned" below, never dropped.
                Some(_) => orphans.push(scope.id),
            }
        }
        fn walk(
            out: &mut String,
            id: u64,
            depth: usize,
            listed: &HashMap<u64, &ScopeView>,
            children: &BTreeMap<Option<u64>, Vec<u64>>,
            shown: &mut HashSet<u64>,
        ) {
            if !shown.insert(id) {
                return;
            }
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
                    walk(out, *kid, depth + 1, listed, children, shown);
                }
            }
        }
        let mut shown = HashSet::new();
        if let Some(roots) = children.get(&None) {
            for root in roots {
                walk(&mut out, *root, 0, &listed, &children, &mut shown);
            }
        }
        // w3-d2 Fix 6: never omit a scope silently. A validated report has
        // none, but a scope unreachable from the roots is listed here.
        let unreached: Vec<u64> = ext
            .scopes
            .iter()
            .map(|scope| scope.id)
            .filter(|id| !shown.contains(id) && !orphans.contains(id))
            .collect();
        orphans.extend(unreached);
        if !orphans.is_empty() {
            let _ = writeln!(out, "  orphaned (parent missing or not older):");
            for id in orphans {
                walk(&mut out, id, 1, &listed, &children, &mut shown);
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
        let _ = writeln!(
            out,
            "\nmore: --cursor {}",
            terminal_safe(cursor, MAX_MESSAGE_CHARS)
        );
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
            serving_mode: None,
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
    fn connection_is_found_upwards_and_checked() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".axiom/inspector")).unwrap();
        std::fs::set_permissions(
            root.path().join(".axiom/inspector"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::create_dir_all(root.path().join("a/b")).unwrap();
        let path = root.path().join(".axiom/inspector/live-connection.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            discover_connection(&root.path().join("a/b")).is_err(),
            "an invalid connection is refused, not skipped"
        );
        let valid = serde_json::json!({"format": axiom_lib::live_inspection::LIVE_CONNECTION_FORMAT, "endpoint": "http://127.0.0.1:4242", "token": "0123456789abcdef0123456789abcdef", "graphRevision": "0".repeat(64), "processId": 1});
        std::fs::write(&path, valid.to_string()).unwrap();
        let found = discover_connection(&root.path().join("a/b")).unwrap();
        assert_eq!(found.loopback_endpoint().unwrap().address().port(), 4242);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(discover_connection(&root.path().join("a/b")).is_err());
    }

    /// w3-d2 Fix 6: a scope unreachable from a root is shown, never dropped.
    #[test]
    fn unreachable_scopes_are_listed_as_orphaned() {
        let mut report = fixture();
        let ids: Vec<u64> = report.extensions[0].scopes.iter().map(|s| s.id).collect();
        let last = report.extensions[0].scopes.len() - 1;
        let self_parent = report.extensions[0].scopes[last].id;
        report.extensions[0].scopes[last].parent = Some(self_parent);
        assert!(report.validate().is_err(), "the contract refuses it");
        let text = render_tree(&report);
        assert!(text.contains("orphaned (parent missing or not older):"), "{text}");
        for id in ids {
            assert!(text.contains(&format!("#{id} ")), "#{id} missing:\n{text}");
        }
        // A valid report has no orphaned heading (the golden is unchanged).
        assert!(!render_tree(&fixture()).contains("orphaned"));
    }

    /// w3-d2 Fix 3: a hostile "Inspector" on the loopback port cannot put
    /// control sequences on the terminal, flood it, or redirect the token.
    #[tokio::test]
    async fn hostile_inspector_replies_are_escaped_bounded_and_not_followed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let replies: Vec<Vec<u8>> = vec![
            {
                let body = r#"{"error":"\u001b]0;PWNED-TITLE\u0007\u001b[2J\u001b[31mred\r\nforged line"}"#;
                format!("HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).into_bytes()
            },
            {
                let body = format!("{{\"error\":\"{}\"}}", "\\u001b[31m".repeat(200_000));
                format!("HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).into_bytes()
            },
            format!("HTTP/1.1 302 Found\r\nlocation: http://{address}/stolen\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").into_bytes(),
        ];
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = std::sync::Arc::clone(&requests);
        tokio::spawn(async move {
            for reply in replies.into_iter().chain(std::iter::repeat(b"HTTP/1.1 500 x\r\ncontent-length: 0\r\n\r\n".to_vec())) {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let mut buffer = vec![0u8; 8192];
                let n = stream.read(&mut buffer).await.unwrap_or(0);
                seen.lock().unwrap().push(String::from_utf8_lossy(&buffer[..n]).lines().next().unwrap_or("").to_string());
                let _ = stream.write_all(&reply).await;
                let _ = stream.shutdown().await;
            }
        });
        let connection = InspectorConnection {
            format: axiom_lib::live_inspection::LIVE_CONNECTION_FORMAT.into(),
            endpoint: format!("http://{address}"),
            token: "0123456789abcdef0123456789abcdef".into(),
            graph_revision: "0".repeat(64),
            process_id: 1,
        };
        let query = OwnershipQuery::default();
        let unauthorized = fetch(&connection, &query).await.unwrap_err();
        assert_eq!(unauthorized.code, EXIT_UNAUTHENTICATED);
        assert!(!unauthorized.message.chars().any(char::is_control), "{:?}", unauthorized.message);
        assert!(unauthorized.message.contains("PWNED-TITLE"));
        // Escaped exactly once.
        assert!(unauthorized.message.contains("\\u{1b}]0;PWNED-TITLE\\u{7}"), "{}", unauthorized.message);
        assert!(!unauthorized.message.contains("\\\\u{"), "{}", unauthorized.message);
        let flood = fetch(&connection, &query).await.unwrap_err();
        assert_eq!(flood.code, EXIT_UNAVAILABLE);
        assert!(flood.message.chars().count() <= MAX_MESSAGE_CHARS + 1, "{}", flood.message.len());
        let redirect = fetch(&connection, &query).await.unwrap_err();
        assert_eq!(redirect.code, EXIT_UNAVAILABLE);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 3, "{requests:?}");
        assert!(requests.iter().all(|line| line.starts_with("GET /api/v1/live/ownership?")), "{requests:?}");
    }

    #[test]
    fn server_text_is_escaped_and_bounded() {
        let hostile = "\u{1b}]0;PWNED-TITLE\u{7}\u{1b}[2J\u{1b}[31mred\u{9b}31m\r\nfake line\u{202e}txt.exe\u{0}";
        let safe = terminal_safe(hostile, MAX_MESSAGE_CHARS);
        assert!(!safe.chars().any(|c| c.is_control()), "{safe:?}");
        assert!(!safe.contains('\u{202e}'));
        assert!(safe.contains("\\u{1b}]0;PWNED-TITLE\\u{7}"), "{safe}");
        assert_eq!(terminal_safe("plain text", 100), "plain text");
        assert_eq!(terminal_safe("a\\b", 100), "a\\\\b");
        let long = "x".repeat(10_000);
        let cut = terminal_safe(&long, MAX_MESSAGE_CHARS);
        assert_eq!(cut.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(cut.ends_with('…'));
        // An escape is never split by the bound.
        let cut = terminal_safe(&"\u{1b}".repeat(100), 10);
        assert!(!cut.chars().any(|c| c.is_control()));
        assert!(cut.chars().count() <= 11);
    }
}
