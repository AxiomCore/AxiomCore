//! Compiler-backed frontend evidence producer for Axiom Inspector.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use axiom_lib::application_evidence::{ApplicationEvidence, EvidenceDiagnostic, EvidenceReference};
use axiom_ui::inspector::{add_cross_layer_edges, add_ir, add_readiness, span_reference};
use axiom_ui::{
    compile_ui_source_at_path, compile_ui_source_with_package_lock_at_path, UiCompileOptions,
    UiTarget,
};
use sha2::{Digest, Sha256};

pub fn enrich(path: &Path, mut evidence: ApplicationEvidence) -> Result<ApplicationEvidence> {
    let requested = fs::canonicalize(path)?;
    let root = if requested.is_file() {
        requested
            .parent()
            .context("inspection input has no parent")?
            .to_path_buf()
    } else {
        requested
    };
    let mut sources = Vec::new();
    discover(&root, &mut sources)?;
    let sources = entry_sources(sources)?;
    for source_path in sources {
        compile_source(&root, &source_path, &mut evidence)?;
    }
    add_cross_layer_edges(&mut evidence);
    add_readiness(&mut evidence);
    evidence.completeness.notes.push(
        "frontend facts come from typed Axiom UI IR; backend facts come from immutable .axiom artifacts".into(),
    );
    evidence.finalize()
}

fn entry_sources(sources: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let import_re = regex::Regex::new(
        r#"(?m)^\s*use\s+(?:component|page)\s+[A-Za-z_][A-Za-z0-9_]*\s+from\s+\"([^\"]+)\"\s*$"#,
    )?;
    let mut imported = BTreeSet::new();
    for source in &sources {
        let text = fs::read_to_string(source)?;
        for capture in import_re.captures_iter(&text) {
            if let Ok(path) = fs::canonicalize(
                source
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(capture.get(1).unwrap().as_str()),
            ) {
                imported.insert(path);
            }
        }
    }
    Ok(sources
        .into_iter()
        .filter(|path| !imported.contains(path))
        .collect())
}

fn discover(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            let name = entry.file_name();
            if matches!(
                name.to_string_lossy().as_ref(),
                ".git" | ".axiom" | "target" | "build" | "node_modules"
            ) {
                continue;
            }
            discover(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("acore") {
            files.push(path);
        }
    }
    Ok(())
}

fn compile_source(root: &Path, path: &Path, evidence: &mut ApplicationEvidence) -> Result<()> {
    let source = fs::read_to_string(path)?;
    let relative = slash(path.strip_prefix(root)?);
    let parent = path.parent().context("Acore source has no parent")?;
    let lock = parent.join("axiom.ui.lock.json");
    let generated_lock = parent.join(".axiom/dev/axiom.ui.lock.json");
    let lock = if lock.is_file() { lock } else { generated_lock };
    let package_lock = parent.join("AxiomPackages.lock");
    let reference = EvidenceReference {
        kind: "source-span".into(),
        path: Some(relative.clone()),
        sha256: Some(hex::encode(Sha256::digest(source.as_bytes()))),
        detail: None,
    };
    let mut emitted = false;
    // Target compilation is independent and read-only. Preserve the fixed
    // Web/Android/iOS merge order while doing the expensive parser/lowerer
    // work concurrently.
    let compilations = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for target in [UiTarget::Web, UiTarget::Android, UiTarget::Ios] {
            let source = &source;
            let lock = lock.clone();
            let package_lock = package_lock.clone();
            let asset_root = parent.to_path_buf();
            handles.push((
                target,
                scope.spawn(move || {
                    let options = UiCompileOptions {
                        target,
                        lock_path: lock,
                        asset_root: Some(asset_root),
                    };
                    if package_lock.is_file() {
                        compile_ui_source_with_package_lock_at_path(
                            source,
                            path,
                            &options,
                            &package_lock,
                        )
                    } else {
                        compile_ui_source_at_path(source, path, &options)
                    }
                }),
            ));
        }
        handles
            .into_iter()
            .map(|(target, handle)| {
                handle
                    .join()
                    .map(|compilation| (target, compilation))
                    .map_err(|_| anyhow::anyhow!("Axiom UI target compiler panicked"))
            })
            .collect::<Result<Vec<_>>>()
    })?;
    for (target, compilation) in compilations {
        if let Some(ir) = compilation.ir {
            emitted = true;
            add_ir(evidence, &ir, &relative, &reference)?;
        }
        for diagnostic in compilation.diagnostics {
            let diagnostic_path = diagnostic
                .source_path
                .as_deref()
                .map(|local| {
                    let parent = Path::new(&relative)
                        .parent()
                        .unwrap_or_else(|| Path::new(""));
                    slash(&parent.join(local))
                })
                .unwrap_or_else(|| relative.clone());
            evidence.diagnostics.push(EvidenceDiagnostic {
                code: diagnostic.code,
                severity: format!("{:?}", diagnostic.severity).to_lowercase(),
                message: format!("{} [{}]", diagnostic.message, target.as_str()),
                node_id: None,
                evidence: vec![span_reference(
                    &diagnostic_path,
                    &diagnostic.span,
                    &reference,
                )],
            });
        }
    }
    if !emitted {
        // Backend-only Acore and non-UI declarations are valid source units;
        // the absence of UiIr is deliberately not turned into an inferred UI.
        return Ok(());
    }
    Ok(())
}

