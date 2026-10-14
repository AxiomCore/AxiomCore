//! Verified immutable releases and atomic active-host replacement.
use super::*;

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("host record has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn release_root(target: UiTarget) -> Result<PathBuf> {
    Ok(ui_host_record_path(target)?.parent().context("host record has no parent")?.join("verified-releases"))
}

pub(super) fn is_quarantined(target: UiTarget, version: &str) -> Result<bool> {
    let file=ui_host_record_path(target)?.parent().context("host record has no parent")?.join("quarantined-releases.json");
    if !file.exists(){return Ok(false);}
    let value:serde_json::Value=serde_json::from_slice(&std::fs::read(file)?)?;
    if value["format"]!="axiom-ui-host-quarantine/v1"{bail!("invalid local host quarantine index");}
    Ok(value["releases"].get(version).is_some())
}

pub(super) fn quarantine(target: UiTarget, version: String, incident: String) -> Result<()> {
    if ui_host_release_version(&format!("{UI_HOST_RELEASE_TAG_PREFIX}{version}")).is_none() || incident.trim().is_empty() {
        bail!("quarantine requires an exact release version and incident reference");
    }
    let parent=ui_host_record_path(target)?.parent().context("host record has no parent")?.to_path_buf();
    std::fs::create_dir_all(&parent)?;
    let lease=std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(parent.join("install.lock"))?;lease.lock()?;
    let path=parent.join("quarantined-releases.json");
    let mut value=if path.exists(){serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path)?)?}
        else {serde_json::json!({"format":"axiom-ui-host-quarantine/v1","releases":{}})};
    if value["format"]!="axiom-ui-host-quarantine/v1"{bail!("invalid local host quarantine index");}
    if value["releases"].get(&version).is_some(){return Ok(());}
    value["releases"][&version]=serde_json::json!({"incident":incident,"scope":"Local operator quarantine; retained signed bytes and application data are preserved"});
    atomic_json(&path,&value)?;
    println!("Quarantined {version} for {}. Select a compatible retained release with `axiom ui host rollback --target {}`.",target.as_str(),target.as_str());
    Ok(())
}

pub(super) fn validate_record(target: UiTarget, record: &UiHostInstallation) -> Result<()> {
    if record.format != "axiom-ui-host/v1" || record.target != target.as_str() {
        bail!("AXIOM_UI_HOST_RECORD_INVALID: retained host identity differs");
    }
    let artifact = record.artifact.as_ref().context("rollback requires a signed release, not a local source registration")?;
    if is_quarantined(target,&artifact.version)? {bail!("LYNX_BUNDLE_INCOMPATIBLE: release {} is quarantined; select a compatible signed successor or rollback",artifact.version);}
    let manifest = record.host_root.join("host-manifest.json");
    verify_release_manifest_signature(&manifest)?;
    let parsed: UiHostReleaseManifest = serde_json::from_slice(&std::fs::read(&manifest)?)?;
    let asset = select_release_asset(&parsed, target, Some(&artifact.variant), true)?;
    if parsed.version != artifact.version || asset.file != artifact.file || asset.sha256 != artifact.sha256
        || sha256_file(&record.host_root.join(&artifact.file))? != artifact.sha256 {
        bail!("AXIOM_UI_HOST_RECORD_INVALID: retained manifest, record and archive disagree");
    }
    // Use authenticated metadata from the signed manifest, not editable JSON.
    let expected = UiHostInstallation {
        format: record.format.clone(), target: record.target.clone(), host_root: record.host_root.clone(),
        artifact: record.artifact.clone(), delivery_adapter_ready: version_at_least(&parsed.version, host_protocol_minimum(target)),
        toolchain: parsed.toolchain.clone(), compatibility: asset.compatibility.clone(),
    };
    if serde_json::to_value(&expected)? != serde_json::to_value(record)? {
        bail!("AXIOM_UI_HOST_RECORD_INVALID: retained compatibility metadata changed");
    }
    check_installed_frontend(&expected, target)?;
    Ok(())
}

