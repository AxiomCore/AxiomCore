//! Local, offline half of `axiom deploy`: walk a build folder, apply the path rules, hash
//! every file and build the canonical manifest whose SHA-256 the upload is keyed by
//! (`axiom-static-bundle/v1` for a frontend, `axiom-container-input/v1` for a backend input;
//! internal-docs planning/deployments/B-W0-FRONTEND-AND-PACKAGING.md §3, §5).
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
    /// Manifest path: one leading slash, `/`-separated.
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
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
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

/// Content types by extension (lowercase). Anything else is `application/octet-stream`,
/// which browsers never execute or render as a page. The dashboard uses the same table.
pub const CONTENT_TYPES: &[(&str, &str)] = &[
    ("html", "text/html; charset=utf-8"),
    ("htm", "text/html; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("cjs", "text/javascript; charset=utf-8"),
    ("json", "application/json"),
    ("map", "application/json"),
    ("webmanifest", "application/manifest+json"),
    ("txt", "text/plain; charset=utf-8"),
    ("md", "text/markdown; charset=utf-8"),
    ("csv", "text/csv; charset=utf-8"),
    ("xml", "application/xml"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("ico", "image/x-icon"),
    ("bmp", "image/bmp"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("wasm", "application/wasm"),
    ("pdf", "application/pdf"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("mp3", "audio/mpeg"),
    ("ogg", "audio/ogg"),
    ("wav", "audio/wav"),
    ("glb", "model/gltf-binary"),
    ("gltf", "model/gltf+json"),
];

pub fn content_type(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    CONTENT_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, t)| *t).unwrap_or("application/octet-stream")
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
pub fn frontend(root: &Path, spa: Spa, include_source_maps: bool) -> Result<Bundle> {
    let shown = root.display().to_string();
    let meta = std::fs::symlink_metadata(root).with_context(|| format!("{shown} does not exist. Build your app first, or pass the folder: axiom deploy frontend ./dist"))?;
    if !meta.is_dir() {
        bail!("{shown} is not a folder. Pass the build output folder: axiom deploy frontend ./dist");
    }
    let mut files = Vec::new();
    walk(root, root, &mut files, &|rel, name| frontend_rule(rel, name, include_source_maps))?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if !files.iter().any(|f| f.path == "/index.html") {
        bail!("{shown} has no index.html at its top level. Pass the folder that contains index.html (your build output), for example: axiom deploy frontend ./dist");
    }
    for f in &mut files {
        f.content_type = Some(content_type(&f.path));
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

/// Check a container input folder (`axiom server package --output-input DIR`) and build its
/// `axiom-container-input/v1` manifest. Every regular file is included.
pub fn backend(root: &Path) -> Result<Bundle> {
    let shown = root.display().to_string();
    let meta = std::fs::symlink_metadata(root).with_context(|| format!("{shown} does not exist. Create it with `axiom server package … --output-input {shown}`, or leave out --from-input to package this project"))?;
    if !meta.is_dir() {
        bail!("{shown} is not a folder. Pass the folder written by `axiom server package … --output-input DIR`");
    }
    let mut files = Vec::new();
    walk(root, root, &mut files, &|_, _| Ok(Visit::Keep))?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.is_empty() {
        bail!("{shown} is empty. Pass the folder written by `axiom server package … --output-input DIR`");
    }
    limits(&files, &shown)?;
    #[derive(Serialize)]
    struct Entry<'a> {
        path: &'a str,
        sha256: &'a str,
        size: u64,
    }
    #[derive(Serialize)]
    struct Manifest<'a> {
        schema: &'a str,
        files: Vec<Entry<'a>>,
    }
    let manifest = serde_json::to_vec(&Manifest {
        schema: CONTAINER_SCHEMA,
        files: files.iter().map(|f| Entry { path: &f.path, sha256: &f.sha256, size: f.size }).collect(),
    })?;
    Ok(finish(root, files, manifest, None))
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

/// Path rules of `axiom-static-bundle/v1` for one entry. `rel` is the manifest path.
fn frontend_rule(rel: &str, name: &str, include_source_maps: bool) -> Result<Visit> {
    let lower = name.to_ascii_lowercase();
    if rel == "/_axiom" || rel.starts_with("/_axiom/") {
        bail!("{rel}: the /_axiom/ path is reserved for the platform. Rename that folder in your build.");
    }
    if matches!(lower.as_str(), "_headers" | "_redirects" | "_worker.js") {
        bail!("{rel}: {name} files are not used by Axiom hosting. Remove it from the build; single-page routing is the --spa flag, and headers/redirects are not configurable yet.");
    }
    if lower.starts_with(".env") {
        bail!("{rel}: environment files can hold secrets and are never published. Remove it from the build output.");
    }
    if lower.ends_with(".pem") || lower.ends_with(".key") || lower.starts_with("id_rsa") || lower.starts_with("id_ed25519") || lower.starts_with("id_ecdsa") {
        bail!("{rel}: this looks like a private key and is never published. Remove it from the build output.");
    }
    if name.starts_with('.') && name != ".well-known" {
        // .DS_Store, .git, editor folders: never part of a site.
        return Ok(Visit::Skip);
    }
    if lower.ends_with(".map") && !include_source_maps {
        bail!("{rel}: source maps publish your original source and are left out by default. Remove them from the build, or pass --include-source-maps to publish them.");
    }
    Ok(Visit::Keep)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<File>, rule: &dyn Fn(&str, &str) -> Result<Visit>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("cannot read {}", dir.display()))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let shown = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        let name = name.to_str().with_context(|| format!("{shown}: file names must be UTF-8. Rename it."))?.to_string();
        if name.contains('\\') || name.contains('\0') || name.chars().any(char::is_control) || name == "." || name == ".." {
            bail!("{shown}: file names cannot contain backslashes or control characters. Rename it.");
        }
        let rel_parent = path.parent().and_then(|p| p.strip_prefix(root).ok()).map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")).unwrap_or_default();
        let rel = if rel_parent.is_empty() { format!("/{name}") } else { format!("/{rel_parent}/{name}") };
        if let Visit::Skip = rule(&rel, &name)? {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path).with_context(|| format!("cannot read {shown}"))?;
        if meta.file_type().is_symlink() {
            bail!("{rel}: symbolic links are not deployed. Replace it with the real file (or folder) in the build output.");
        }
        if meta.is_dir() {
            walk(root, &path, out, rule)?;
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
        frontend(d.path(), Spa::Auto, false).unwrap_err().to_string()
    }

    #[test]
    fn manifest_is_canonical_sorted_and_deterministic() {
        let d = site(&[("assets/app.js", "console.log(1)"), ("index.html", "<h1>hi</h1>"), (".DS_Store", "x"), (".git/HEAD", "x"), (".well-known/security.txt", "c")]);
        let b = frontend(d.path(), Spa::Auto, false).unwrap();
        let paths: Vec<_> = b.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/.well-known/security.txt", "/assets/app.js", "/index.html"]);
        let text = String::from_utf8(b.manifest.clone()).unwrap();
        let html = sha256_hex(b"<h1>hi</h1>");
        assert!(text.starts_with(r#"{"schema":"axiom-static-bundle/v1","entrypoint":"/index.html","spa_fallback":true,"files":[{"path":"/.well-known/security.txt""#), "{text}");
        assert!(text.ends_with(&format!(r#"{{"path":"/index.html","sha256":"{html}","size":11,"content_type":"text/html; charset=utf-8"}}]}}"#)), "{text}");
        assert_eq!(b.manifest_sha256, sha256_hex(&b.manifest));
        assert_eq!(frontend(d.path(), Spa::Auto, false).unwrap().manifest, b.manifest);
        assert_eq!(b.total_bytes(), 1 + 14 + 11);
    }

    #[test]
    fn spa_defaults_on_unless_the_site_has_a_404_page() {
        let d = site(&[("index.html", "a")]);
        assert_eq!(frontend(d.path(), Spa::Auto, false).unwrap().spa_fallback, Some(true));
        assert_eq!(frontend(d.path(), Spa::Off, false).unwrap().spa_fallback, Some(false));
        let d = site(&[("index.html", "a"), ("404.html", "b")]);
        assert_eq!(frontend(d.path(), Spa::Auto, false).unwrap().spa_fallback, Some(false));
        assert_eq!(frontend(d.path(), Spa::On, false).unwrap().spa_fallback, Some(true));
    }

    #[test]
    fn path_rules_reject_with_guidance() {
        assert!(err(&[("about.html", "a")]).contains("no index.html"));
        assert!(err(&[("index.html", "a"), ("_headers", "x")]).contains("_headers files are not used"));
        assert!(err(&[("index.html", "a"), ("_redirects", "x")]).contains("--spa"));
        assert!(err(&[("index.html", "a"), ("_worker.js", "x")]).contains("_worker.js"));
        assert!(err(&[("index.html", "a"), ("_axiom/x.js", "x")]).contains("reserved for the platform"));
        assert!(err(&[("index.html", "a"), (".env", "SECRET=1")]).contains("environment files"));
        assert!(err(&[("index.html", "a"), ("sub/.env.production", "S=1")]).contains("/sub/.env.production"));
        assert!(err(&[("index.html", "a"), ("certs/server.pem", "k")]).contains("private key"));
        assert!(err(&[("index.html", "a"), ("id_rsa", "k")]).contains("private key"));
        assert!(err(&[("index.html", "a"), ("tls.key", "k")]).contains("private key"));
        let maps = err(&[("index.html", "a"), ("assets/app.js.map", "{}")]);
        assert!(maps.contains("--include-source-maps") && maps.contains("/assets/app.js.map"), "{maps}");
        let d = site(&[("index.html", "a"), ("assets/app.js.map", "{}")]);
        let b = frontend(d.path(), Spa::Auto, true).unwrap();
        assert_eq!(b.files[0].content_type, Some("application/json"));
        let missing = frontend(Path::new("/definitely/not/here"), Spa::Auto, false).unwrap_err().to_string();
        assert!(missing.contains("does not exist") && missing.contains("axiom deploy frontend ./dist"), "{missing}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_traversal_are_refused() {
        let d = site(&[("index.html", "a")]);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), d.path().join("leak.txt")).unwrap();
        let e = frontend(d.path(), Spa::Auto, false).unwrap_err().to_string();
        assert!(e.contains("/leak.txt: symbolic links"), "{e}");
        let d = site(&[("index.html", "a")]);
        std::os::unix::fs::symlink(outside.path(), d.path().join("up")).unwrap();
        assert!(frontend(d.path(), Spa::Auto, false).unwrap_err().to_string().contains("symbolic links"));
        // A backslash in a name would become a traversal on other systems.
        let d = site(&[("index.html", "a"), ("..\\x.js", "a")]);
        assert!(frontend(d.path(), Spa::Auto, false).unwrap_err().to_string().contains("backslashes"));
        // The root itself may not be a symlink either.
        let real = site(&[("index.html", "a")]);
        let link = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(real.path(), link.path().join("dist")).unwrap();
        assert!(frontend(&link.path().join("dist"), Spa::Auto, false).unwrap_err().to_string().contains("not a folder"));
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

    #[test]
    fn backend_input_manifest_lists_every_file() {
        let d = site(&[("contract.axiom", "c"), ("files/a.json", "{}")]);
        let b = backend(d.path()).unwrap();
        assert_eq!(String::from_utf8(b.manifest).unwrap(), format!(r#"{{"schema":"axiom-container-input/v1","files":[{{"path":"/contract.axiom","sha256":"{}","size":1}},{{"path":"/files/a.json","sha256":"{}","size":2}}]}}"#, sha256_hex(b"c"), sha256_hex(b"{}")));
        assert_eq!(b.spa_fallback, None);
        let empty = tempfile::tempdir().unwrap();
        assert!(backend(empty.path()).unwrap_err().to_string().contains("is empty"));
    }

    #[test]
    fn content_types_come_from_the_allowlist() {
        assert_eq!(content_type("/a/B.JS"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("/index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("/x.php"), "application/octet-stream");
        assert_eq!(content_type("/README"), "application/octet-stream");
    }
}
