//! The published `axiom-server-host` builder image a no-source container context starts
//! `FROM` (DEPLOYMENTS-DESIGN §10). The release plane publishes and signs one image per
//! runtime version and architecture, then writes its digest into `builder-image.json` before
//! it builds this CLI, so each CLI release pins exactly one builder per target.
use anyhow::{ensure, Context, Result};
use axiom_server::container::{BuilderImage, RUNTIME_VERSION};
use serde::Deserialize;
use std::collections::BTreeMap;

const PIN: &str = include_str!("../../builder-image.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pin {
    format: String,
    repository: String,
    runtime_version: String,
    /// Target → manifest digest (`sha256:<hex>`) of that architecture's image; `null` until
    /// the release plane has published it.
    images: BTreeMap<String, Option<String>>,
    /// The PEM cosign public key the images are signed with; `null` until signing exists.
    cosign_public_key: Option<String>,
}

/// The builder a context uses, and where its pin came from.
#[derive(Debug)]
pub(super) struct Selected {
    pub image: BuilderImage,
    pub pinned_by: &'static str,
    pub public_key: Option<String>,
}

fn pin(text: &str) -> Result<Pin> {
    let pin: Pin = serde_json::from_str(text).context("builder-image.json")?;
    ensure!(pin.format == "axiom-builder-pin/v1", "unsupported builder pin format");
    Ok(pin)
}

/// `explicit` (`--builder-image`, for a mirror) or the image pinned for `target` in this CLI.
pub(super) fn select(target: &str, explicit: Option<&str>) -> Result<Selected> {
    select_from(PIN, target, explicit)
}

fn select_from(text: &str, target: &str, explicit: Option<&str>) -> Result<Selected> {
    let pin = pin(text)?;
    if let Some(reference) = explicit {
        return Ok(Selected { image: BuilderImage::parse(reference)?, pinned_by: "--builder-image", public_key: pin.cosign_public_key });
    }
    ensure!(
        pin.runtime_version == RUNTIME_VERSION,
        "AXPACKAGE_BUILDER_UNPINNED: this CLI pins builder images for runtime {} but packages runtime {RUNTIME_VERSION}; reinstall the CLI from a release",
        pin.runtime_version
    );
    let digest = pin.images.get(target).cloned().flatten().with_context(|| {
        format!(
            "AXPACKAGE_BUILDER_UNPINNED: this CLI build pins no published builder image for {target} (runtime {RUNTIME_VERSION}); pass --builder-image <registry>/<repository>@sha256:<digest>, or --runtime-source DIR to build the runtime from source"
        )
    })?;
    let image = BuilderImage::parse(&format!("{}@{digest}", pin.repository))?;
    Ok(Selected { image, pinned_by: "cli-release", public_key: pin.cosign_public_key })
}

#[cfg(test)]
mod tests {
    use super::*;
    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    fn text(amd64: Option<&str>, version: &str) -> String {
        serde_json::json!({"format":"axiom-builder-pin/v1","repository":"ghcr.io/axiomcore/axiom-server-host","runtimeVersion":version,"images":{"container-amd64":amd64,"container-arm64":null},"cosignPublicKey":null}).to_string()
    }
    #[test]
    fn the_shipped_pin_parses_and_targets_this_runtime() {
        let shipped = pin(PIN).unwrap();
        assert_eq!(shipped.runtime_version, RUNTIME_VERSION, "builder-image.json runtimeVersion must follow axiom-runtime");
        assert_eq!(shipped.images.keys().map(String::as_str).collect::<Vec<_>>(), ["container-amd64", "container-arm64"]);
        for digest in shipped.images.values().flatten() {
            BuilderImage::parse(&format!("{}@{digest}", shipped.repository)).unwrap();
        }
    }
    #[test]
    fn pinned_images_are_used_and_missing_pins_fail_closed() {
        let selected = select_from(&text(Some(DIGEST), RUNTIME_VERSION), "container-amd64", None).unwrap();
        assert_eq!(selected.image.reference(), format!("ghcr.io/axiomcore/axiom-server-host@{DIGEST}"));
        assert_eq!(selected.pinned_by, "cli-release");
        for (pin, target) in [(text(Some(DIGEST), RUNTIME_VERSION), "container-arm64"), (text(None, RUNTIME_VERSION), "container-amd64"), (text(Some(DIGEST), "0.0.9"), "container-amd64")] {
            let error = select_from(&pin, target, None).unwrap_err().to_string();
            assert!(error.starts_with("AXPACKAGE_BUILDER_UNPINNED"), "{error}");
        }
        // A tag in the pin is refused, not resolved.
        assert!(select_from(&text(Some(":0.1.0"), RUNTIME_VERSION), "container-amd64", None).is_err());
        // An explicit mirror must itself be digest-pinned.
        let mirror = format!("asia-southeast1-docker.pkg.dev/p/r/axiom-server-host@{DIGEST}");
        assert_eq!(select_from(&text(None, RUNTIME_VERSION), "container-arm64", Some(&mirror)).unwrap().pinned_by, "--builder-image");
        assert!(select_from(&text(None, RUNTIME_VERSION), "container-arm64", Some("ghcr.io/a/b:latest")).is_err());
    }
}
