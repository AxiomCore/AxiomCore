//! Dashboard captures on the CLI side (data-tools/v1, "CLI: pull, migrate and version
//! skew"; confirmation C-S6): pull's three-way region comparison and the two mechanical
//! edits, written under the path rules (security S-5), and the label of capture releases.
use anyhow::{bail, Result};
use axiom_database::capture::region::{self, Region};
use axiom_database::developer::{capture::source_key, capture::migration_path, Release};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

/// At most this many bytes are written by one pull (security S-5).
const MAX_WRITE: usize = 8 * 1024 * 1024;

/// `captured in dashboard: <title> by <capturedBy>`, for a capture release.
pub(crate) fn label(release: &Release) -> Option<String> {
    release.revision.capture.as_ref().map(|c| format!("captured in dashboard: {} by {}", c.title, c.captured_by))
}

/// The source edits pull applies to the working sources: new file texts (`Some`) and a
/// removed migration source (`None`), by path relative to the project root.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) files: BTreeMap<String, Option<String>>,
    /// Schemas whose region was replaced, inserted or removed.
    pub(crate) replaced: Vec<String>,
    /// Schemas whose local region was kept (the developer's edit stands).
    pub(crate) kept: Vec<String>,
    pub(crate) version: Option<String>,
    pub(crate) removed_migration: Option<String>,
}

fn regions_of(sources: &BTreeMap<String, String>) -> Result<BTreeMap<String, Region>> {
    let bodies = region::schema_bodies(sources).map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;
    region::regions(sources, &bodies).map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))
}
fn text<'a>(sources: &'a BTreeMap<String, String>, r: Option<&Region>) -> Option<&'a str> {
    r.map(|r| &sources[&r.file][r.span.clone()])
}

/// Plans pull's source edits for the newest capture release of `chain` (the incoming
/// release). `catalog`: the newest chain release that was already in the local catalog
/// before this pull. Per schema, comparing the catalog, local and incoming regions:
/// local equals catalog, the local region becomes the incoming one (inserted at the end of
/// the schema body when absent); incoming equals catalog, the local region is kept;
/// otherwise pull refuses with both diffs and writes nothing.
pub(crate) fn plan(chain: &[Release], catalog: Option<&Release>, entry: &str, local: &BTreeMap<String, String>) -> Result<Option<(Release, Plan)>> {
    let Some(index) = chain.iter().rposition(|r| r.revision.capture.is_some()) else { return Ok(None) };
    let incoming = &chain[index];
    let empty = BTreeMap::new();
    let catalog_sources = catalog.map_or(&empty, |c| &c.revision.sources);
    let (c_regions, l_regions, i_regions) = (regions_of(catalog_sources)?, regions_of(local)?, regions_of(&incoming.revision.sources)?);
    let bodies = region::schema_bodies(local).map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;
    let schemas: BTreeSet<&String> = c_regions.keys().chain(l_regions.keys()).chain(i_regions.keys()).collect();
    let mut plan = Plan::default();
    let mut edits: BTreeMap<String, Vec<(std::ops::Range<usize>, String)>> = BTreeMap::new();
    let mut conflicts = vec![];
    for schema in schemas {
        let (c, l, i) = (text(catalog_sources, c_regions.get(schema)), text(local, l_regions.get(schema)), text(&incoming.revision.sources, i_regions.get(schema)));
        if l == i {
            continue;
        }
        if l == c {
            let body = bodies.get(schema).ok_or_else(|| anyhow::anyhow!("ADB1142: schema `{schema}` is not declared in the working sources; nothing was written"))?;
            let (span, replacement) = match l_regions.get(schema) {
                Some(r) => (r.span.clone(), i.unwrap_or("").to_string()),
                None => region::edit(&local[&body.file], body, None, i.unwrap_or("")),
            };
            edits.entry(body.file.clone()).or_default().push((span, replacement));
            plan.replaced.push(schema.clone());
        } else if i == c {
            plan.kept.push(schema.clone());
        } else {
            conflicts.push(format!(
                "schema {schema}:\n--- catalog to local\n{}\n--- catalog to incoming\n{}",
                diff(c.unwrap_or(""), l.unwrap_or("")),
                diff(c.unwrap_or(""), i.unwrap_or(""))
            ));
        }
    }
    if !conflicts.is_empty() {
        bail!("ADB1104: the local dashboard region and the incoming capture {} both changed; nothing was written. Reconcile by hand:\n{}", incoming.revision.identity.version, conflicts.join("\n"));
    }
    // Mechanical edits: the version, and the removed migration declaration with its file.
    let entry_text = local.get(entry).ok_or_else(|| anyhow::anyhow!("ADB1101: the database entry is not in the working sources"))?;
    let declarations = acore::database::syntax::parse(entry_text)?;
    if let Some(project) = declarations.iter().find(|d| d.kind == "project") {
        if let Some(arg) = project.args.iter().find(|a| a.name.as_deref() == Some("version")) {
            if let acore::database::syntax::Value::String(local_version) = &arg.value.value {
                let newer = incoming.revision.identity.version.parse::<axiom_lib::database::version::Version>()? > local_version.parse::<axiom_lib::database::version::Version>()?;
                if newer {
                    edits.entry(entry.to_string()).or_default().push((arg.value.span.clone(), serde_json::to_string(&incoming.revision.identity.version)?));
                    plan.version = Some(incoming.revision.identity.version.clone());
                }
            }
        }
    }
    let previous = index.checked_sub(1).map(|i| &chain[i]);
    let removed = previous.and_then(|p| migration_path(&p.revision.entry, p.revision.sources.get(&p.revision.entry).map(String::as_str)));
    if let (Some(path), None) = (removed, migration_path(&incoming.revision.entry, incoming.revision.sources.get(&incoming.revision.entry).map(String::as_str))) {
        if let Some(d) = declarations.iter().find(|d| d.kind == "migration") {
            if migration_path(entry, Some(entry_text)).as_deref() == Some(path.as_str()) {
                edits.entry(entry.to_string()).or_default().push((axiom_database::capture::whole_lines(entry_text, d.span.clone()), String::new()));
                if local.contains_key(&path) {
                    if !source_key(&path, Some(&path)) {
                        bail!("ADB1101: the migration source path `{path}` is outside the source-key rule; nothing was written");
                    }
                    plan.files.insert(path.clone(), None);
                }
                plan.removed_migration = Some(path);
            }
        }
    }
    for (file, mut list) in edits {
        list.sort_by(|a, b| b.0.start.cmp(&a.0.start));
        let mut out = local[&file].clone();
        let mut floor = usize::MAX;
        for (span, with) in list {
            if span.end > floor {
                bail!("ADB1142: pull edits overlap in {file}; nothing was written");
            }
            floor = span.start;
            out.replace_range(span, &with);
        }
        plan.files.insert(file, Some(out));
    }
    Ok(Some((incoming.clone(), plan)))
}

