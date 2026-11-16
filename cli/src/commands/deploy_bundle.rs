//! Local, offline half of `axiom deploy`: walk a build folder, apply the path rules, hash
//! every file and build the canonical manifest whose SHA-256 the upload is keyed by
//! (`axiom-static-bundle/v1` for a frontend, internal-docs planning/deployments/
//! B-W0-FRONTEND-AND-PACKAGING.md §3). A backend's manifest is the runtime's own `input.json`
//! (`axiom server package --output-input`, `axiom-container-input/v1`), uploaded verbatim with
//! the files it lists (contract amendment A4.2).
//!
//! Nothing here signs in or sends a request: `axiom deploy` runs it before registration,
//! sign-in or any network call, so a bad folder fails fast with a fix to apply.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Server limits (CONTRACT `AXDEPLOY_INVALID_UPLOAD`), checked locally first.
pub const MAX_FILES: usize = 10_000;
pub const MAX_BYTES: u64 = 1 << 30;

pub const STATIC_SCHEMA: &str = "axiom-static-bundle/v1";
pub const CONTAINER_SCHEMA: &str = "axiom-container-input/v1";

/// One content file of a bundle.
#[derive(Clone, Debug)]
pub struct File {
    /// Manifest path: one leading slash for a frontend; the `input.json` name (no leading
    /// slash) for a backend.
    pub path: String,
    pub source: PathBuf,
    pub sha256: String,
    pub size: u64,
    /// Frontend only.
    pub content_type: Option<&'static str>,
}

/// A checked folder and its canonical manifest.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub root: PathBuf,
    /// Sorted by path.
    pub files: Vec<File>,
    pub manifest: Vec<u8>,
    pub manifest_sha256: String,
    /// Frontend only.
    pub spa_fallback: Option<bool>,
}

impl Bundle {
    /// Bytes of the content files (the manifest is not counted, A4.1).
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
    /// Exact byte length of the manifest object (`manifest_bytes`, A4.1).
    pub fn manifest_bytes(&self) -> u64 {
        self.manifest.len() as u64
    }
    /// `files[]` of the create-upload body (one per path, manifest order).
    pub fn entries(&self) -> Vec<axiom_cloud::deployments::bodies::FileEntry> {
        self.files
            .iter()
            .map(|f| axiom_cloud::deployments::bodies::FileEntry { sha256: f.sha256.clone(), size: f.size })
            .collect()
    }
}

/// SPA fallback choice for a frontend bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spa {
    On,
    Off,
    /// On unless the site ships its own `/404.html`.
    Auto,
}

