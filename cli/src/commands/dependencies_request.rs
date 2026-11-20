//! X10d (D8): `axiom dependencies request --from-summary` and the
//! `axiom dependencies check --from-summary` report.
//!
//! The permission request is generated from the extension's static effect
//! summary, and only from a summary this host computed while building the
//! extension from source (`effect_summary::trusted_summary` with
//! `SummaryTrust::BuiltFromSourceHere`). Anything else (an `axiom-build`
//! without the X9a analysers, a package without a summary, a legacy package,
//! a TypeScript summary while TypeScript is not authoritative) is "not
//! derivable": the current permissions are kept unchanged and the exit code
//! is zero with a note. An extension that does not build from source is
//! different (w3-bc3, w3-vbc2 finding 11): its permissions were not checked
//! at all, so both commands fail with `AXIOM_PERMISSION_REQUEST_BUILD_FAILED`
//! and the build's message (a removed Rust permission makes the Rust build
//! fail, which must not look like a green check). A proposal never
//! changes the grant or the lock; `--write` edits `AxiomDeps.toml` only, and
//! an increase then needs the existing permission-increase review.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use axiom_build::core::extension_source::SourceBuildOptions;
use axiom_build::core::typescript_source::build_source_extension;
use axiom_lib::deps_manifest::{parse_axiom_deps_v2, PermissionStyle};
use axiom_lib::effect_summary::{trusted_summary, EffectSummary, SummaryTrust, TrustedSummary};
use axiom_lib::package::{decode_axiom_package, DecodedAxiomPackage, ExtensionPackagePayload, PackageContents, PackageTarget};
use axiom_lib::permission_request::{
    permission_request_report, render_permissions_table, rewrite_extension_permissions,
    PermissionRequestReport, PERMISSION_REQUEST_BUILD_FAILED, PERMISSION_REQUEST_MISSING,
};
use sha2::{Digest, Sha256};

/// File name of the canonical summary a source build writes beside
/// `module.axiom` (X9a contract §6; `axiom-build` `EFFECT_SUMMARY_FILE`).
pub const EFFECT_SUMMARY_FILE: &str = "effect-summary.json";

/// `--format` of `axiom dependencies request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum RequestFormatArg {
    /// Keep the file's spelling (shorthand when it already uses it).
    Preserve,
    Long,
    Shorthand,
}

impl From<RequestFormatArg> for PermissionStyle {
    fn from(value: RequestFormatArg) -> Self {
        match value {
            RequestFormatArg::Preserve => Self::Preserve,
            RequestFormatArg::Long => Self::Long,
            RequestFormatArg::Shorthand => Self::Shorthand,
        }
    }
}

/// Why a local summary is not trusted, beyond what the summary carries.
pub(crate) enum LocalNote {
    /// Not derivable (no analysers, legacy package, ...): permissions kept,
    /// exit zero.
    Untrusted(String),
    /// The extension does not build from source: an error.
    BuildFailed(String),
}

