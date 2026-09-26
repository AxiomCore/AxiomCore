//! Compiler-backed frontend evidence producer for Axiom Inspector.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use axiom_lib::application_evidence::{
    edge_id, semantic_id, ApplicationEvidence, EvidenceDiagnostic, EvidenceEdge, EvidenceEdgeKind,
    EvidenceNode, EvidenceNodeKind, EvidenceReference, TruthLayer, VerificationState,
};
use axiom_ui::{
    compile_ui_source_at_path, compile_ui_source_with_package_lock_at_path,
    expression_references_identifier, standard_component_contract, SourceSpan, UiActionStep,
    UiCompileOptions, UiIr, UiNode, UiTarget,
};
use serde_json::json;
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

fn add_ir(
    evidence: &mut ApplicationEvidence,
    ir: &UiIr,
    source: &str,
    reference: &EvidenceReference,
) -> Result<()> {
    let target = ir.target.as_str().to_string();
    if !evidence.targets.contains(&target) {
        evidence.targets.push(target.clone());
    }
    let module_id = semantic_id(
        "frontend-module",
        &format!("{source}#{}", ir.semantic_id.value),
    );
    evidence.upsert_node(node(
        module_id.clone(),
        EvidenceNodeKind::FrontendModule,
        &ir.module,
        vec![target.clone()],
        BTreeMap::from([
            ("format".into(), json!(ir.format)),
            ("source".into(), json!(source)),
        ]),
        reference.clone(),
        None,
    ));
    for local in &ir.local_modules {
        let local_path = local_graph_path(source, &local.path);
        let local_id = semantic_id("local-acore-module", &local_path);
        evidence.upsert_node(node(
            local_id.clone(),
            EvidenceNodeKind::SourceUnit,
            &local_path,
            vec![target.clone()],
            BTreeMap::from([
                ("sha256".into(), json!(local.sha256)),
                ("compileTimeOnly".into(), json!(true)),
                ("exports".into(), json!(local.exports)),
            ]),
            EvidenceReference {
                kind: "source".into(),
                path: Some(local_path.clone()),
                sha256: Some(local.sha256.clone()),
                detail: Some("local Acore module".into()),
            },
            None,
        ));
        add_edge(
            evidence,
            &module_id,
            &local_id,
            EvidenceEdgeKind::Contains,
            vec![target.clone()],
        );
        for import in &local.imports {
            let dependency = semantic_id(
                "local-acore-module",
                &local_graph_path(source, &import.resolved_path),
            );
            add_edge(
                evidence,
                &local_id,
                &dependency,
                EvidenceEdgeKind::Imports,
                vec![target.clone()],
            );
        }
    }
    if let Some(application) = evidence
        .nodes
        .iter()
        .find(|node| node.kind == EvidenceNodeKind::Application)
        .map(|node| node.id.clone())
    {
        add_edge(
            evidence,
            &application,
            &module_id,
            EvidenceEdgeKind::Contains,
            vec![target.clone()],
        );
    }
    if let Some(source_node) = evidence
        .nodes
        .iter()
        .find(|node| node.kind == EvidenceNodeKind::SourceUnit && node.label == source)
        .map(|node| node.id.clone())
    {
        add_edge(
            evidence,
            &module_id,
            &source_node,
            EvidenceEdgeKind::SourcedFrom,
            vec![target.clone()],
        );
    }

    for import in &ir.imports {
        if let Some(contract) = evidence
            .nodes
            .iter()
            .find(|node| {
                node.kind == EvidenceNodeKind::Contract
                    && node.label == import.alias
                    && node.attributes.get("artifactSha256") == Some(&json!(import.artifact_sha256))
            })
            .map(|node| node.id.clone())
        {
            add_edge(
                evidence,
                &module_id,
                &contract,
                EvidenceEdgeKind::Imports,
                vec![target.clone()],
            );
        }
    }
    for import in &ir.extension_imports {
        if let Some(extension) = evidence
            .nodes
            .iter()
            .find(|node| node.kind == EvidenceNodeKind::Extension && node.label == import.alias)
            .map(|node| node.id.clone())
        {
            add_edge(
                evidence,
                &module_id,
                &extension,
                EvidenceEdgeKind::Imports,
                vec![target.clone()],
            );
        }
    }

    let mut route_ids = BTreeMap::new();
    for route in &ir.routes {
        let id = semantic_id("route", &format!("{source}#{}", route.semantic_id.value));
        route_ids.insert(route.path.clone(), id.clone());
        evidence.upsert_node(node(
            id.clone(),
            EvidenceNodeKind::Route,
            &route.path,
            vec![target.clone()],
            BTreeMap::from([
                ("renderedSemanticId".into(), json!(route.semantic_id.value)),
                ("page".into(), json!(route.page)),
            ]),
            reference.clone(),
            None,
        ));
        add_edge(
            evidence,
            &module_id,
            &id,
            EvidenceEdgeKind::Declares,
            vec![target.clone()],
        );
    }

    for page in &ir.pages {
        let page_id = semantic_id("page", &format!("{source}#{}", page.semantic_id.value));
        evidence.upsert_node(node(
            page_id.clone(),
            EvidenceNodeKind::Page,
            &page.name,
            vec![target.clone()],
            BTreeMap::from([("renderedSemanticId".into(), json!(page.semantic_id.value))]),
            semantic_reference(ir, &page.semantic_id, source, reference),
            Some(&page.span),
        ));
        add_edge(
            evidence,
            &module_id,
            &page_id,
            EvidenceEdgeKind::Declares,
            vec![target.clone()],
        );
        for route in &ir.routes {
            if route.page == page.name {
                if let Some(route_id) = route_ids.get(&route.path) {
                    add_edge(
                        evidence,
                        route_id,
                        &page_id,
                        EvidenceEdgeKind::ResolvesTo,
                        vec![target.clone()],
                    );
                }
            }
        }
        let mut state_ids = BTreeMap::new();
        for state in &page.states {
            let id = semantic_id("state", &format!("{source}#{}", state.semantic_id.value));
            let state_path = state.path();
            state_ids.insert(state_path.clone(), id.clone());
            evidence.upsert_node(node(
                id.clone(),
                EvidenceNodeKind::State,
                &format!("{}.{}", page.name, state_path),
                vec![target.clone()],
                BTreeMap::from([
                    ("renderedSemanticId".into(), json!(state.semantic_id.value)),
                    ("name".into(), json!(state.name)),
                    ("scope".into(), json!(state.scope)),
                    ("path".into(), json!(state_path)),
                    ("type".into(), json!(state.declared_type)),
                    ("initializer".into(), json!(state.initializer)),
                ]),
                semantic_reference(ir, &state.semantic_id, source, reference),
                Some(&state.span),
            ));
            add_edge(
                evidence,
                &page_id,
                &id,
                EvidenceEdgeKind::Declares,
                vec![target.clone()],
            );
        }
        let mut operation_ids = BTreeMap::new();
        for operation in &page.operations {
            let id = semantic_id(
                "ui-operation",
                &format!("{source}#{}", operation.semantic_id.value),
            );
            operation_ids.insert(operation.name.clone(), id.clone());
            evidence.upsert_node(node(
                id.clone(),
                EvidenceNodeKind::Operation,
                &format!("{}.{}", page.name, operation.name),
                vec![target.clone()],
                BTreeMap::from([
                    (
                        "renderedSemanticId".into(),
                        json!(operation.semantic_id.value),
                    ),
                    ("frontend".into(), json!(true)),
                    ("binding".into(), json!(operation)),
                    (
                        "contractLocalName".into(),
                        json!(operation.contract_local_name),
                    ),
                    ("contractOperation".into(), json!(operation.operation)),
                ]),
                semantic_reference(ir, &operation.semantic_id, source, reference),
                Some(&operation.span),
            ));
            add_edge(
                evidence,
                &page_id,
                &id,
                EvidenceEdgeKind::Declares,
                vec![target.clone()],
            );
            if let Some(contract_operation) = contract_operation(
                evidence,
                ir,
                &operation.contract_local_name,
                &operation.operation,
            ) {
                add_edge(
                    evidence,
                    &id,
                    &contract_operation,
                    EvidenceEdgeKind::Calls,
                    vec![target.clone()],
                );
            }
            for (state, state_id) in &state_ids {
                if expression_references_identifier(&operation.arguments, state) {
                    add_edge(
                        evidence,
                        &id,
                        state_id,
                        EvidenceEdgeKind::Reads,
                        vec![target.clone()],
                    );
                }
            }
        }
        let mut action_ids = BTreeMap::new();
        for action in &page.actions {
            let action_id =
                semantic_id("action", &format!("{source}#{}", action.semantic_id.value));
            action_ids.insert(action.name.clone(), action_id.clone());
            evidence.upsert_node(node(
                action_id.clone(),
                EvidenceNodeKind::Action,
                &format!("{}.{}", page.name, action.name),
                vec![target.clone()],
                BTreeMap::from([("renderedSemanticId".into(), json!(action.semantic_id.value))]),
                semantic_reference(ir, &action.semantic_id, source, reference),
                Some(&action.span),
            ));
            add_edge(
                evidence,
                &page_id,
                &action_id,
                EvidenceEdgeKind::Declares,
                vec![target.clone()],
            );
            for (index, step) in action.steps.iter().enumerate() {
                let (label, span) = step_label(step);
                let step_id = semantic_id("action-step", &format!("{action_id}/{index}/{label}"));
                evidence.upsert_node(node(
                    step_id.clone(),
                    EvidenceNodeKind::ActionStep,
                    &label,
                    vec![target.clone()],
                    BTreeMap::from([("step".into(), serde_json::to_value(step)?)]),
                    semantic_reference(ir, &action.semantic_id, source, reference),
                    Some(span),
                ));
                add_edge(
                    evidence,
                    &action_id,
                    &step_id,
                    EvidenceEdgeKind::Contains,
                    vec![target.clone()],
                );
                match step {
                    UiActionStep::OperationRun { operation, .. } => {
                        if let Some(id) = operation_ids.get(operation) {
                            add_edge(
                                evidence,
                                &step_id,
                                id,
                                EvidenceEdgeKind::Calls,
                                vec![target.clone()],
                            );
                        }
                    }
                    UiActionStep::ExtensionInvoke { alias, input, .. } => {
                        if let Some(id) = evidence
                            .nodes
                            .iter()
                            .find(|node| {
                                node.kind == EvidenceNodeKind::Extension && node.label == *alias
                            })
                            .map(|node| node.id.clone())
                        {
                            add_edge(
                                evidence,
                                &step_id,
                                &id,
                                EvidenceEdgeKind::Invokes,
                                vec![target.clone()],
                            );
                        }
                        for (state, state_id) in &state_ids {
                            if expression_references_identifier(input, state) {
                                add_edge(
                                    evidence,
                                    &step_id,
                                    state_id,
                                    EvidenceEdgeKind::Reads,
                                    vec![target.clone()],
                                );
                            }
                        }
                    }
                    UiActionStep::StateAssign {
                        state, expression, ..
                    } => {
                        if let Some(id) = state_ids.get(state) {
                            add_edge(
                                evidence,
                                &step_id,
                                id,
                                EvidenceEdgeKind::Writes,
                                vec![target.clone()],
                            );
                        }
                        for (candidate, state_id) in &state_ids {
                            if expression_references_identifier(expression, candidate) {
                                add_edge(
                                    evidence,
                                    &step_id,
                                    state_id,
                                    EvidenceEdgeKind::Reads,
                                    vec![target.clone()],
                                );
                            }
                        }
                    }
                    UiActionStep::Navigate { path, .. } => {
                        if let Some(id) = route_ids.get(path) {
                            add_edge(
                                evidence,
                                &step_id,
                                id,
                                EvidenceEdgeKind::NavigatesTo,
                                vec![target.clone()],
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        for view in &page.view {
            add_view(
                evidence,
                view,
                &page_id,
                &action_ids,
                &state_ids,
                &target,
                source,
                reference,
                ir,
            )?;
        }
    }

    for component in &ir.components {
        let id = semantic_id(
            "component",
            &format!("{source}#{}", component.semantic_id.value),
        );
        evidence.upsert_node(node(
            id.clone(),
            EvidenceNodeKind::Component,
            &component.name,
            vec![target.clone()],
            BTreeMap::from([
                (
                    "renderedSemanticId".into(),
                    json!(component.semantic_id.value),
                ),
                ("props".into(), json!(component.props)),
            ]),
            semantic_reference(ir, &component.semantic_id, source, reference),
            Some(&component.span),
        ));
        add_edge(
            evidence,
            &module_id,
            &id,
            EvidenceEdgeKind::Declares,
            vec![target.clone()],
        );
        for view in &component.view {
            add_view(
                evidence,
                view,
                &id,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &target,
                source,
                reference,
                ir,
            )?;
        }
    }
    for derived in &ir.derived {
        let id = semantic_id(
            "derived",
            &format!("{source}#{}", derived.semantic_id.value),
        );
        evidence.upsert_node(node(
            id.clone(),
            EvidenceNodeKind::DerivedValue,
            &derived.name,
            vec![target.clone()],
            BTreeMap::from([
                (
                    "renderedSemanticId".into(),
                    json!(derived.semantic_id.value),
                ),
                ("expression".into(), json!(derived.expression)),
            ]),
            semantic_reference(ir, &derived.semantic_id, source, reference),
            Some(&derived.span),
        ));
        add_edge(
            evidence,
            &module_id,
            &id,
            EvidenceEdgeKind::Declares,
            vec![target.clone()],
        );
    }
    for effect in &ir.effects {
        let id = semantic_id("effect", &format!("{source}#{}", effect.semantic_id.value));
        evidence.upsert_node(node(
            id.clone(),
            EvidenceNodeKind::Effect,
            &effect.name,
            vec![target.clone()],
            BTreeMap::from([
                ("renderedSemanticId".into(), json!(effect.semantic_id.value)),
                ("trigger".into(), json!(effect.trigger)),
            ]),
            semantic_reference(ir, &effect.semantic_id, source, reference),
            Some(&effect.span),
        ));
        add_edge(
            evidence,
            &module_id,
            &id,
            EvidenceEdgeKind::Declares,
            vec![target.clone()],
        );
    }
    for asset in &ir.assets {
        let id = semantic_id("asset", &format!("{source}#{}", asset.semantic_id.value));
        evidence.upsert_node(node(
            id.clone(),
            EvidenceNodeKind::Asset,
            &asset.path,
            vec![target.clone()],
            BTreeMap::from([("sha256".into(), json!(asset.sha256))]),
            semantic_reference(ir, &asset.semantic_id, source, reference),
            Some(&asset.span),
        ));
        add_edge(
            evidence,
            &module_id,
            &id,
            EvidenceEdgeKind::Declares,
            vec![target.clone()],
        );
    }
    for (sheet_index, sheet) in ir.stylesheets.iter().enumerate() {
        for variable in &sheet.variables {
            let id = semantic_id(
                "style-variable",
                &format!("{source}/{sheet_index}/{}", variable.name),
            );
            evidence.upsert_node(node(
                id.clone(),
                EvidenceNodeKind::StyleVariable,
                &variable.name,
                vec![target.clone()],
                BTreeMap::from([("value".into(), serde_json::to_value(&variable.value)?)]),
                reference.clone(),
                Some(&variable.span),
            ));
            add_edge(
                evidence,
                &module_id,
                &id,
                EvidenceEdgeKind::Declares,
                vec![target.clone()],
            );
        }
        for rule in &sheet.rules {
            let id = semantic_id(
                "style-rule",
                &format!("{source}/{sheet_index}/{}", rule.source_order),
            );
            evidence.upsert_node(node(
                id.clone(),
                EvidenceNodeKind::StyleRule,
                &rule
                    .selectors
                    .iter()
                    .map(|value| value.authored.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                vec![target.clone()],
                BTreeMap::from([
                    ("selectors".into(), json!(rule.selectors)),
                    ("declarations".into(), json!(rule.declarations)),
                    ("sourceOrder".into(), json!(rule.source_order)),
                    ("viewport".into(), json!(rule.viewport)),
                ]),
                reference.clone(),
                Some(&rule.span),
            ));
            add_edge(
                evidence,
                &module_id,
                &id,
                EvidenceEdgeKind::Declares,
                vec![target.clone()],
            );
            let selectors = rule
                .selectors
                .iter()
                .map(|selector| selector.canonical.as_str())
                .collect::<BTreeSet<_>>();
            let styled = evidence
                .nodes
                .iter()
                .filter(|node| node.kind == EvidenceNodeKind::Primitive)
                .filter(|node| {
                    node.evidence
                        .iter()
                        .any(|item| item.path.as_deref() == Some(source))
                })
                .filter(|node| {
                    primitive_classes(node)
                        .iter()
                        .any(|class| selectors.contains(format!(".{class}").as_str()))
                })
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            for primitive in styled {
                add_edge(
                    evidence,
                    &primitive,
                    &id,
                    EvidenceEdgeKind::DependsOn,
                    vec![target.clone()],
                );
            }
            if let Some(viewport) = rule.viewport {
                let responsive = semantic_id("responsive", &format!("{id}/{}", viewport.as_str()));
                evidence.upsert_node(node(
                    responsive.clone(),
                    EvidenceNodeKind::ResponsiveBranch,
                    viewport.as_str(),
                    vec![target.clone()],
                    BTreeMap::from([("viewport".into(), json!(viewport))]),
                    reference.clone(),
                    Some(&rule.span),
                ));
                add_edge(
                    evidence,
                    &id,
                    &responsive,
                    EvidenceEdgeKind::Contains,
                    vec![target.clone()],
                );
            }
        }
    }
    Ok(())
}

fn local_graph_path(entry: &str, local: &str) -> String {
    let parent = Path::new(entry).parent().unwrap_or_else(|| Path::new(""));
    slash(&parent.join(local))
}

fn semantic_reference(
    ir: &UiIr,
    semantic: &axiom_ui::SemanticId,
    entry: &str,
    fallback: &EvidenceReference,
) -> EvidenceReference {
    ir.source_evidence
        .iter()
        .find(|item| item.semantic_id == *semantic)
        .map(|item| EvidenceReference {
            kind: "source-span".into(),
            path: Some(local_graph_path(entry, &item.source_path)),
            sha256: Some(item.source_sha256.clone()),
            detail: None,
        })
        .unwrap_or_else(|| fallback.clone())
}

fn add_view(
    evidence: &mut ApplicationEvidence,
    value: &UiNode,
    owner: &str,
    actions: &BTreeMap<String, String>,
    states: &BTreeMap<String, String>,
    target: &str,
    source: &str,
    reference: &EvidenceReference,
    ir: &UiIr,
) -> Result<()> {
    let id = semantic_id(
        "primitive",
        &format!("{source}#{}", value.semantic_id.value),
    );
    let label = value
        .component_name
        .clone()
        .unwrap_or_else(|| format!("{:?}", value.primitive));
    let standard = standard_component_contract(&value.primitive).and_then(|expected| {
        ir.standard_components
            .iter()
            .find(|contract| contract.name == expected.name)
            .cloned()
    });
    evidence.upsert_node(node(
        id.clone(),
        EvidenceNodeKind::Primitive,
        &label,
        vec![target.into()],
        BTreeMap::from([
            ("renderedSemanticId".into(), json!(value.semantic_id.value)),
            ("primitive".into(), serde_json::to_value(&value.primitive)?),
            ("component".into(), json!(value.component_name)),
            ("properties".into(), json!(value.properties)),
            ("condition".into(), json!(value.condition)),
            ("iterator".into(), json!(value.iterator)),
            ("authoredComponent".into(), json!(label)),
            ("expandedComponentContract".into(), json!(standard)),
        ]),
        semantic_reference(ir, &value.semantic_id, source, reference),
        Some(&value.span),
    ));
    add_edge(
        evidence,
        owner,
        &id,
        EvidenceEdgeKind::Renders,
        vec![target.into()],
    );
    for property in &value.properties {
        for (state, state_id) in states {
            if expression_references_identifier(&property.expression, state) {
                add_edge(
                    evidence,
                    &id,
                    state_id,
                    EvidenceEdgeKind::Reads,
                    vec![target.into()],
                );
            }
        }
        if matches!(
            property.name.as_str(),
            "accessibility_label" | "accessibility_role" | "accessibility_hint" | "semantic_id"
        ) {
            let accessibility = semantic_id("accessibility", &format!("{id}/{}", property.name));
            evidence.upsert_node(node(
                accessibility.clone(),
                EvidenceNodeKind::Accessibility,
                &property.name,
                vec![target.into()],
                BTreeMap::from([("value".into(), json!(property.expression))]),
                reference.clone(),
                Some(&property.span),
            ));
            add_edge(
                evidence,
                &id,
                &accessibility,
                EvidenceEdgeKind::Declares,
                vec![target.into()],
            );
        }
        if property.name.starts_with("on_") {
            let expression = property.expression.trim();
            let action_name = expression.strip_suffix("()").unwrap_or(expression);
            if let Some(action) = actions.get(action_name) {
                add_edge(
                    evidence,
                    &id,
                    action,
                    EvidenceEdgeKind::Triggers,
                    vec![target.into()],
                );
            }
        }
    }
    for child in &value.children {
        add_view(
            evidence, child, &id, actions, states, target, source, reference, ir,
        )?;
    }
    for branch in &value.else_if {
        for child in &branch.children {
            add_view(
                evidence, child, &id, actions, states, target, source, reference, ir,
            )?;
        }
    }
    for child in &value.else_children {
        add_view(
            evidence, child, &id, actions, states, target, source, reference, ir,
        )?;
    }
    Ok(())
}

fn contract_operation(
    evidence: &ApplicationEvidence,
    ir: &UiIr,
    local: &str,
    operation: &str,
) -> Option<String> {
    let import = ir.imports.iter().find(|value| value.local_name == local)?;
    let contract = evidence.nodes.iter().find(|node| {
        node.kind == EvidenceNodeKind::Contract
            && node.label == import.alias
            && node.attributes.get("artifactSha256") == Some(&json!(import.artifact_sha256))
    })?;
    evidence
        .edges
        .iter()
        .filter(|edge| edge.from == contract.id && edge.kind == EvidenceEdgeKind::Exposes)
        .find_map(|edge| {
            evidence
                .nodes
                .iter()
                .find(|node| node.id == edge.to && node.label == operation)
                .map(|node| node.id.clone())
        })
}

fn add_cross_layer_edges(evidence: &mut ApplicationEvidence) {
    let backend = evidence
        .nodes
        .iter()
        .filter(|node| {
            node.kind == EvidenceNodeKind::Operation
                && node.attributes.get("backend") == Some(&json!(true))
        })
        .map(|node| (node.label.to_lowercase(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let contract_operations = evidence
        .nodes
        .iter()
        .filter(|node| {
            node.kind == EvidenceNodeKind::Operation && node.attributes.contains_key("surface")
        })
        .map(|node| (node.label.to_lowercase(), node.id.clone()))
        .collect::<Vec<_>>();
    for (name, contract) in contract_operations {
        if let Some(server) = backend.get(&name) {
            add_edge(
                evidence,
                &contract,
                server,
                EvidenceEdgeKind::ResolvesTo,
                vec!["server".into()],
            );
        }
    }
    let states = evidence
        .nodes
        .iter()
        .filter(|node| node.kind == EvidenceNodeKind::State)
        .filter_map(|node| {
            node.attributes
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(|name| (name.to_string(), node.id.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let authority = evidence
        .nodes
        .iter()
        .filter(|node| node.kind == EvidenceNodeKind::Permission)
        .filter_map(|node| {
            let permission = node.attributes.get("permission")?;
            if permission.get("kind")?.as_str()? != "ui-state" {
                return None;
            }
            Some((
                node.id.clone(),
                permission.get("path")?.as_str()?.to_string(),
                permission.get("access")?.as_str()?.to_string(),
                node.targets.clone(),
            ))
        })
        .collect::<Vec<_>>();
    for (permission, path, access, targets) in authority {
        if let Some(state) = states.get(&path) {
            let kind = match access.as_str() {
                "read" => EvidenceEdgeKind::Reads,
                "write" => EvidenceEdgeKind::Writes,
                "subscribe" => EvidenceEdgeKind::Subscribes,
                "dispatch" => EvidenceEdgeKind::Dispatches,
                _ => continue,
            };
            add_edge(evidence, &permission, state, kind, targets);
        }
    }
    let operations = evidence
        .nodes
        .iter()
        .filter(|node| node.kind == EvidenceNodeKind::Operation)
        .map(|node| (node.label.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let contract_permissions = evidence
        .nodes
        .iter()
        .filter(|node| node.kind == EvidenceNodeKind::Permission)
        .filter_map(|node| {
            let permission = node.attributes.get("permission")?;
            (permission.get("kind")?.as_str()? == "contract-runtime").then(|| {
                (
                    node.id.clone(),
                    permission
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    node.targets.clone(),
                )
            })
        })
        .collect::<Vec<_>>();
    for (permission, path, targets) in contract_permissions {
        let operation = path.rsplit('.').next().unwrap_or(&path);
        if let Some(operation_id) = operations.get(operation) {
            add_edge(
                evidence,
                &permission,
                operation_id,
                EvidenceEdgeKind::Calls,
                targets,
            );
        }
    }
    let package_permissions = evidence
        .edges
        .iter()
        .filter(|edge| edge.kind == EvidenceEdgeKind::Impacts)
        .filter(|edge| {
            evidence
                .nodes
                .iter()
                .find(|node| node.id == edge.from)
                .is_some_and(|node| node.kind == EvidenceNodeKind::ThirdPartyPackage)
                && evidence
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.to)
                    .is_some_and(|node| node.kind == EvidenceNodeKind::Permission)
        })
        .map(|edge| (edge.from.clone(), edge.to.clone(), edge.targets.clone()))
        .collect::<Vec<_>>();
    let permission_effects = evidence
        .edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.kind,
                EvidenceEdgeKind::Reads
                    | EvidenceEdgeKind::Writes
                    | EvidenceEdgeKind::Subscribes
                    | EvidenceEdgeKind::Dispatches
                    | EvidenceEdgeKind::Calls
            )
        })
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect::<Vec<_>>();
    for (package, permission, targets) in package_permissions {
        for (_, affected) in permission_effects
            .iter()
            .filter(|(source, _)| source == &permission)
        {
            add_edge(
                evidence,
                &package,
                affected,
                EvidenceEdgeKind::Impacts,
                targets.clone(),
            );
        }
    }
    let mut field_candidates = BTreeMap::<String, Vec<String>>::new();
    for field in evidence
        .nodes
        .iter()
        .filter(|node| node.kind == EvidenceNodeKind::Field)
    {
        if let Some(name) = field
            .attributes
            .get("name")
            .and_then(serde_json::Value::as_str)
        {
            field_candidates
                .entry(name.into())
                .or_default()
                .push(field.id.clone());
        }
    }
    let unique_fields = field_candidates
        .into_iter()
        .filter_map(|(name, ids)| (ids.len() == 1).then(|| (name, ids[0].clone())))
        .collect::<Vec<_>>();
    let primitives = evidence
        .nodes
        .iter()
        .filter(|node| node.kind == EvidenceNodeKind::Primitive)
        .map(|node| {
            (
                node.id.clone(),
                node.targets.clone(),
                node_property_expressions(node),
            )
        })
        .collect::<Vec<_>>();
    for (primitive, targets, expressions) in primitives {
        for (field, field_id) in &unique_fields {
            if expressions
                .iter()
                .any(|expression| expression_references_identifier(expression, field))
            {
                add_edge(
                    evidence,
                    field_id,
                    &primitive,
                    EvidenceEdgeKind::Impacts,
                    targets.clone(),
                );
            }
        }
    }
}

fn add_readiness(evidence: &mut ApplicationEvidence) {
    let mut blockers = evidence
        .nodes
        .iter()
        .filter(|node| {
            node.kind == EvidenceNodeKind::Finding
                && node.attributes.get("severity") == Some(&json!("error"))
        })
        .count();
    let mut warnings = evidence
        .nodes
        .iter()
        .filter(|node| {
            node.kind == EvidenceNodeKind::Finding
                && node.attributes.get("severity") == Some(&json!("warning"))
        })
        .count();
    let application = evidence
        .nodes
        .iter()
        .find(|node| node.kind == EvidenceNodeKind::Application)
        .map(|node| node.id.clone());
    let candidates = evidence.nodes.clone();
    for value in candidates {
        let finding = if value.kind == EvidenceNodeKind::Package
            && value.attributes.get("signed") == Some(&json!(false))
        {
            Some(("unsigned-package", "warning", "Package is not signed"))
        } else if value.kind == EvidenceNodeKind::Extension
            && value.attributes.get("developmentOnly") == Some(&json!(true))
        {
            Some((
                "development-extension",
                "warning",
                "Extension is development-only",
            ))
        } else if value.kind == EvidenceNodeKind::Operation
            && value.attributes.get("exposure") == Some(&json!("public"))
        {
            Some(("public-endpoint", "info", "Endpoint is explicitly public"))
        } else {
            None
        };
        if let Some((rule, severity, message)) = finding {
            if severity == "error" {
                blockers += 1;
            } else if severity == "warning" {
                warnings += 1;
            }
            let id = semantic_id("finding", &format!("{rule}/{}", value.id));
            evidence.upsert_node(node(
                id.clone(),
                EvidenceNodeKind::Finding,
                rule,
                value.targets.clone(),
                BTreeMap::from([
                    ("rule".into(), json!(rule)),
                    ("severity".into(), json!(severity)),
                    ("message".into(), json!(message)),
                    ("subject".into(), json!(value.id)),
                ]),
                EvidenceReference {
                    kind: "derived-from-verified-fact".into(),
                    path: None,
                    sha256: None,
                    detail: Some(value.id.clone()),
                },
                None,
            ));
            add_edge(
                evidence,
                &value.id,
                &id,
                EvidenceEdgeKind::Declares,
                value.targets,
            );
        }
    }
    let id = semantic_id("readiness", &evidence.workspace);
    evidence.upsert_node(node(
        id.clone(),
        EvidenceNodeKind::ReadinessFact,
        "production readiness",
        vec![],
        BTreeMap::from([
            ("ready".into(), json!(blockers == 0)),
            ("blockers".into(), json!(blockers)),
            ("warnings".into(), json!(warnings)),
        ]),
        EvidenceReference {
            kind: "deterministic-policy".into(),
            path: None,
            sha256: None,
            detail: Some("axiom-inspector-readiness/v1".into()),
        },
        None,
    ));
    if let Some(application) = application {
        add_edge(
            evidence,
            &application,
            &id,
            EvidenceEdgeKind::Declares,
            vec![],
        );
    }
}

fn node(
    id: String,
    kind: EvidenceNodeKind,
    label: &str,
    targets: Vec<String>,
    mut attributes: BTreeMap<String, serde_json::Value>,
    reference: EvidenceReference,
    span: Option<&SourceSpan>,
) -> EvidenceNode {
    if let Some(span) = span {
        attributes.insert("span".into(), json!(span));
    }
    EvidenceNode {
        id,
        kind,
        label: label.into(),
        layer: TruthLayer::Resolved,
        verification: VerificationState::Verified,
        targets,
        attributes,
        evidence: vec![span
            .map(|value| span_reference(reference.path.as_deref().unwrap_or(""), value, &reference))
            .unwrap_or(reference)],
    }
}

fn add_edge(
    evidence: &mut ApplicationEvidence,
    from: &str,
    to: &str,
    kind: EvidenceEdgeKind,
    targets: Vec<String>,
) {
    evidence.upsert_edge(EvidenceEdge {
        id: edge_id(kind, from, to),
        from: from.into(),
        to: to.into(),
        kind,
        layer: TruthLayer::Resolved,
        targets,
        evidence: vec![],
    });
}

fn span_reference(path: &str, span: &SourceSpan, base: &EvidenceReference) -> EvidenceReference {
    EvidenceReference {
        kind: "source-span".into(),
        path: Some(path.into()),
        sha256: base.sha256.clone(),
        detail: Some(format!("{}..{}", span.start, span.end)),
    }
}

fn step_label(step: &UiActionStep) -> (String, &SourceSpan) {
    match step {
        UiActionStep::OperationRun { operation, span } => (format!("run {operation}"), span),
        UiActionStep::ExtensionInvoke {
            alias,
            export,
            span,
            ..
        } => (format!("invoke {alias}.{export}"), span),
        UiActionStep::StateAssign { state, span, .. } => (format!("write {state}"), span),
        UiActionStep::Navigate { path, span } => (format!("navigate {path}"), span),
        UiActionStep::NavigateBack { span } => ("navigate back".into(), span),
        UiActionStep::NodeInvoke {
            node_id,
            method,
            span,
            ..
        } => (format!("invoke {node_id}.{method}"), span),
    }
}

fn slash(path: &Path) -> String {
    path.components()
        .map(|value| value.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn primitive_classes(node: &EvidenceNode) -> Vec<String> {
    node.attributes
        .get("properties")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|property| {
            property.get("name").and_then(serde_json::Value::as_str) == Some("class")
        })
        .filter_map(|property| {
            property
                .get("expression")
                .and_then(serde_json::Value::as_str)
        })
        .filter_map(|expression| serde_json::from_str::<String>(expression).ok())
        .flat_map(|classes| {
            classes
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn node_property_expressions(node: &EvidenceNode) -> Vec<String> {
    node.attributes
        .get("properties")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|property| {
            property
                .get("expression")
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