/// A line diff (`-`/`+`/` ` prefixes), small and dependency-free.
fn diff(a: &str, b: &str) -> String {
    let (x, y): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let mut table = vec![vec![0usize; y.len() + 1]; x.len() + 1];
    for i in (0..x.len()).rev() {
        for j in (0..y.len()).rev() {
            table[i][j] = if x[i] == y[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < x.len() || j < y.len() {
        if i < x.len() && j < y.len() && x[i] == y[j] {
            out.push(format!("  {}", x[i]));
            i += 1;
            j += 1;
        } else if j < y.len() && (i == x.len() || table[i][j + 1] >= table[i + 1][j]) {
            out.push(format!("+ {}", y[j]));
            j += 1;
        } else {
            out.push(format!("- {}", x[i]));
            i += 1;
        }
    }
    out.join("\n")
}

/// Writes a plan under the path rules (security S-5): every path is a valid source key that
/// already exists in the project as a regular file (not a symlink) inside the canonical
/// root; originals are copied to `.axiom/database/pulled/<version>/` first; new texts are
/// written with `create_new` into a fresh directory and renamed over their targets; at most
/// 8 MiB in total. Returns the backup directory.
pub(crate) fn write(root: &Path, version: &str, plan: &Plan, migration: Option<&str>) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let total: usize = plan.files.values().flatten().map(String::len).sum();
    if total > MAX_WRITE {
        bail!("ADB1101: pull would write more than 8 MiB of source; nothing was written");
    }
    let mut targets = vec![];
    for path in plan.files.keys() {
        if !source_key(path, migration) {
            bail!("ADB1101: source key `{path}` is outside the source-key rule; nothing was written");
        }
        let target = root.join(path);
        let meta = std::fs::symlink_metadata(&target).map_err(|_| anyhow::anyhow!("ADB1101: pull writes only existing project files; `{path}` does not exist; nothing was written"))?;
        if !meta.file_type().is_file() || !target.canonicalize()?.starts_with(&root) {
            bail!("ADB1101: `{path}` is not a regular file inside the project; nothing was written");
        }
        targets.push((path.clone(), target));
    }
    let database = root.join(".axiom/database");
    std::fs::create_dir_all(&database)?;
    let mut backup = database.join("pulled").join(version);
    let mut n = 1;
    while backup.exists() {
        n += 1;
        backup = database.join("pulled").join(format!("{version}.{n}"));
    }
    for (path, target) in &targets {
        let copy = backup.join(path);
        std::fs::create_dir_all(copy.parent().unwrap())?;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&copy)?;
        file.write_all(&std::fs::read(target)?)?;
        file.sync_all()?;
    }
    let staging = tempfile::Builder::new().prefix("pull-").tempdir_in(&database)?;
    let mut staged = vec![];
    for (index, (path, target)) in targets.iter().enumerate() {
        if let Some(Some(text)) = plan.files.get(path) {
            let file_path = staging.path().join(index.to_string());
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&file_path)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            staged.push((file_path, target.clone()));
        }
    }
    for (from, to) in staged {
        // Checked again just before the rename: still a regular file inside the root.
        if !std::fs::symlink_metadata(&to)?.file_type().is_file() || !to.canonicalize()?.starts_with(&root) {
            bail!("ADB1101: `{}` changed during pull", to.display());
        }
        std::fs::rename(from, to)?;
    }
    for (path, target) in &targets {
        if plan.files.get(path) == Some(&None) {
            std::fs::remove_file(target)?;
        }
    }
    Ok(backup)
}
