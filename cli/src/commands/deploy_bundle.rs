//! Local, offline half of `axiom deploy`: walk a build folder, apply the path rules, hash
//! every file and build the canonical manifest whose SHA-256 the upload is keyed by
//! (`axiom-static-bundle/v1` for a frontend, internal-docs planning/deployments/
//! B-W0-FRONTEND-AND-PACKAGING.md §3). A backend's manifest is the runtime's own `input.json`
//! (`axiom server package --output-input`, `axiom-container-input/v1`), uploaded verbatim with
//! the files it lists (contract amendment A4.2).
//!
//! Nothing here signs in or sends a request. `axiom deploy` runs the folder checks
//! ([`check_frontend`], [`check_backend`]: names, path rules, content types, sizes, without
//! reading any file) before registration, sign-in or any network call, so a bad folder fails
//! fast with a fix to apply; [`Checked::hash`] then reads and hashes the files once the
//! project is known to be in the hosting pilot.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Server limits (CONTRACT `AXDEPLOY_INVALID_UPLOAD`), checked locally first.
pub const MAX_FILES: usize = 10_000;
pub const MAX_BYTES: u64 = 1 << 30;
/// One object (axiom-backend `MaxUploadObjectBytes`) and a static manifest (`MaxManifestBytes`).
pub const MAX_OBJECT_BYTES: u64 = 64 << 20;
pub const MAX_MANIFEST_BYTES: usize = 8 << 20;

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
/// serve. The extension is what follows the last `.` of the name, as the server reads it (Go
/// `path.Ext`, so `/.json` has the extension `json`). A name without one (`README`, `foo.`)
/// and a dotfile whose only dot leads (`/.nojekyll`) are `text/plain`, which the server
/// allows on any extension it does not pin to another type (every pinned one is in the table).
pub fn content_type(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        None if path == "/.well-known/apple-app-site-association" => Some("application/json"),
        None => Some("text/plain; charset=utf-8"),
        Some((stem, ext)) => {
            let ext = ext.to_ascii_lowercase();
            match CONTENT_TYPES.iter().find(|(e, _)| *e == ext) {
                Some((_, t)) => Some(*t),
                None if ext.is_empty() || stem.is_empty() => Some("text/plain; charset=utf-8"),
                None => None,
            }
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
pub fn site_path_problem(path: &str) -> Option<std::borrow::Cow<'static, str>> {
    site_path_rule(path).map(|rule| match rule {
        PathRule::Fixed(text) => std::borrow::Cow::Borrowed(text),
        PathRule::NotNfc => std::borrow::Cow::Owned(not_nfc(path)),
    })
}

enum PathRule {
    Fixed(&'static str),
    NotNfc,
}

/// The characters of `name` that Unicode NFC changes (combining marks, and singletons such
/// as the Kelvin sign U+212A), as `U+XXXX`; every non-ASCII character when none stands out.
pub fn non_nfc_chars(name: &str) -> String {
    use unicode_normalization::{char::canonical_combining_class, UnicodeNormalization};
    let code = |c: char| format!("U+{:04X}", c as u32);
    let mut found: Vec<String> = name
        .chars()
        .filter(|&c| canonical_combining_class(c) != 0 || std::iter::once(c).nfc().ne(std::iter::once(c)))
        .map(code)
        .collect();
    if found.is_empty() {
        found = name.chars().filter(|c| !c.is_ascii()).map(code).collect();
    }
    found.dedup();
    found.truncate(4);
    found.join(", ")
}

/// What to do about a path that is not in Unicode NFC: the segment, the offending
/// characters and the normalised name to use instead.
fn not_nfc(path: &str) -> String {
    use unicode_normalization::{is_nfc, UnicodeNormalization};
    let name = path.split('/').find(|s| !is_nfc(s)).unwrap_or(path);
    let normalised: String = name.nfc().collect();
    format!(
        "the name {name:?} is not in Unicode normal form NFC: it contains {} (a decomposed or combining character). Rename it to {normalised:?} (the same name with precomposed characters) and rebuild.",
        non_nfc_chars(name)
    )
}

fn site_path_rule(path: &str) -> Option<PathRule> {
    use unicode_normalization::is_nfc;
    const SYNTAX: &str = "file and folder names cannot contain backslashes, %, ?, # or control characters, or be empty, . or ... Rename it.";
    if path.len() < 2 || !path.starts_with('/') {
        return Some(PathRule::Fixed(SYNTAX));
    }
    if path.len() > 1024 {
        return Some(PathRule::Fixed("the path is longer than 1024 bytes. Shorten the file or folder names."));
    }
    if !is_nfc(path) {
        return Some(PathRule::NotNfc);
    }
    if path.chars().any(|c| matches!(c, '\\' | '%' | '?' | '#' | '\u{FFFD}') || c.is_control()) {
        return Some(PathRule::Fixed(SYNTAX));
    }
    let segments: Vec<&str> = path[1..].split('/').collect();
    if segments.len() > 32 {
        return Some(PathRule::Fixed("the path is nested more than 32 folders deep. Flatten the build output."));
    }
    for (i, s) in segments.iter().enumerate() {
        if s.is_empty() || *s == "." || *s == ".." {
            return Some(PathRule::Fixed(SYNTAX));
        }
        if matches!(go_lower(s).as_str(), ".git" | ".aws" | ".ssh" | ".docker" | ".gnupg" | ".kube")
            || (i > 0 && segments[i - 1].eq_ignore_ascii_case(".config") && s.eq_ignore_ascii_case("gcloud"))
        {
            return Some(PathRule::Fixed("version-control and credential folders (.git, .aws, .ssh, .docker, .gnupg, .kube, .config/gcloud) are never published. Remove it from the build output."));
        }
    }
    if segments[0].eq_ignore_ascii_case("_axiom") {
        return Some(PathRule::Fixed("the /_axiom/ path is reserved for the platform. Rename it in your build."));
    }
    let base = go_lower(segments[segments.len() - 1]);
    if matches!(base.as_str(), "_worker.js" | "_headers" | "_redirects" | "_routes.json") {
        return Some(PathRule::Fixed("this provider control file is not used by Axiom hosting. Remove it from the build; single-page routing is the --spa flag, and headers/redirects are not configurable yet."));
    }
    if base == ".env" || base.starts_with(".env.") {
        return Some(PathRule::Fixed("environment files can hold secrets and are never published. Remove it from the build output."));
    }
    if base.ends_with(".map") {
        return Some(PathRule::Fixed("source maps are not published (they expose your original source). Remove the *.map files from the build output, for example by turning off production source maps in your bundler."));
    }
    let credential = matches!(
        base.as_str(),
        ".npmrc" | ".yarnrc" | ".yarnrc.yml" | ".netrc" | ".git-credentials" | ".htpasswd" | ".htaccess" | ".pgpass" | ".dockercfg" | "credentials" | "credentials.json" | "secrets.json"
    ) || (base.starts_with("service-account") && base.ends_with(".json"))
        || [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".kdbx"].iter().any(|e| base.ends_with(e))
        || ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"].iter().any(|k| base.starts_with(k));
    if credential {
        return Some(PathRule::Fixed("this looks like a credential, private key or server configuration file and is never published. Remove it from the build output."));
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

/// A folder that passed every local check except the file digests: [`Checked::hash`] reads
/// the files and builds the manifest.
#[derive(Clone, Debug)]
pub struct Checked {
    root: PathBuf,
    /// Sorted by path; `sha256` is empty for a frontend until hashed, the listed digest for a
    /// backend.
    files: Vec<File>,
    what: CheckedKind,
}

#[derive(Clone, Debug)]
enum CheckedKind {
    Frontend { spa_fallback: bool },
    /// `input.json` as read (the manifest, uploaded verbatim).
    Backend { manifest: Vec<u8> },
}

impl Checked {
    /// Read and hash every file and build the canonical manifest.
    pub fn hash(self) -> Result<Bundle> {
        let shown = self.root.display().to_string();
        match self.what {
            CheckedKind::Frontend { spa_fallback } => {
                let mut files = self.files;
                for f in &mut files {
                    let bytes = std::fs::read(&f.source).with_context(|| format!("cannot read {}", f.path))?;
                    f.size = bytes.len() as u64;
                    f.sha256 = sha256_hex(&bytes);
                }
                // A file may have grown since the check.
                limits(&files, &shown)?;
                frontend_manifest(&self.root, files, spa_fallback)
            }
            CheckedKind::Backend { manifest } => {
                let redo = BACKEND_REDO;
                for f in &self.files {
                    let bytes = std::fs::read(&f.source).with_context(|| format!("cannot read {shown}/{}", f.path))?;
                    if sha256_hex(&bytes) != f.sha256 || bytes.len() as u64 != f.size {
                        bail!("{shown}/{} does not match its digest in input.json (changed after packaging). {redo}.", f.path);
                    }
                }
                Ok(finish(&self.root, self.files, manifest, None))
            }
        }
    }
}

/// Check a static site folder and build its `axiom-static-bundle/v1` manifest.
pub fn frontend(root: &Path, spa: Spa) -> Result<Bundle> {
    check_frontend(root, spa)?.hash()
}

/// Every local check of a static site folder that needs no file contents.
pub fn check_frontend(root: &Path, spa: Spa) -> Result<Checked> {
    let shown = root.display().to_string();
    let meta = std::fs::symlink_metadata(root).with_context(|| format!("{shown} does not exist. Build your app first, or pass the folder: axiom deploy frontend ./dist"))?;
    if !meta.is_dir() {
        bail!("{shown} is not a folder. Pass the build output folder: axiom deploy frontend ./dist");
    }
    let mut files = Vec::new();
    walk(root, root, "", &mut files, &frontend_rule)?;
    sort_unique(root, &mut files)?;
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
    Ok(Checked { root: root.to_path_buf(), files, what: CheckedKind::Frontend { spa_fallback } })
}

fn frontend_manifest(root: &Path, files: Vec<File>, spa_fallback: bool) -> Result<Bundle> {
    let shown = root.display().to_string();
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
    if manifest.len() > MAX_MANIFEST_BYTES {
        bail!("{shown}: the file list (manifest) would be larger than 8 MiB. Remove files you do not serve, or shorten long paths.");
    }
    Ok(finish(root, files, manifest, Some(spa_fallback)))
}

/// Read a package input folder (`axiom server package … --output-input DIR`): its
/// `input.json` (`axiom-container-input/v1`, written by the runtime) is the manifest,
/// uploaded byte for byte, and the content files are exactly the files it lists, each checked
/// against its listed digest and size (contract amendment A4.2). Other files in the folder are
/// not uploaded.
pub fn backend(root: &Path) -> Result<Bundle> {
    check_backend(root)?.hash()
}

const BACKEND_REDO: &str = "Create it again with `axiom server package <contract> <plan> --deployment <file> --target container-amd64 --output-input <dir>`, or leave out --from-input so `axiom deploy backend` packages this project itself";

/// Every local check of a package input folder except the file digests.
pub fn check_backend(root: &Path) -> Result<Checked> {
    #[derive(serde::Deserialize)]
    struct Listed {
        sha256: String,
        bytes: u64,
        #[serde(default)]
        executable: bool,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Input {
        format: String,
        files: std::collections::BTreeMap<String, Listed>,
        #[serde(default)]
        artifact_references: std::collections::BTreeMap<String, String>,
    }
    let shown = root.display().to_string();
    let redo = BACKEND_REDO;
    let meta = std::fs::symlink_metadata(root).with_context(|| format!("{shown} does not exist. {redo}."))?;
    if !meta.is_dir() {
        bail!("{shown} is not a folder. Pass the folder written by `axiom server package <contract> <plan> --deployment <file> --target container-amd64 --output-input <dir>`, or leave out --from-input so `axiom deploy backend` packages this project itself.");
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
    if input.files.len() < 3 || input.files.len() > MAX_INPUT_FILES {
        bail!("{shown}/input.json lists {} files; a backend input holds 3 to {MAX_INPUT_FILES}. {redo}.", input.files.len());
    }
    // The server's ParseContainerInput requires these and that every artifact reference is listed.
    for required in ["contract.axiom", "contract.server.json", "deployment.json"] {
        if !input.files.contains_key(required) {
            bail!("{shown}/input.json does not list {required}, which every backend input holds. {redo}.");
        }
    }
    if let Some(name) = input.artifact_references.values().find(|n| !input.files.contains_key(n.as_str())) {
        bail!("{shown}/input.json references {name:?}, which it does not list. {redo}.");
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
        // Every folder on the way must be a real folder (no symbolic link out of the input).
        let segments: Vec<&str> = name.split('/').collect();
        let mut at = root.to_path_buf();
        let mut ok = true;
        for (i, segment) in segments.iter().enumerate() {
            at.push(segment);
            let meta = std::fs::symlink_metadata(&at);
            ok = match meta {
                Ok(m) if i + 1 < segments.len() => m.is_dir(),
                Ok(m) => m.is_file(),
                Err(_) => false,
            };
            if !ok {
                break;
            }
        }
        if !ok {
            bail!("{shown}/{name} is listed in input.json but is missing (or not a regular file inside the folder; symbolic links are not followed). {redo}.");
        }
        files.push(File { path: name.clone(), source, sha256: listed.sha256.clone(), size: listed.bytes, content_type: None });
    }
    Ok(Checked { root: root.to_path_buf(), files, what: CheckedKind::Backend { manifest } })
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
    if let Some(f) = files.iter().find(|f| f.size > MAX_OBJECT_BYTES) {
        bail!("{}: {} is larger than the 64 MiB one file may be. Remove it from the build output or serve it from elsewhere.", f.path, human_bytes(f.size));
    }
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
        bail!("{rel}: .{ext} files are not served by Axiom hosting yet. In the pilot every file is served with a fixed, safe content type, and only web content has one: web pages, CSS, JavaScript, JSON, XML, text, images, fonts, WebAssembly, audio and video. Remove it from the build output (or move it out of the folder you deploy) and host it elsewhere, for example in object storage, linking to it from your site.");
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
            if meta.len() > MAX_OBJECT_BYTES {
                bail!("{rel}: {} is larger than the 64 MiB one file may be. Remove it from the build output or serve it from elsewhere.", human_bytes(meta.len()));
            }
            out.push(File { path: rel, source: path, sha256: String::new(), size: meta.len(), content_type: None });
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
fn sort_unique(root: &Path, files: &mut [File]) -> Result<()> {
    use unicode_normalization::is_nfc;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(w) = files.windows(2).find(|w| w[0].path == w[1].path) {
        // The names as they are on disk, relative to the folder; the culprit is the first
        // segment (file or folder) that is not NFC.
        let on_disk = |f: &File| f.source.strip_prefix(root).unwrap_or(&f.source).to_string_lossy().into_owned();
        let (a, b) = (on_disk(&w[0]), on_disk(&w[1]));
        let odd = [&a, &b].into_iter().flat_map(|p| p.split(std::path::MAIN_SEPARATOR)).find(|s| !is_nfc(s)).unwrap_or(&a).to_string();
        bail!(
            "{}: two files have this path once Unicode-normalised (NFC): {a:?} and {b:?}; {odd:?} contains {} (a decomposed or combining character). Delete or rename one of them.",
            w[0].path,
            non_nfc_chars(&odd)
        );
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
    fn files_over_64_mib_are_refused_locally() {
        let d = site(&[("index.html", "a")]);
        let big = std::fs::File::create(d.path().join("video.mp4")).unwrap();
        big.set_len(MAX_OBJECT_BYTES + 1).unwrap();
        let e = frontend(d.path(), Spa::Auto).unwrap_err().to_string();
        assert!(e.contains("/video.mp4") && e.contains("64 MiB"), "{e}");
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
        // Why, and what to do instead.
        assert!(pdf.contains("fixed, safe content type") && pdf.contains("Remove it from the build output") && pdf.contains("host it elsewhere"), "{pdf}");
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
        assert!(e.contains("Unicode-normalised") && e.contains("U+0301") && e.contains("Delete or rename one of them"), "{e}");
        // The server-rule message names the character and the name to use.
        let p = site_path_problem("/cafe\u{301}.txt").unwrap();
        assert!(p.contains("U+0301") && p.contains("Rename it to \"caf\u{e9}.txt\""), "{p}");
        assert!(site_path_problem("/\u{212A}.txt").unwrap().contains("U+212A"));
        // A collision in a folder name names the folder's character.
        let d = site(&[("index.html", "a"), ("cafe\u{301}/a.js", "x"), ("caf\u{e9}/a.js", "y")]);
        let e = frontend(d.path(), Spa::Auto).unwrap_err().to_string();
        assert!(e.contains(r#""cafe\u{301}" contains U+0301"#), "{e}");
    }

    /// The folder check reads no file; hashing does (after the pilot check in `axiom deploy`).
    #[cfg(unix)]
    #[test]
    fn the_folder_check_reads_no_file_and_hashing_matches_frontend() {
        use std::os::unix::fs::PermissionsExt;
        let d = site(&[("index.html", "<h1>hi</h1>"), ("a.js", "1")]);
        let checked = check_frontend(d.path(), Spa::Auto).unwrap();
        assert_eq!(checked.clone().hash().unwrap().manifest, frontend(d.path(), Spa::Auto).unwrap().manifest);
        std::fs::set_permissions(d.path().join("a.js"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let checked = check_frontend(d.path(), Spa::Auto).unwrap();
        if std::fs::read(d.path().join("a.js")).is_err() {
            assert!(checked.hash().unwrap_err().to_string().contains("cannot read /a.js"));
        }
        std::fs::set_permissions(d.path().join("a.js"), std::fs::Permissions::from_mode(0o644)).unwrap();
        // A backend input's digests are checked when hashing.
        let (input, _) = input(&[("contract.axiom", "c"), ("contract.server.json", "{}"), ("deployment.json", "{}")], &[]);
        let checked = check_backend(input.path()).unwrap();
        std::fs::write(input.path().join("contract.axiom"), "x").unwrap();
        assert!(checked.hash().unwrap_err().to_string().contains("does not match its digest"));
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

    const MINIMAL: &[(&str, &str)] = &[("contract.axiom", "c"), ("contract.server.json", "{}"), ("deployment.json", "{}")];

    #[cfg(unix)]
    #[test]
    fn backend_input_does_not_follow_symlinked_folders() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.json"), "{}").unwrap();
        let mut files = MINIMAL.to_vec();
        files.push(("files/x.json", "{}"));
        let (d, _) = input(&files, &[]);
        std::fs::remove_dir_all(d.path().join("files")).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("files")).unwrap();
        let e = backend(d.path()).unwrap_err().to_string();
        assert!(e.contains("files/x.json is listed in input.json but is missing") && e.contains("symbolic links are not followed"), "{e}");
    }

    #[test]
    fn backend_input_problems_are_refused_locally() {
        let empty = tempfile::tempdir().unwrap();
        assert!(backend(empty.path()).unwrap_err().to_string().contains("has no input.json"));
        let (d, _) = input(MINIMAL, &[]);
        std::fs::write(d.path().join("contract.axiom"), "changed").unwrap();
        assert!(backend(d.path()).unwrap_err().to_string().contains("does not match its digest"));
        let (d, _) = input(MINIMAL, &[]);
        std::fs::remove_file(d.path().join("contract.axiom")).unwrap();
        assert!(backend(d.path()).unwrap_err().to_string().contains("is missing"));
        let d = site(&[("input.json", r#"{"format":"axiom-container-input/v0","files":{}}"#)]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("not axiom-container-input/v1"));
        let d = site(&[("input.json", "not json")]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("not a package input manifest"));
        // A traversal name in input.json (written by hand, so nothing lands outside the folder).
        let d = site(&[("input.json", r#"{"format":"axiom-container-input/v1","files":{"../escape":{"sha256":"aa","bytes":1},"contract.axiom":{"sha256":"aa","bytes":1},"contract.server.json":{"sha256":"aa","bytes":1},"deployment.json":{"sha256":"aa","bytes":1}}}"#)]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("cannot hold"));
        // The server's minimum: three files, the three required names, listed artifact references.
        let (d, _) = input(&[("contract.axiom", "c"), ("x.json", "{}"), ("y.json", "{}")], &[]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("does not list contract.server.json"));
        let (d, _) = input(&[("contract.axiom", "c")], &[]);
        assert!(backend(d.path()).unwrap_err().to_string().contains("holds 3 to 256"));
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
        // As Go path.Ext: "/.json" has the pinned extension json; "/.nojekyll" and "foo." are text.
        assert_eq!(content_type("/a/.json"), Some("application/json"));
        assert_eq!(content_type("/.html"), Some("text/html; charset=utf-8"));
        assert_eq!(content_type("/.nojekyll"), Some("text/plain; charset=utf-8"));
        assert_eq!(content_type("/foo."), Some("text/plain; charset=utf-8"));
        // The dashboard's table is the same (pinned by its test against this fixture).
        let raw = include_str!("../../tests/fixtures/deployments-content-types.json");
        let fixture: serde_json::Value = serde_json::from_str(raw).unwrap();
        let table: serde_json::Map<String, serde_json::Value> = CONTENT_TYPES.iter().map(|(e, t)| (e.to_string(), serde_json::json!(t))).collect();
        assert_eq!(fixture["content_types"], serde_json::Value::Object(table));
    }
}