/// Content types by extension (lowercase). Every type is on the server's allowlist and fits
/// its extension rules (axiom-backend internal/adapters/hosting/manifest.go checkContentType);
/// a file with another extension is refused before any request, since the server would
/// refuse it. A name without an extension is `text/plain`, except
/// `/.well-known/apple-app-site-association`, which is JSON. The dashboard uses the same table.
pub const CONTENT_TYPES: &[(&str, &str)] = &[
    ("html", "text/html; charset=utf-8"),
    ("htm", "text/html; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("cjs", "text/javascript; charset=utf-8"),
    ("json", "application/json"),
    ("webmanifest", "application/manifest+json"),
    ("txt", "text/plain; charset=utf-8"),
    ("md", "text/plain; charset=utf-8"),
    ("csv", "text/plain; charset=utf-8"),
    ("xml", "application/xml"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("ico", "image/x-icon"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("wasm", "application/wasm"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("ogv", "video/ogg"),
    ("mp3", "audio/mpeg"),
    ("ogg", "audio/ogg"),
    ("wav", "audio/wav"),
    ("aac", "audio/aac"),
    ("m4a", "audio/mp4"),
];

/// The content type of a manifest path, or `None` for an extension Axiom hosting does not
/// serve. The extension is what follows the last `.` of the name after its first character;
/// a name without one (`README`, `/.nojekyll`) is `text/plain`, which the server allows on
/// any name.
pub fn content_type(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) {
        None if path == "/.well-known/apple-app-site-association" => Some("application/json"),
        None => Some("text/plain; charset=utf-8"),
        Some((_, ext)) => {
            let ext = ext.to_ascii_lowercase();
            CONTENT_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t)
        }
    }
}

/// Operating-system clutter that is never part of a build and is left out silently.
pub fn is_os_clutter(name: &str) -> bool {
    name.eq_ignore_ascii_case(".DS_Store") || name.eq_ignore_ascii_case("Thumbs.db")
}

/// Go `strings.ToLower` for the server's name patterns, which are all ASCII: a character
/// lowercases to an ASCII letter only when it is an ASCII capital, U+0130 (`İ` → `i`) or the
/// Kelvin sign U+212A (→ `k`); every other character can never match a pattern either way.
fn go_lower(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{130}' => 'i',
            '\u{212A}' => 'k',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// The server's site-path rules exactly (axiom-backend internal/adapters/hosting/manifest.go
/// `ValidSitePath`, `forbiddenSegment`, `forbiddenName`; contract amendment A4.5): `None` when
/// the server accepts `path`, else what to do. The shared fixture
/// `tests/fixtures/deployments-path-rules.json` pins both this and the dashboard copy.
pub fn site_path_problem(path: &str) -> Option<&'static str> {
    use unicode_normalization::is_nfc;
    const SYNTAX: &str = "file and folder names cannot contain backslashes, %, ?, # or control characters, or be empty, . or ... Rename it.";
    if path.len() < 2 || !path.starts_with('/') {
        return Some(SYNTAX);
    }
    if path.len() > 1024 {
        return Some("the path is longer than 1024 bytes. Shorten the file or folder names.");
    }
    if !is_nfc(path) {
        return Some("the name is not Unicode-normalised (NFC). Rename it.");
    }
    if path.chars().any(|c| matches!(c, '\\' | '%' | '?' | '#' | '\u{FFFD}') || c.is_control()) {
        return Some(SYNTAX);
    }
    let segments: Vec<&str> = path[1..].split('/').collect();
    if segments.len() > 32 {
        return Some("the path is nested more than 32 folders deep. Flatten the build output.");
    }
    for (i, s) in segments.iter().enumerate() {
        if s.is_empty() || *s == "." || *s == ".." {
            return Some(SYNTAX);
        }
        if matches!(go_lower(s).as_str(), ".git" | ".aws" | ".ssh" | ".docker" | ".gnupg" | ".kube")
            || (i > 0 && segments[i - 1].eq_ignore_ascii_case(".config") && s.eq_ignore_ascii_case("gcloud"))
        {
            return Some("version-control and credential folders (.git, .aws, .ssh, .docker, .gnupg, .kube, .config/gcloud) are never published. Remove it from the build output.");
        }
    }
    if segments[0].eq_ignore_ascii_case("_axiom") {
        return Some("the /_axiom/ path is reserved for the platform. Rename it in your build.");
    }
    let base = go_lower(segments[segments.len() - 1]);
    if matches!(base.as_str(), "_worker.js" | "_headers" | "_redirects" | "_routes.json") {
        return Some("this provider control file is not used by Axiom hosting. Remove it from the build; single-page routing is the --spa flag, and headers/redirects are not configurable yet.");
    }
    if base == ".env" || base.starts_with(".env.") {
        return Some("environment files can hold secrets and are never published. Remove it from the build output.");
    }
    if base.ends_with(".map") {
        return Some("source maps are not published (they expose your original source). Remove the *.map files from the build output, for example by turning off production source maps in your bundler.");
    }
    let credential = matches!(
        base.as_str(),
        ".npmrc" | ".yarnrc" | ".yarnrc.yml" | ".netrc" | ".git-credentials" | ".htpasswd" | ".htaccess" | ".pgpass" | ".dockercfg" | "credentials" | "credentials.json" | "secrets.json"
    ) || (base.starts_with("service-account") && base.ends_with(".json"))
        || [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".kdbx"].iter().any(|e| base.ends_with(e))
        || ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"].iter().any(|k| base.starts_with(k));
    if credential {
        return Some("this looks like a credential, private key or server configuration file and is never published. Remove it from the build output.");
    }
    None
}

/// The first existing build folder among `dist/`, `build/`, `out/` under `cwd`.
pub fn default_frontend_dir(cwd: &Path) -> Result<PathBuf> {
    ["dist", "build", "out"]
        .iter()
        .map(|d| cwd.join(d))
        .find(|p| p.is_dir())
        .with_context(|| "No build output found (looked for dist/, build/ and out/). Build your app first, or pass the folder: axiom deploy frontend ./dist".to_string())
}

/// Check a static site folder and build its `axiom-static-bundle/v1` manifest.
pub fn frontend(root: &Path, spa: Spa) -> Result<Bundle> {
    let shown = root.display().to_string();
    let meta = std::fs::symlink_metadata(root).with_context(|| format!("{shown} does not exist. Build your app first, or pass the folder: axiom deploy frontend ./dist"))?;
    if !meta.is_dir() {
        bail!("{shown} is not a folder. Pass the build output folder: axiom deploy frontend ./dist");
    }
    let mut files = Vec::new();
    walk(root, root, "", &mut files, &frontend_rule)?;
    sort_unique(&mut files)?;
    if !files.iter().any(|f| f.path == "/index.html") {
        bail!("{shown} has no index.html at its top level. Pass the folder that contains index.html (your build output), for example: axiom deploy frontend ./dist");
    }
    for f in &mut files {
        f.content_type = content_type(&f.path);
    }
    limits(&files, &shown)?;
    let spa_fallback = match spa {
        Spa::On => true,
        Spa::Off => false,
        Spa::Auto => !files.iter().any(|f| f.path == "/404.html"),
    };
    #[derive(Serialize)]
    struct Entry<'a> {
        path: &'a str,
        sha256: &'a str,
        size: u64,
        content_type: &'a str,
    }
    #[derive(Serialize)]
    struct Manifest<'a> {
        schema: &'a str,
        entrypoint: &'a str,
        spa_fallback: bool,
        files: Vec<Entry<'a>>,
    }
    let manifest = serde_json::to_vec(&Manifest {
        schema: STATIC_SCHEMA,
        entrypoint: "/index.html",
        spa_fallback,
        files: files
            .iter()
            .map(|f| Entry { path: &f.path, sha256: &f.sha256, size: f.size, content_type: f.content_type.unwrap_or_default() })
            .collect(),
    })?;
    Ok(finish(root, files, manifest, Some(spa_fallback)))
}