fn slash(path: &Path) -> String {
    path.components()
        .map(|value| value.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_lib::application_evidence::{EvidenceEdgeKind, EvidenceNodeKind};
    use serde_json::json;

    #[test]
    fn frontend_enrichment_is_deterministic_for_extension_example() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extension-sandbox");
        if !root.exists() {
            return;
        }
        let first = enrich(
            &root,
            axiom_lib::application_inspector::inspect_workspace(&root, "test").unwrap(),
        )
        .unwrap();
        let second = enrich(
            &root,
            axiom_lib::application_inspector::inspect_workspace(&root, "test").unwrap(),
        )
        .unwrap();
        assert_eq!(first.graph_revision, second.graph_revision);
        assert!(first
            .nodes
            .iter()
            .any(|node| node.kind == EvidenceNodeKind::FrontendModule));
        assert!(first
            .nodes
            .iter()
            .any(|node| node.kind == EvidenceNodeKind::Action));
    }

    #[test]
    fn shopping_fixture_has_cross_layer_lineage_and_authority() {
        use axiom_lib::application_evidence::{
            AxiomQuery, EvidenceIndex, QueryDirection, QueryOperation, AXIOM_QUERY_FORMAT,
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/axiom-shopping-app");
        if !root.exists() {
            return; // The public examples repository is a separate checkout.
        }
        // The public checkout may carry a lock from an older descriptor schema.
        // Resolve this test's current types in an owned copy, preserving source.
        fn copy_fixture(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let kind = entry.file_type().unwrap();
                if kind.is_symlink()
                    || matches!(
                        entry.file_name().to_str(),
                        Some(".git" | "node_modules" | "target" | "dist")
                    )
                {
                    continue;
                }
                if kind.is_dir() {
                    copy_fixture(&entry.path(), &to.join(entry.file_name()));
                } else if kind.is_file() {
                    fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
                }
            }
        }
        let temporary = tempfile::tempdir().unwrap();
        copy_fixture(&root, temporary.path());
        let root = temporary.path();
        let manifest = root.join("frontend/AxiomDeps.toml");
        let lock = axiom_lib::ui_contract::resolve_manifest(&manifest).unwrap();
        axiom_lib::ui_contract::write_lock(&root.join("frontend/axiom.ui.lock.json"), &lock)
            .unwrap();
        let graph = enrich(
            &root,
            axiom_lib::application_inspector::inspect_workspace(&root, "test").unwrap(),
        )
        .unwrap();
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == EvidenceNodeKind::StyleRule));
        assert!(graph.nodes.iter().any(|node| {
            node.kind == EvidenceNodeKind::SourceUnit
                && node.label == "frontend/pages/home.acore"
                && node.attributes.get("compileTimeOnly") == Some(&json!(true))
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.kind == EvidenceNodeKind::Page
                && node.label == "Home"
                && node
                    .evidence
                    .iter()
                    .any(|reference| reference.path.as_deref() == Some("frontend/pages/home.acore"))
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.kind == EvidenceNodeKind::Component
                && node.label == "ProductCard"
                && node.evidence.iter().any(|reference| {
                    reference.path.as_deref() == Some("frontend/components/product_card.acore")
                })
        }));
        assert!(graph.nodes.iter().any(|node| {
            node.kind == EvidenceNodeKind::Operation
                && node.label == "list_products"
                && node.attributes.get("exposure") == Some(&json!("public"))
        }));
        let button = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == EvidenceNodeKind::Primitive
                    && node
                        .attributes
                        .get("properties")
                        .map(|value| value.to_string().contains("apply_member_price"))
                        .unwrap_or(false)
            })
            .unwrap();
        let trace = EvidenceIndex::new(&graph)
            .unwrap()
            .query(AxiomQuery {
                format: AXIOM_QUERY_FORMAT.into(),
                operation: QueryOperation::Trace {
                    root: button.id.clone(),
                    direction: QueryDirection::Downstream,
                },
                max_depth: 8,
                max_results: 1_000,
            })
            .unwrap();
        assert!(trace
            .nodes
            .iter()
            .any(|node| node.kind == EvidenceNodeKind::Extension && node.label == "pricing"));
        assert!(trace
            .nodes
            .iter()
            .any(|node| node.kind == EvidenceNodeKind::State
                && node.label == "Home.cart.subtotal_cents"));
        let checkout = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == EvidenceNodeKind::Primitive
                    && node
                        .attributes
                        .get("properties")
                        .map(|value| value.to_string().contains("place_order"))
                        .unwrap_or(false)
            })
            .expect("checkout primitive");
        assert_eq!(checkout.targets, vec!["android", "ios", "web"]);
        assert_eq!(
            checkout
                .attributes
                .get("expandedComponentContract")
                .and_then(|value| value.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("ActionButton")
        );
        let field = graph
            .nodes
            .iter()
            .find(|node| node.label == "Product.price_label")
            .unwrap();
        assert!(graph
            .edges
            .iter()
            .any(|edge| { edge.from == field.id && edge.kind == EvidenceEdgeKind::Impacts }));
    }
}