/// The summary of `alias` that this host may rely on, and a note when it is
/// not trusted for a reason the summary itself cannot carry (the build).
pub(crate) fn local_trusted_summary(deps: &Path, alias: &str, out: &Path) -> (TrustedSummary, Option<LocalNote>) {
    let manifest = std::fs::read_to_string(deps).ok().and_then(|source| parse_axiom_deps_v2(&source).ok());
    let extension = manifest.as_ref().and_then(|manifest| manifest.extensions.get(alias));
    let language = extension.map(|extension| extension.language.clone()).unwrap_or_default();
    // Exports named in the manifest, else the ones a build recorded; the
    // generator keeps every permission of an untrusted summary either way.
    let mut declared: Vec<String> = extension.map(|extension| extension.exports.clone()).unwrap_or_default();
    if declared.is_empty() {
        declared = axiom_lib::extension_source::derive_extension_exports(deps)
            .ok()
            .and_then(|mut derived| derived.remove(alias))
            .unwrap_or_default();
    }
    let untrusted = |exports: &[String], language: &str, why: String| {
        (trusted_summary(None, &"0".repeat(64), language, exports, &SummaryTrust::PublisherClaim), Some(LocalNote::Untrusted(why)))
    };
    let target = extension.and_then(|extension| extension.targets.first().copied()).unwrap_or(PackageTarget::Web);
    let build = match build_source_extension(&SourceBuildOptions {
        deps: deps.to_path_buf(),
        alias: alias.to_string(),
        output_root: out.to_path_buf(),
        target,
        clean: false,
    }) {
        Ok(build) => build,
        Err(error) => {
            return (
                trusted_summary(None, &"0".repeat(64), &language, &declared, &SummaryTrust::PublisherClaim),
                Some(LocalNote::BuildFailed(format!("{error:#}"))),
            )
        }
    };
    let executable = match std::fs::read(&build.package)
        .context("read the built package")
        .and_then(|bytes| decode_axiom_package(&bytes))
    {
        Ok(DecodedAxiomPackage::Package(package)) => match package.contents {
            PackageContents::Extension(ExtensionPackagePayload::V2(executable)) => executable,
            _ => return untrusted(&declared, &language, "the source build did not produce axiom-extension/v2".into()),
        },
        Ok(_) => return untrusted(&declared, &language, "the source build produced a legacy package".into()),
        Err(error) => return untrusted(&declared, &language, format!("the built package does not decode: {error:#}")),
    };
    let exports: Vec<String> = executable.exports.iter().map(|export| export.name.clone()).collect();
    let local_path = build.package.parent().map(|dir| dir.join(EFFECT_SUMMARY_FILE));
    let local = local_path.as_ref().and_then(|path| std::fs::read(path).ok());
    // The local file is the analyser output of the build that just ran (or
    // verified its cache entry) in this process; the packaged summary must be
    // byte-for-byte that output and bound to this module and these sources.
    let trust = match (&executable.effect_summary, &local) {
        (Some(_), Some(bytes)) => match EffectSummary::from_canonical_bytes(bytes) {
            Ok(computed) => SummaryTrust::BuiltFromSourceHere {
                module_sha256: build.module_sha256.clone(),
                source_sha256: build.source_sha256.clone(),
                current: vec![computed.analyzer.clone()],
                local_summary_sha256: hex::encode(Sha256::digest(bytes)),
            },
            Err(_) => SummaryTrust::PublisherClaim,
        },
        _ => SummaryTrust::PublisherClaim,
    };
    let note = match (&executable.effect_summary, &local) {
        (None, _) => Some("the local source build produced no effect summary (this axiom-build has no X9a analysers, or the package predates them)".to_string()),
        (Some(_), None) => Some(format!("the local source build wrote no {EFFECT_SUMMARY_FILE}")),
        _ => None,
    };
    (
        trusted_summary(executable.effect_summary.as_ref(), &executable.interface_sha256, &executable.sdk.language, &exports, &trust),
        note.map(LocalNote::Untrusted),
    )
}