/// Read a package input folder (`axiom server package … --output-input DIR`): its
/// `input.json` (`axiom-container-input/v1`, written by the runtime) is the manifest,
/// uploaded byte for byte, and the content files are exactly the files it lists, each checked
/// against its listed digest and size (contract amendment A4.2). Other files in the folder are
/// not uploaded.
pub fn backend(root: &Path) -> Result<Bundle> {
    #[derive(serde::Deserialize)]
    struct Listed {
        sha256: String,
        bytes: u64,
        #[serde(default)]
        executable: bool,
    }
    #[derive(serde::Deserialize)]
    struct Input {
        format: String,
        files: std::collections::BTreeMap<String, Listed>,
    }
    let shown = root.display().to_string();
    let redo = "Create it again with `axiom server package … --output-input DIR`, or leave out --from-input to package this project";
    let meta = std::fs::symlink_metadata(root).with_context(|| format!("{shown} does not exist. {redo}."))?;
    if !meta.is_dir() {
        bail!("{shown} is not a folder. Pass the folder written by `axiom server package … --output-input DIR`.");
    }
    let manifest_path = root.join("input.json");
    let manifest_meta = std::fs::symlink_metadata(&manifest_path).ok().filter(|m| m.is_file());
    let Some(manifest_meta) = manifest_meta else {
        bail!("{shown} has no input.json, so it is not a package input. {redo}.");
    };
    if manifest_meta.len() > MAX_INPUT_JSON_BYTES {
        bail!("{shown}/input.json is larger than 16 MiB. {redo}.");
    }
    let manifest = std::fs::read(&manifest_path).with_context(|| format!("cannot read {shown}/input.json"))?;
    let input: Input = serde_json::from_slice(&manifest).with_context(|| format!("{shown}/input.json is not a package input manifest. {redo}."))?;
    if input.format != CONTAINER_SCHEMA {
        bail!("{shown}/input.json has format {:?}, not {CONTAINER_SCHEMA}. Update the Axiom CLI and package again.", input.format);
    }
    if input.files.is_empty() || input.files.len() > MAX_INPUT_FILES {
        bail!("{shown}/input.json lists {} files; a backend input holds 1 to {MAX_INPUT_FILES}. {redo}.", input.files.len());
    }
    let mut files = Vec::with_capacity(input.files.len());
    let mut total = 0u64;
    for (name, listed) in &input.files {
        if !portable_input_name(name) || name == "input.json" || listed.executable {
            bail!("{shown}/input.json lists {name:?}, which a package input cannot hold. {redo}.");
        }
        if listed.bytes > MAX_INPUT_FILE_BYTES {
            bail!("{name}: {} is larger than the 16 MiB a backend input file may hold.", human_bytes(listed.bytes));
        }
        total += listed.bytes;
        if total > MAX_INPUT_TOTAL_BYTES {
            bail!("{shown} lists more than 512 MiB of files; a backend input holds at most 512 MiB.");
        }
        let source = root.join(name);
        let ok = std::fs::symlink_metadata(&source).map(|m| m.is_file()).unwrap_or(false);
        if !ok {
            bail!("{shown}/{name} is listed in input.json but is missing (or not a regular file). {redo}.");
        }
        let bytes = std::fs::read(&source).with_context(|| format!("cannot read {shown}/{name}"))?;
        let sha256 = sha256_hex(&bytes);
        if sha256 != listed.sha256 || bytes.len() as u64 != listed.bytes {
            bail!("{shown}/{name} does not match its digest in input.json (changed after packaging). {redo}.");
        }
        files.push(File { path: name.clone(), source, sha256, size: listed.bytes, content_type: None });
    }
    Ok(finish(root, files, manifest, None))
}