pub(super) fn commit(target: UiTarget, record: &UiHostInstallation) -> Result<()> {
    validate_record(target, record)?;
    let active = ui_host_record_path(target)?;
    let parent = active.parent().context("host record has no parent")?;
    std::fs::create_dir_all(parent)?;
    let lease = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(parent.join("install.lock"))?;
    lease.lock()?;
    let journal = parent.join("install-journal.json");
    if journal.exists() {
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&journal)?)?;
        let current = if active.exists() { Some(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&active)?)?) } else { None };
        if saved.get("next") != current.as_ref() && saved.get("previous") != current.as_ref().or(Some(&serde_json::Value::Null)) {
            bail!("AXIOM_UI_HOST_INSTALL_INTERRUPTED: active record differs from saved install; inspect the journal");
        }
        if let Some(next) = saved.get("next") {
            let next: UiHostInstallation = serde_json::from_value(next.clone())?;
            validate_record(target, &next)?;
        }
        std::fs::remove_file(&journal)?;
    }
    let previous: Option<serde_json::Value> = if active.is_file() {
        Some(serde_json::from_slice(&std::fs::read(&active)?)?)
    } else { None };
    if previous.as_ref() == Some(&serde_json::to_value(record)?) { return Ok(()); }
    atomic_json(&journal, &serde_json::json!({"format":"axiom-ui-host-install-journal/v1", "previous":previous, "next":record}))?;
    // The prior pointer is retained even for legacy records. Rollback still
    // requires its actual signed manifest and compatibility to verify.
    if let Some(previous) = &previous { atomic_json(&parent.join("previous-host.json"), previous)?; }
    atomic_json(&active, record)?;
    std::fs::remove_file(&journal)?;
    Ok(())
}

pub(super) fn retain(target: UiTarget, manifest: &Path, source: &Path, version: &str, variant: &str, digest: &str) -> Result<PathBuf> {
    if !is_safe_variant(variant) || !is_sha256(digest) || ui_host_release_version(&format!("{UI_HOST_RELEASE_TAG_PREFIX}{version}")).is_none() {
        bail!("AXIOM_UI_HOST_RELEASE_INVALID: unsafe immutable release identity");
    }
    let root = release_root(target)?;
    std::fs::create_dir_all(&root)?;
    let destination = root.join(format!("{version}-{variant}-{digest}"));
    if destination.is_symlink() {bail!("AXIOM_UI_HOST_RELEASE_CHANGED: retained release path is linked");}
    let name = source.file_name().context("host archive has no filename")?;
    if destination.exists() {
        if std::fs::read(destination.join("host-manifest.json"))? != std::fs::read(manifest)?
            || std::fs::read(destination.join("host-manifest.json.sig"))? != std::fs::read(format!("{}.sig",manifest.display()))?
            || sha256_file(&destination.join(name))? != digest {
            bail!("AXIOM_UI_HOST_RELEASE_CHANGED: existing retained release has different bytes");
        }
        return Ok(destination);
    }
    let temporary = tempfile::Builder::new().prefix(".install-").tempdir_in(&root)?;
    std::fs::copy(manifest, temporary.path().join("host-manifest.json"))?;
    std::fs::copy(format!("{}.sig", manifest.display()), temporary.path().join("host-manifest.json.sig"))?;
    std::fs::copy(source, temporary.path().join(name))?;
    if sha256_file(&temporary.path().join(name))? != digest { bail!("host archive changed while retaining verified bytes"); }
    for entry in std::fs::read_dir(temporary.path())? { std::fs::File::open(entry?.path())?.sync_all()?; }
    let saved = temporary.keep();
    match std::fs::rename(&saved, &destination) {
        Ok(()) => Ok(destination),
        Err(error) => { let _ = std::fs::remove_dir_all(saved); Err(error.into()) }
    }
}

pub(super) fn cached_manifest(target: UiTarget, version: &str, variant: Option<&str>) -> Result<PathBuf> {
    if ui_host_release_version(&format!("{UI_HOST_RELEASE_TAG_PREFIX}{version}")).is_none() {
        bail!("UI Host version must be an exact stable X.Y.Z");
    }
    let root = release_root(target)?;
    let mut matches = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(root)? {
            let directory = entry?.path();
            if !directory.is_dir() || directory.is_symlink() { continue; }
            if !directory.file_name().is_some_and(|name|name.to_string_lossy().starts_with(&format!("{version}-"))) {continue;}
            let path = directory.join("host-manifest.json");
            if !path.is_file() { continue; }
            verify_release_manifest_signature(&path)?;
            let manifest: UiHostReleaseManifest = serde_json::from_slice(&std::fs::read(&path)?)?;
            if manifest.version == version && select_release_asset(&manifest, target, variant, true).is_ok() { matches.push(path); }
        }
    }
    if matches.len() != 1 { bail!("AXIOM_UI_HOST_OFFLINE_SELECTION: expected one verified cached {version} host; found {}. Supply --release-manifest with its signed assets", matches.len()); }
    Ok(matches.remove(0))
}