/// The prefix that makes summary sites project-relative. Analysers write
/// site files relative to the extension's source root; for some languages
/// that is already the manifest directory (Python), for others the entry
/// point's directory. The first site that exists decides; with no site, the
/// entry point's directory is used.
fn site_prefix(deps: &Path, source: &Path, summary: &TrustedSummary) -> String {
    let root = deps.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let entry_dir = source.parent().map(|dir| dir.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    match summary.summary().sites.first() {
        Some(site) if root.join(&site.file).is_file() => String::new(),
        _ => entry_dir,
    }
}

/// One report per selected extension (all when `alias` is `None`).
pub(crate) fn reports(deps: &Path, alias: Option<&str>, out: &Path) -> Result<Vec<PermissionRequestReport>> {
    let source = std::fs::read_to_string(deps).with_context(|| format!("read {}", deps.display()))?;
    let manifest = parse_axiom_deps_v2(&source)?;
    let aliases: Vec<String> = match alias {
        Some(alias) => {
            if !manifest.extensions.contains_key(alias) {
                bail!("AxiomDeps.toml has no extension `{alias}`");
            }
            vec![alias.to_string()]
        }
        None => manifest.extensions.keys().cloned().collect(),
    };
    let mut out_reports = Vec::new();
    for alias in aliases {
        let extension = &manifest.extensions[&alias];
        let (summary, note) = local_trusted_summary(deps, &alias, out);
        let mut report = permission_request_report(&alias, &summary, &extension.permissions, &extension.budgets, &site_prefix(deps, &extension.source, &summary))?;
        match note {
            Some(LocalNote::Untrusted(note)) if !report.trusted => {
                report.trust_note = axiom_lib::effect_summary::printable(&note);
                report.proposal.trust_note = report.trust_note.clone();
            }
            Some(LocalNote::BuildFailed(error)) => {
                report.trust_note = "the extension does not build from source here".into();
                report.proposal.trust_note = report.trust_note.clone();
                report.build_error = axiom_lib::effect_summary::printable(&error);
            }
            _ => {}
        }
        out_reports.push(report);
    }
    Ok(out_reports)
}

fn output_root(deps: &Path, out: &Path) -> PathBuf {
    if out.is_absolute() {
        out.to_path_buf()
    } else {
        deps.parent().filter(|dir| !dir.as_os_str().is_empty()).map(|dir| dir.join(out)).unwrap_or_else(|| out.to_path_buf())
    }
}

/// `axiom dependencies request --from-summary [<alias>] [--write]
/// [--format long|shorthand|preserve] [--json]`.
pub async fn handle_request(
    deps: PathBuf,
    alias: Option<String>,
    from_summary: bool,
    write: bool,
    format: RequestFormatArg,
    json: bool,
    out: PathBuf,
) -> Result<()> {
    if !from_summary {
        bail!("`axiom dependencies request` needs `--from-summary` (the only request source today)");
    }
    let out = output_root(&deps, &out);
    let reports = reports(&deps, alias.as_deref(), &out)?;
    let mut written = Vec::new();
    if write {
        let original = std::fs::read_to_string(&deps).with_context(|| format!("read {}", deps.display()))?;
        let mut source = original.clone();
        // Only tables whose canonical permissions change are rewritten; an
        // unchanged (or untrusted, hence kept) table keeps its bytes.
        for report in reports.iter().filter(|report| !report.diff.increase.is_empty() || !report.diff.decrease.is_empty()) {
            source = rewrite_extension_permissions(&source, &report.alias, &report.proposal.permissions, format.into())?;
        }
        if source != original {
            super::dependencies::atomic_write(&deps, source.as_bytes())?;
            written = reports
                .iter()
                .filter(|report| !report.diff.increase.is_empty() || !report.diff.decrease.is_empty())
                .map(|report| report.alias.clone())
                .collect();
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
    } else {
        for report in &reports {
            print!("{}", report.render());
            if !write {
                println!("  proposed table:");
                for line in render_permissions_table(&report.alias, &report.proposal.permissions, format.into())?.lines() {
                    println!("    {line}");
                }
            }
        }
        if write {
            if written.is_empty() {
                println!("{} unchanged.", deps.display());
            } else {
                println!(
                    "Wrote {} ({}). The grant and the lock are unchanged: the next build reviews the authority change, and any increase needs approval as before.",
                    deps.display(),
                    written.join(", ")
                );
            }
        }
    }
    // CI: non-zero when the manifest lacks a permission a known effect needs
    // (after --write, only what could not be generated is still missing).
    let failing: Vec<&str> = reports
        .iter()
        .filter(|report| if write { !report.unsatisfiable.is_empty() } else { report.missing_required })
        .map(|report| report.alias.as_str())
        .collect();
    let broken: Vec<&str> = reports.iter().filter(|report| !report.build_error.is_empty()).map(|report| report.alias.as_str()).collect();
    if !broken.is_empty() {
        bail!("{}", build_failed_message(&reports));
    }
    if !failing.is_empty() {
        bail!("{PERMISSION_REQUEST_MISSING}: AxiomDeps.toml lacks permission(s) a known effect needs or the code attempts without a permission (ungranted): {}", failing.join(", "));
    }
    Ok(())
}

/// The error of `request` and `check --from-summary` when an extension does
/// not build: every failing alias with the build's message.
pub(crate) fn build_failed_message(reports: &[PermissionRequestReport]) -> String {
    let failures: Vec<String> = reports
        .iter()
        .filter(|report| !report.build_error.is_empty())
        .map(|report| format!("`{}`: {}", report.alias, report.build_error))
        .collect();
    format!(
        "{PERMISSION_REQUEST_BUILD_FAILED}: the extension does not build from source, so its permissions could not be checked against its effect summary: {}",
        failures.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir.join("logic")).unwrap();
        std::fs::write(dir.join("logic/pricing.rs"), "pub fn pricing() {}\n").unwrap();
        let deps = dir.join("AxiomDeps.toml");
        std::fs::write(
            &deps,
            "format = \"axiom-deps/v2\"\ntargets = [\"web\"]\n\n[extensions.pricing]\nlanguage = \"rust\"\nsource = \"logic/pricing.rs\"\nexports = [\"apply\"]\ntargets = [\"web\"]\n\n[extensions.pricing.permissions]\nruntime = [\"clock.wall\"]\nsecrets = [\"payment\"]\n",
        )
        .unwrap();
        deps
    }

    /// w3-bc3 (w3-vbc2 finding 11): an extension that does not build from
    /// source (this one is not a valid extension) was not checked at all:
    /// `request` and `check --from-summary` fail with the build's message,
    /// and the manifest is kept exactly (`--write` changes nothing).
    #[tokio::test]
    async fn an_extension_that_does_not_build_fails_the_request_and_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let deps = project(dir.path());
        let before = std::fs::read(&deps).unwrap();
        let out = dir.path().join(".axiom/extensions");
        let reports = reports(&deps, Some("pricing"), &out).unwrap();
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert!(!report.trusted);
        assert!(!report.build_error.is_empty(), "{report:?}");
        assert!(report.render().contains(PERMISSION_REQUEST_BUILD_FAILED), "{}", report.render());
        assert!(report.diff.increase.is_empty() && report.diff.decrease.is_empty());
        let error = handle_request(deps.clone(), Some("pricing".into()), true, true, RequestFormatArg::Long, true, out.clone()).await.unwrap_err();
        assert!(error.to_string().contains(PERMISSION_REQUEST_BUILD_FAILED), "{error:#}");
        assert!(error.to_string().contains(&report.build_error), "the build's message: {error:#}");
        assert_eq!(std::fs::read(&deps).unwrap(), before, "--write without a trusted summary changes nothing");
        let error = super::super::dependencies::handle_check_with(deps.clone(), true, None, Some(out)).await.unwrap_err();
        assert!(error.to_string().contains(PERMISSION_REQUEST_BUILD_FAILED), "{error:#}");
    }

    #[tokio::test]
    async fn request_needs_from_summary_and_a_known_alias() {
        let dir = tempfile::tempdir().unwrap();
        let deps = project(dir.path());
        let out = dir.path().join(".axiom/extensions");
        assert!(handle_request(deps.clone(), None, false, false, RequestFormatArg::Preserve, false, out.clone()).await.is_err());
        let error = handle_request(deps, Some("nope".into()), true, false, RequestFormatArg::Preserve, false, out).await.unwrap_err();
        assert!(error.to_string().contains("no extension `nope`"), "{error}");
    }
}