/// Package input bounds (axiom-backend hosting `ParseContainerInput`, axiom-runtime release.rs).
pub const MAX_INPUT_JSON_BYTES: u64 = 16 << 20;
pub const MAX_INPUT_FILES: usize = 256;
pub const MAX_INPUT_FILE_BYTES: u64 = 16 << 20;
pub const MAX_INPUT_TOTAL_BYTES: u64 = 512 << 20;

/// A relative, `/`-separated input name (axiom-runtime release.rs `portable()`, axiom-backend
/// `PortableInputName`).
fn portable_input_name(p: &str) -> bool {
    if p.is_empty() || p.len() > 512 || p.starts_with('/') || p.contains('\\') || p.chars().any(char::is_control) {
        return false;
    }
    let segments: Vec<&str> = p.split('/').collect();
    segments.len() <= 8 && segments.iter().all(|s| !s.is_empty() && *s != "." && *s != "..")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn finish(root: &Path, files: Vec<File>, manifest: Vec<u8>, spa_fallback: Option<bool>) -> Bundle {
    let manifest_sha256 = sha256_hex(&manifest);
    Bundle { root: root.to_path_buf(), files, manifest, manifest_sha256, spa_fallback }
}

fn limits(files: &[File], shown: &str) -> Result<()> {
    if files.len() > MAX_FILES {
        bail!("{shown} has {} files; a deployment holds at most {MAX_FILES}. Remove files you do not serve.", files.len());
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    if total > MAX_BYTES {
        bail!("{shown} holds {}; a deployment holds at most 1 GiB. Remove files you do not serve.", human_bytes(total));
    }
    Ok(())
}

enum Visit {
    Keep,
    Skip,
}

/// Frontend walk rule for one entry: OS clutter files are left out; every other file must
/// pass the server's path rules ([`site_path_problem`]) and have a served content type.
/// `rel` is the manifest path (NFC), `name` its last segment. Folders are not judged on
/// their own: the server's rules apply to the paths of the files under them.
fn frontend_rule(rel: &str, name: &str, is_dir: bool) -> Result<Visit> {
    if is_dir {
        return Ok(Visit::Keep);
    }
    if is_os_clutter(name) {
        return Ok(Visit::Skip);
    }
    if let Some(problem) = site_path_problem(rel) {
        bail!("{rel}: {problem}");
    }
    if content_type(rel).is_none() {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        bail!("{rel}: .{ext} files are not served by Axiom hosting yet (served: web pages, CSS, JavaScript, JSON, XML, text, images, fonts, WebAssembly, audio and video). Remove it from the build output.");
    }
    Ok(Visit::Keep)
}

type Rule<'a> = &'a dyn Fn(&str, &str, bool) -> Result<Visit>;

/// Collect the regular files under `root`. Manifest paths are built from NFC-normalised
/// names (`rel_dir` is the normalised parent path, "" at the root); two entries that
/// normalise to the same path are refused.
fn walk(root: &Path, dir: &Path, rel_dir: &str, out: &mut Vec<File>, rule: Rule<'_>) -> Result<()> {
    use unicode_normalization::UnicodeNormalization;
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("cannot read {}", dir.display()))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let shown = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        let name = name.to_str().with_context(|| format!("{shown}: file names must be UTF-8. Rename it."))?.nfc().collect::<String>();
        if name.contains('\\') || name.contains('\0') || name.chars().any(char::is_control) || name == "." || name == ".." {
            bail!("{shown}: file names cannot contain backslashes or control characters. Rename it.");
        }
        let rel = format!("{rel_dir}/{name}");
        let meta = std::fs::symlink_metadata(&path).with_context(|| format!("cannot read {shown}"))?;
        if meta.file_type().is_symlink() {
            bail!("{rel}: symbolic links are not deployed. Replace it with the real file (or folder) in the build output.");
        }
        if let Visit::Skip = rule(&rel, &name, meta.is_dir())? {
            continue;
        }
        if meta.is_dir() {
            walk(root, &path, &rel, out, rule)?;
        } else if meta.is_file() {
            let bytes = std::fs::read(&path).with_context(|| format!("cannot read {shown}"))?;
            out.push(File { path: rel, source: path, sha256: sha256_hex(&bytes), size: bytes.len() as u64, content_type: None });
            if out.len() > MAX_FILES {
                bail!("{} has more than {MAX_FILES} files; a deployment holds at most {MAX_FILES}. Remove files you do not serve.", root.display());
            }
        } else {
            bail!("{rel}: only regular files and folders are deployed. Remove it from the build output.");
        }
    }
    Ok(())
}