pub(super) fn rollback(target: UiTarget, version: Option<String>, variant: Option<String>) -> Result<()> {
    let manifest = if let Some(version) = version {
        cached_manifest(target, &version, variant.as_deref())?
    } else {
        let previous = ui_host_record_path(target)?.parent().context("host record has no parent")?.join("previous-host.json");
        let record: UiHostInstallation = serde_json::from_slice(&std::fs::read(previous).context("no previous verified host; install an exact signed compatible release first")?)?;
        validate_record(target, &record)?;
        record.host_root.join("host-manifest.json")
    };
    install_released_ui_host(target, &manifest, variant, true)
        .context("AXIOM_UI_HOST_ROLLBACK_REFUSED: prior host must support this CLI's renderer, IR and helper contract. Install the matching CLI/bundle pair if needed")
}

pub(super) fn history(target: UiTarget) -> Result<Vec<serde_json::Value>> {
    let root = release_root(target)?;
    let mut result = Vec::new();
    if root.is_dir() {
        for entry in std::fs::read_dir(root)? {
            let path = entry?.path().join("host-manifest.json");
            if path.is_file() {
                let compatible = verify_release_manifest_signature(&path).and_then(|_| {
                    let manifest: UiHostReleaseManifest = serde_json::from_slice(&std::fs::read(&path)?)?;
                    let asset = select_release_asset(&manifest, target, None, true)?;
                    let host = UiHostInstallation {format:"axiom-ui-host/v1".into(),target:target.as_str().into(),
                        host_root:path.parent().unwrap().into(),artifact:Some(UiHostArtifact{version:manifest.version.clone(),
                            variant:asset.variant.clone(),file:asset.file.clone(),sha256:asset.sha256.clone()}),
                        delivery_adapter_ready:version_at_least(&manifest.version,host_protocol_minimum(target)),
                        toolchain:manifest.toolchain.clone(),compatibility:asset.compatibility.clone()};
                    validate_record(target,&host)?; Ok(serde_json::to_value(host)?)
                });
                if let Ok(host) = compatible { result.push(host); }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;
    static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Fixture {
        _lease: std::sync::MutexGuard<'static, ()>,
        directory: tempfile::TempDir,
        old_cache: Option<std::ffi::OsString>,
        old_key: Option<std::ffi::OsString>,
    }
    impl Fixture {
        fn new() -> Self {
            let lease = ENVIRONMENT.lock().unwrap();
            let directory = tempfile::tempdir().unwrap();
            let old_cache = std::env::var_os("AXIOM_UI_CACHE_ROOT");
            let old_key = std::env::var_os("AXIOM_UI_HOST_SIGNING_PUBLIC_KEY_HEX");
            std::env::set_var("AXIOM_UI_CACHE_ROOT", directory.path());
            let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
            std::env::set_var("AXIOM_UI_HOST_SIGNING_PUBLIC_KEY_HEX", key.verifying_key().as_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>());
            Self {_lease:lease,directory,old_cache,old_key}
        }
        fn release(&self, version: &str, data: &[u8]) -> PathBuf {
            let directory = self.directory.path().join(format!("source-{version}")); std::fs::create_dir_all(&directory).unwrap();
            let asset = directory.join("host.zip"); std::fs::write(&asset,data).unwrap();
            let metadata = frontend_toolchain::supported_metadata();
            let digest = sha256_file(&asset).unwrap();
            let manifest = serde_json::json!({"format":"axiom-ui-host-release/v1","version":version,
                "engine":{"commit":metadata.engine_commit},"toolchain":metadata,
                "assets":[{"target":"web","variant":"browser","file":"host.zip","sha256":digest,
                    "compatibility":{"format":"axiom-ui-host-compatibility/v1","target":"web","renderingBackend":"acore-dom",
                        "artifact":{"path":"host.zip","sha256":format!("sha256:{digest}"),"mediaType":"application/zip"},
                        "frontendToolchainId":metadata.frontend_toolchain_id,"engineCommit":null,"engineVersion":null,
                        "acceptedBundleToolchainIds":[metadata.frontend_toolchain_id],"compatibility":metadata.compatibility,"minimumBundleReader":1}}]});
            let path = directory.join("host-manifest.json"); std::fs::write(&path,serde_json::to_vec(&manifest).unwrap()).unwrap();
            let signature = ed25519_dalek::SigningKey::from_bytes(&[7;32]).sign(&std::fs::read(&path).unwrap());
            std::fs::write(directory.join("host-manifest.json.sig"),signature.to_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>()).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for (name, old) in [("AXIOM_UI_CACHE_ROOT",&self.old_cache),("AXIOM_UI_HOST_SIGNING_PUBLIC_KEY_HEX",&self.old_key)] {
                if let Some(value)=old {std::env::set_var(name,value)} else {std::env::remove_var(name)}
            }
        }
    }

    #[test]
    fn signed_update_and_offline_rollback_preserve_both_archives() {
        let fixture=Fixture::new();
        let first=fixture.release("0.6.11",b"first signed fixture archive");
        let second=fixture.release("0.6.12",b"second signed fixture archive");
        install_released_ui_host(UiTarget::Web,&first,None,true).unwrap();
        let previous=std::fs::read(ui_host_record_path(UiTarget::Web).unwrap()).unwrap();
        install_released_ui_host(UiTarget::Web,&second,None,true).unwrap();
        assert_eq!(history(UiTarget::Web).unwrap().len(),2);
        rollback(UiTarget::Web,None,None).unwrap();
        assert_eq!(std::fs::read(ui_host_record_path(UiTarget::Web).unwrap()).unwrap(),previous);
        assert!(cached_manifest(UiTarget::Web,"0.6.12",None).unwrap().is_file());
    }

    #[test]
    fn tampered_update_and_incompatible_rollback_leave_active_pointer_untouched() {
        let fixture=Fixture::new();let first=fixture.release("0.6.11",b"baseline bytes");
        install_released_ui_host(UiTarget::Web,&first,None,true).unwrap();
        let active=ui_host_record_path(UiTarget::Web).unwrap();let before=std::fs::read(&active).unwrap();
        let second=fixture.release("0.6.12",b"candidate bytes");std::fs::write(second.parent().unwrap().join("host.zip"),b"tampered").unwrap();
        assert!(install_released_ui_host(UiTarget::Web,&second,None,true).is_err());
        assert_eq!(std::fs::read(&active).unwrap(),before);
        let mut previous:serde_json::Value=serde_json::from_slice(&before).unwrap();previous["toolchain"]["frontendToolchainId"]=serde_json::Value::String(format!("sha256:{}","c".repeat(64)));
        std::fs::write(active.parent().unwrap().join("previous-host.json"),serde_json::to_vec(&previous).unwrap()).unwrap();
        assert!(rollback(UiTarget::Web,None,None).is_err());assert_eq!(std::fs::read(&active).unwrap(),before);
    }

    #[test]
    fn interrupted_commit_reconciles_only_known_old_or_new_record() {
        let fixture=Fixture::new();let first=fixture.release("0.6.11",b"old bytes");
        install_released_ui_host(UiTarget::Web,&first,None,true).unwrap();let active=ui_host_record_path(UiTarget::Web).unwrap();
        let current:UiHostInstallation=serde_json::from_slice(&std::fs::read(&active).unwrap()).unwrap();
        let journal=active.parent().unwrap().join("install-journal.json");
        atomic_json(&journal,&serde_json::json!({"previous":null,"next":current})).unwrap();
        commit(UiTarget::Web,&current).unwrap();assert!(!journal.exists());
        atomic_json(&journal,&serde_json::json!({"previous":null,"next":{"unexpected":"changed"}})).unwrap();
        let before=std::fs::read(&active).unwrap();assert!(commit(UiTarget::Web,&current).is_err());
        assert_eq!(std::fs::read(&active).unwrap(),before);assert!(journal.exists());
    }

    #[test]
    fn quarantined_regression_is_excluded_and_compatible_rollback_preserves_history() {
        let fixture=Fixture::new();let first=fixture.release("0.6.11",b"verified baseline");
        let second=fixture.release("0.6.12",b"published regression fixture");
        install_released_ui_host(UiTarget::Web,&first,None,true).unwrap();
        install_released_ui_host(UiTarget::Web,&second,None,true).unwrap();
        quarantine(UiTarget::Web,"0.6.12".into(),"fixture-rendering-incident".into()).unwrap();
        assert!(read_ui_host(UiTarget::Web).is_err());
        assert_eq!(history(UiTarget::Web).unwrap().len(),1);
        rollback(UiTarget::Web,None,None).unwrap();
        let path=ui_host_record_path(UiTarget::Web).unwrap();let previous=std::fs::read(&path).unwrap();
        assert!(install_released_ui_host(UiTarget::Web,&second,None,true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(),previous);
        assert!(release_root(UiTarget::Web).unwrap().read_dir().unwrap().any(|entry|entry.unwrap().file_name().to_string_lossy().starts_with("0.6.12-")));
    }
}
