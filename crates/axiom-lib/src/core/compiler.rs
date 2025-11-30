use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use target_lexicon::Triple;
use tempfile::{tempdir, TempDir};
use tokio::process::Command;

use std::fs;
use std::io::Write;

// URL for macOS flatc binary zip
const FLATC_MAC_URL: &str =
    "https://github.com/google/flatbuffers/releases/download/v25.9.23/Mac.flatc.binary.zip";

/// Entry point used by build
pub async fn compile_runtime(
    rust_code: &str,
    target: &str,
    fbs_schema: &str, // NEW
) -> Result<Vec<u8>> {
    // 1. Validate the target triple
    let target_triple: Triple = target
        .parse()
        .map_err(|e| anyhow!("Invalid target triple '{}': {}", target, e))?;

    // 2. Ensure rustup target is installed
    ensure_target_installed(target).await?;

    // 2.5 Ensure flatc exists (install if needed)
    let flatc_path = ensure_flatc_exists().await?;

    // 3. Create temporary Cargo project
    let dir = tempdir().context("Failed to create temporary directory for build.")?;
    let src_path = dir.path().join("src");
    fs::create_dir(&src_path)?;

    // 3.1 Write Cargo.toml with flatbuffers dependency
    let cargo_toml_content = r#"
[package]
name = "axiom-generated-runtime"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "staticlib"]

[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["macros", "rt-multi-thread"] }
reqwest = { version = "0.12", features = ["json"] }
lazy_static = "1.4"
flatbuffers = "25.0" 
serde_json = "1.0"
"#;
    fs::write(dir.path().join("Cargo.toml"), cargo_toml_content)?;

    // 3.2 Write Rust client code
    fs::write(src_path.join("lib.rs"), rust_code)?;

    // 3.3 Write schema.fbs
    let fbs_path = dir.path().join("schema.fbs");
    fs::write(&fbs_path, fbs_schema)?;

    // 3.4 Run flatc --rust schema.fbs -o src/
    run_flatc_generate_rust(&dir, &flatc_path, &fbs_path, &src_path).await?;

    // 3.5 Rename the generated file to axiom_fb.rs
    rename_generated_flatbuffers_module(&src_path)?;

    // 4. Run cargo build with target
    let binary_bytes = build_cargo_project(dir.path(), target, &target_triple).await?;

    Ok(binary_bytes)
}

//
// ─────────────────────────────────────────────────────────────────────────────
//   INSTALL flatc IF MISSING
// ─────────────────────────────────────────────────────────────────────────────
//

/// Ensure `.axiom/executables/flatc` exists.
/// If not, download and unzip on macOS.
async fn ensure_flatc_exists() -> Result<PathBuf> {
    let home = dirs::home_dir().unwrap_or(PathBuf::from("."));
    let axiom_dir = home.join(".axiom").join("executables");

    let flatc_path = axiom_dir.join("flatc");

    if flatc_path.exists() {
        return Ok(flatc_path);
    }

    println!("🔍 flatc not found. Installing into ~/.axiom/executables...");

    fs::create_dir_all(&axiom_dir)?;

    // Download zip into memory
    let response = reqwest::get(FLATC_MAC_URL)
        .await
        .context("Failed to download flatc for macOS")?;
    let bytes = response.bytes().await?;

    let zip_path = axiom_dir.join("flatc.zip");
    fs::write(&zip_path, &bytes)?;

    // Extract zip
    unzip(&zip_path, &axiom_dir)?;

    // After extraction, the archive contains "flatc"
    let extracted_flatc = axiom_dir.join("flatc");
    if !extracted_flatc.exists() {
        anyhow::bail!("flatc zip extracted but did not contain 'flatc' binary");
    }

    // Ensure executable
    let mut perms = fs::metadata(&extracted_flatc)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&extracted_flatc, perms)?;
    }

    println!("✅ flatc installed at {}", extracted_flatc.display());
    Ok(extracted_flatc)
}

/// Minimal unzip for macOS flatc archive (no dirs inside)
fn unzip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let outpath = dest.join(entry.name());

        if entry.name().ends_with("/") {
            fs::create_dir_all(&outpath)?;
        } else {
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
    }

    Ok(())
}

//
// ─────────────────────────────────────────────────────────────────────────────
//   RUN flatc to generate Rust modules
// ─────────────────────────────────────────────────────────────────────────────
//

async fn run_flatc_generate_rust(
    dir: &TempDir,
    flatc_path: &Path,
    fbs_path: &Path,
    src_path: &Path,
) -> Result<()> {
    let status = Command::new(flatc_path)
        .arg("--rust")
        .arg("--gen-object-api")
        .arg("--gen-name-strings")
        .arg("-o")
        .arg(src_path)
        .arg(fbs_path) // FILE ARG MUST BE LAST
        .status()
        .await
        .context("failed invoking flatc")?;

    if !status.success() {
        anyhow::bail!("flatc failed");
    }

    Ok(())
}

/// flatc generates something like `schema_generated.rs`.
/// Rename it to `axiom_fb.rs`.
fn rename_generated_flatbuffers_module(src_path: &Path) -> Result<()> {
    let entries = fs::read_dir(src_path)?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if name.ends_with("_generated.rs") {
            let new_path = src_path.join("axiom_fb.rs");
            fs::rename(entry.path(), new_path)?;
            return Ok(());
        }
    }

    anyhow::bail!("flatc did not generate a *_generated.rs file");
}

//
// ─────────────────────────────────────────────────────────────────────────────
//   CARGO BUILD
// ─────────────────────────────────────────────────────────────────────────────
//

async fn build_cargo_project(project_dir: &Path, target: &str, triple: &Triple) -> Result<Vec<u8>> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--release")
        .arg("--target")
        .arg(target)
        .current_dir(project_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await.context("Failed to run cargo build")?;
    if !output.status.success() {
        anyhow::bail!(
            "cargo build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let (artifact_filename, artifact_subpath) = match triple.operating_system {
        target_lexicon::OperatingSystem::Windows => ("axiom_generated_runtime.dll", "target"),
        target_lexicon::OperatingSystem::MacOSX { .. } | target_lexicon::OperatingSystem::Ios => {
            ("libaxiom_generated_runtime.dylib", "target")
        }
        _ => ("libaxiom_generated_runtime.so", "target"),
    };

    let artifact_path = project_dir
        .join(artifact_subpath)
        .join(target)
        .join("release")
        .join(artifact_filename);

    if !artifact_path.exists() {
        anyhow::bail!(
            "Build succeeded but artifact missing at {}",
            artifact_path.display()
        );
    }

    let bytes = tokio::fs::read(&artifact_path).await?;
    Ok(bytes)
}

//
// ─────────────────────────────────────────────────────────────────────────────
//   Ensure rustup target exists
// ─────────────────────────────────────────────────────────────────────────────
//

async fn ensure_target_installed(target: &str) -> Result<()> {
    let output = Command::new("rustup")
        .arg("target")
        .arg("list")
        .arg("--installed")
        .output()
        .await
        .context("Failed to run rustup")?;

    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.contains(target) {
        return Ok(());
    }

    println!("Installing missing Rust target {}...", target);

    let status = Command::new("rustup")
        .arg("target")
        .arg("add")
        .arg(target)
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("Failed to install rustup target {}", target);
    }

    println!("Rust target {} installed", target);
    Ok(())
}