/// Sort by manifest path (byte order of the UTF-8 NFC path) and refuse duplicates, which
/// only arise from names that differ before Unicode normalisation.
fn sort_unique(files: &mut [File]) -> Result<()> {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(w) = files.windows(2).find(|w| w[0].path == w[1].path) {
        bail!("{}: two files have this name once Unicode-normalised (NFC). Rename one of them.", w[0].path);
    }
    Ok(())
}

pub fn human_bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.1} KB", f / K)
    } else if f < K * K * K {
        format!("{:.1} MB", f / K / K)
    } else {
        format!("{:.2} GB", f / K / K / K)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        dir
    }
    fn err(files: &[(&str, &str)]) -> String {
        let d = site(files);
        frontend(d.path(), Spa::Auto).unwrap_err().to_string()
    }

    #[test]
    fn manifest_is_canonical_sorted_and_deterministic() {
        let d = site(&[("assets/app.js", "console.log(1)"), ("index.html", "<h1>hi</h1>"), (".DS_Store", "x"), ("img/Thumbs.db", "x"), (".well-known/security.txt", "c")]);
        let b = frontend(d.path(), Spa::Auto).unwrap();
        let paths: Vec<_> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/.well-known/security.txt", "/assets/app.js", "/index.html"]);
        let text = String::from_utf8(b.manifest.clone()).unwrap();
        let html = sha256_hex(b"<h1>hi</h1>");
        assert!(text.starts_with(r#"{"schema":"axiom-static-bundle/v1","entrypoint":"/index.html","spa_fallback":true,"files":[{"path":"/.well-known/security.txt""#), "{text}");
        assert!(text.ends_with(&format!(r#"{{"path":"/index.html","sha256":"{html}","size":11,"content_type":"text/html; charset=utf-8"}}]}}"#)), "{text}");
        assert_eq!(b.manifest_sha256, sha256_hex(&b.manifest));
        assert_eq!(frontend(d.path(), Spa::Auto).unwrap().manifest, b.manifest);
        assert_eq!(b.total_bytes(), 1 + 14 + 11);
    }

    #[test]
    fn spa_defaults_on_unless_the_site_has_a_404_page() {
        let d = site(&[("index.html", "a")]);
        assert_eq!(frontend(d.path(), Spa::Auto).unwrap().spa_fallback, Some(true));
        assert_eq!(frontend(d.path(), Spa::Off).unwrap().spa_fallback, Some(false));
        let d = site(&[("index.html", "a"), ("404.html", "b")]);
        assert_eq!(frontend(d.path(), Spa::Auto).unwrap().spa_fallback, Some(false));
        assert_eq!(frontend(d.path(), Spa::On).unwrap().spa_fallback, Some(true));
    }

    #[test]
    fn path_rules_reject_with_guidance() {
        assert!(err(&[("about.html", "a")]).contains("no index.html"));
        assert!(err(&[("index.html", "a"), ("_headers", "x")]).contains("provider control file"));
        assert!(err(&[("index.html", "a"), ("_redirects", "x")]).contains("--spa"));
        assert!(err(&[("index.html", "a"), ("_routes.json", "x")]).contains("/_routes.json: this provider control file"));
        assert!(err(&[("index.html", "a"), ("_axiom/x.js", "x")]).contains("reserved for the platform"));
        assert!(err(&[("index.html", "a"), (".env", "SECRET=1")]).contains("environment files"));
        assert!(err(&[("index.html", "a"), ("sub/.env.production", "S=1")]).contains("/sub/.env.production"));
        assert!(err(&[("index.html", "a"), ("certs/server.pem", "k")]).contains("private key"));
        assert!(err(&[("index.html", "a"), ("id_dsa", "k")]).contains("private key"));
        assert!(err(&[("index.html", "a"), ("secrets.json", "{}")]).contains("credential"));
        assert!(err(&[("index.html", "a"), (".git/HEAD", "x")]).contains("/.git/HEAD: version-control and credential folders"));
        // A4.3: source maps are never published; there is no option to include them.
        let maps = err(&[("index.html", "a"), ("assets/app.js.map", "{}")]);
        assert!(maps.contains("/assets/app.js.map: source maps are not published") && !maps.contains("--include-source-maps"), "{maps}");
        // A file type the server does not serve is refused before any request.
        let pdf = err(&[("index.html", "a"), ("docs/guide.PDF", "%PDF")]);
        assert!(pdf.contains("/docs/guide.PDF: .pdf files are not served"), "{pdf}");
        let missing = frontend(Path::new("/definitely/not/here"), Spa::Auto).unwrap_err().to_string();
        assert!(missing.contains("does not exist") && missing.contains("axiom deploy frontend ./dist"), "{missing}");
    }

    /// A4.5: the CLI's path rules are the server's `ValidSitePath` exactly, pinned by the
    /// fixture the dashboard tests too.
    #[test]
    fn path_rules_match_the_shared_server_fixture() {
        let raw = include_str!("../../tests/fixtures/deployments-path-rules.json");
        let fixture: serde_json::Value = serde_json::from_str(raw).unwrap();
        let cases = fixture["paths"].as_array().unwrap();
        assert!(cases.len() > 90);
        for case in cases {
            let path = case["path"].as_str().unwrap();
            let allowed = case["allowed"].as_bool().unwrap();
            assert_eq!(site_path_problem(path).is_none(), allowed, "{path:?}: {:?}", site_path_problem(path));
        }
    }

    #[test]
    fn previously_skipped_dotfiles_follow_the_server_rules() {
        // Hidden files the server accepts are published; ones it refuses stop the deploy.
        let d = site(&[("index.html", "a"), (".nojekyll", ""), (".well-known/apple-app-site-association", "{}")]);
        let b = frontend(d.path(), Spa::Auto).unwrap();
        let types: Vec<_> = b.files.iter().map(|f| (f.path.as_str(), f.content_type.unwrap())).collect();
        assert_eq!(types, [("/.nojekyll", "text/plain; charset=utf-8"), ("/.well-known/apple-app-site-association", "application/json"), ("/index.html", "text/html; charset=utf-8")]);
        assert!(err(&[("index.html", "a"), (".htaccess", "x")]).contains("/.htaccess: this looks like a credential"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_traversal_are_refused() {
        let d = site(&[("index.html", "a")]);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), d.path().join("leak.txt")).unwrap();
        let e = frontend(d.path(), Spa::Auto).unwrap_err().to_string();
        assert!(e.contains("/leak.txt: symbolic links"), "{e}");
        let d = site(&[("index.html", "a")]);
        std::os::unix::fs::symlink(outside.path(), d.path().join("up")).unwrap();
        assert!(frontend(d.path(), Spa::Auto).unwrap_err().to_string().contains("symbolic links"));
        // A backslash in a name would become a traversal on other systems.
        let d = site(&[("index.html", "a"), ("..\\x.js", "a")]);
        assert!(frontend(d.path(), Spa::Auto).unwrap_err().to_string().contains("backslashes"));
        // The root itself may not be a symlink either.
        let real = site(&[("index.html", "a")]);
        let link = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(real.path(), link.path().join("dist")).unwrap();
        assert!(frontend(&link.path().join("dist"), Spa::Auto).unwrap_err().to_string().contains("not a folder"));
    }

    #[test]
    fn key_and_map_rules_apply_to_files_not_folders() {
        let d = site(&[("index.html", "a"), ("api.key/readme.txt", "x"), ("vendor.map/x.js", "x"), ("certs.pem/a.txt", "y")]);
        let b = frontend(d.path(), Spa::Auto).unwrap();
        let paths: Vec<_> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/api.key/readme.txt", "/certs.pem/a.txt", "/index.html", "/vendor.map/x.js"]);
        // Names are NFC-normalised before the (ASCII case-insensitive) rules: the Kelvin
        // sign U+212A normalises to K, so "x.\u{212A}EY" is the key file "x.KEY".
        let d = site(&[("index.html", "a"), ("x.\u{212A}EY", "k")]);
        assert!(frontend(d.path(), Spa::Auto).unwrap_err().to_string().contains("private key"));
    }

    #[test]
    fn paths_are_nfc_normalised_and_normalised_duplicates_refused() {
        // "é" decomposed (e + U+0301) becomes the precomposed U+00E9 in the manifest.
        let d = site(&[("index.html", "a"), ("cafe\u{301}.txt", "x")]);
        let b = frontend(d.path(), Spa::Auto).unwrap();
        assert!(b.files.iter().any(|f| f.path == "/caf\u{e9}.txt"));
        let d = site(&[("index.html", "a"), ("cafe\u{301}.txt", "x"), ("caf\u{e9}.txt", "y")]);
        let e = frontend(d.path(), Spa::Auto).unwrap_err().to_string();
        assert!(e.contains("Unicode-normalised"), "{e}");
    }

    #[test]
    fn default_folder_is_dist_then_build_then_out() {
        let d = tempfile::tempdir().unwrap();
        let e = default_frontend_dir(d.path()).unwrap_err().to_string();
        assert!(e.starts_with("No build output found") && e.ends_with("axiom deploy frontend ./dist"), "{e}");
        std::fs::create_dir(d.path().join("out")).unwrap();
        assert!(default_frontend_dir(d.path()).unwrap().ends_with("out"));
        std::fs::create_dir(d.path().join("dist")).unwrap();
        assert!(default_frontend_dir(d.path()).unwrap().ends_with("dist"));
    }

    /// An input folder as `axiom server package --output-input` writes it: JCS input.json
    /// plus the listed files.
    fn input(files: &[(&str, &str)], extra: &[(&str, &str)]) -> (tempfile::TempDir, String) {
        let mut listed = serde_json::Map::new();
        for (name, body) in files {
            listed.insert(name.to_string(), serde_json::json!({"bytes": body.len(), "executable": false, "sha256": sha256_hex(body.as_bytes())}));
        }
        let manifest = serde_json::json!({"artifactReferences": {}, "files": listed, "format": CONTAINER_SCHEMA, "target": {"id": "container-amd64"}, "targetSha256": "a".repeat(64)}).to_string();
        let mut all: Vec<(&str, &str)> = files.to_vec();
        all.extend_from_slice(extra);
        all.push(("input.json", &manifest));
        (site(&all), manifest)
    }

    #[test]
    fn backend_uploads_input_json_verbatim_and_only_the_files_it_lists() {
        let (d, manifest) = input(&[("contract.axiom", "c"), ("contract.server.json", "{}"), ("deployment.json", "{\"d\":1}"), ("files/a.json", "{}")], &[("stray.txt", "not listed")]);
        let b = backend(d.path()).unwrap();
        // A4.2: the manifest is the runtime's input.json byte for byte.
        assert_eq!(b.manifest, manifest.as_bytes());
        assert_eq!(b.manifest_sha256, sha256_hex(manifest.as_bytes()));
        assert_eq!(b.manifest_bytes(), manifest.len() as u64);
        let names: Vec<_> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["contract.axiom", "contract.server.json", "deployment.json", "files/a.json"]);
        // A4.1: entries are the content files only (duplicate content keeps one entry per file).
        assert_eq!(b.entries().len(), 4);
        assert_eq!(b.total_bytes(), 1 + 2 + 7 + 2);
        assert!(b.entries().iter().all(|e| e.sha256 != b.manifest_sha256));
        assert_eq!(b.spa_fallback, None);
    }

    #[test]
    fn backend_input_problems_are_refused_locally() {
        let empty = tempfile::tempdir().unwrap();
        assert!(backend(empty.path()).unwrap_err().to_string().contains("has no input.json"));
        let (d, _) = input(&[("contract.axiom", "c")], &[]);
        std::fs::write(d.path().join("contract.axiom"), "changed").unwrap();
        assert!(backend(d.path()).unwrap_err().to_string().contains("does not match its digest"));
        let (d, _) = input(&[("contract.axiom", "c")], &[]);
        std::fs::remove_file(d.path().join("contract.axiom")).unwrap();
        assert!(backend(d.path()).unwrap_err().to_string().contains("is missing"));
        let d = site(&[("input.json", r#"{"format":"axiom-container-input/v0","files":{}}"#)]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("not axiom-container-input/v1"));
        let d = site(&[("input.json", "not json")]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("not a package input manifest"));
        let (d, _) = input(&[("../escape", "c")], &[]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("cannot hold"));
    }

    /// The real runtime's `--output-input` folder is accepted as it is written.
    #[test]
    fn backend_reads_the_runtimes_own_output_input() {
        let dir = tempfile::tempdir().unwrap();
        let contract: axiom_lib::contract::AxiomFile = serde_json::from_value(serde_json::json!({"header":{"axiom_version":"1","min_runtime_version":1},"project":{"project_id":"q","version":"1","variant":"default","schema_hash":"x"},"ir":{"models":{},"endpoints":{}},"endpoints":[],"validators":{}})).unwrap();
        let plan = axiom_lib::backend::ServerPlan::native(&contract).unwrap();
        let deployment = axiom_lib::backend::Deployment {
            format: axiom_lib::backend::DEPLOYMENT_FORMAT.into(),
            plan_sha256: plan.sha256().unwrap(),
            capabilities: plan.required_capabilities.clone(),
            bindings: Default::default(),
            grants: Default::default(),
            authentication: None,
            coordination: None,
        };
        let (c, p, dep) = (dir.path().join("backend.axiom"), dir.path().join("backend.server.json"), dir.path().join("deployment.json"));
        std::fs::write(&c, serde_json::to_vec(&contract).unwrap()).unwrap();
        std::fs::write(&p, plan.canonical_bytes().unwrap()).unwrap();
        std::fs::write(&dep, serde_json::to_vec(&deployment).unwrap()).unwrap();
        let out = dir.path().join("input");
        axiom_server::container::package_input_only(&c, &p, &dep, None, "container-amd64", &out, None).unwrap();
        let b = backend(&out).unwrap();
        assert_eq!(b.manifest, std::fs::read(out.join("input.json")).unwrap());
        let listed: serde_json::Value = serde_json::from_slice(&b.manifest).unwrap();
        assert_eq!(b.files.len(), listed["files"].as_object().unwrap().len());
        assert!(b.files.iter().any(|f| f.path == "contract.server.json"));
    }

    #[test]
    fn content_types_come_from_the_allowlist() {
        assert_eq!(content_type("/a/B.JS"), Some("text/javascript; charset=utf-8"));
        assert_eq!(content_type("/index.html"), Some("text/html; charset=utf-8"));
        assert_eq!(content_type("/x.php"), None);
        assert_eq!(content_type("/x.map"), None);
        assert_eq!(content_type("/README"), Some("text/plain; charset=utf-8"));
        assert_eq!(content_type("/.well-known/apple-app-site-association"), Some("application/json"));
        // The dashboard's table is the same (pinned by its test against this fixture).
        let raw = include_str!("../../tests/fixtures/deployments-content-types.json");
        let fixture: serde_json::Value = serde_json::from_str(raw).unwrap();
        let table: serde_json::Map<String, serde_json::Value> = CONTENT_TYPES.iter().map(|(e, t)| (e.to_string(), serde_json::json!(t))).collect();
        assert_eq!(fixture["content_types"], serde_json::Value::Object(table));
    }
}
