use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use axiom_lib::{
    application_evidence::{ApplicationEvidence, EvidenceNodeKind},
    runtime_evidence::{
        normalize_extension_audit, AuditBundle, RuntimeCaptureState, RuntimeSession,
        AUDIT_BUNDLE_FORMAT, RUNTIME_EVIDENCE_FORMAT,
    },
};
use serde_json::Value;
use uuid::Uuid;

use super::inspector::InspectorFormat;

const SESSION_FILE: &str = "session.jcs.json";
const EVENT_FILE: &str = "events.jsonl";

pub(super) fn record(
    workspace: &Path,
    target: &str,
    input: Option<&Path>,
    requested_id: Option<&str>,
    disabled: bool,
) -> Result<()> {
    if disabled && input.is_some() {
        bail!("--disabled cannot be combined with --input");
    }
    let evidence = super::inspector::load(workspace)?;
    let session_id = requested_id
        .map(validate_session_id)
        .transpose()?
        .map(str::to_owned)
        .unwrap_or_else(|| format!("session-{}", Uuid::new_v4()));
    let mut session = if disabled {
        RuntimeSession::empty(
            &session_id,
            application_name(&evidence),
            &evidence.graph_revision,
            target,
            RuntimeCaptureState::Disabled,
            env!("CARGO_PKG_VERSION"),
        )
    } else if let Some(input) = input {
        normalize_input(input, &session_id, &evidence)?
    } else {
        let mut session = RuntimeSession::empty(
            &session_id,
            application_name(&evidence),
            &evidence.graph_revision,
            target,
            RuntimeCaptureState::Recording,
            env!("CARGO_PKG_VERSION"),
        );
        session.notes.push(
            "capture is attached and no event has been observed; ingest events through the local Inspector API"
                .into(),
        );
        session
    };
    if session.target != target {
        bail!(
            "audit target `{}` does not match requested target `{target}`",
            session.target
        );
    }
    session.session_id = session_id;
    reidentify_events(&mut session);
    persist(workspace, &session)?;
    print_recorded(&session, workspace)?;
    Ok(())
}

pub(super) fn import(workspace: &Path, input: &Path, requested_id: Option<&str>) -> Result<()> {
    let evidence = super::inspector::load(workspace)?;
    let session_id = requested_id
        .map(validate_session_id)
        .transpose()?
        .map(str::to_owned)
        .unwrap_or_else(|| format!("session-{}", Uuid::new_v4()));
    let mut session = normalize_input(input, &session_id, &evidence)?;
    session.session_id = session_id;
    reidentify_events(&mut session);
    persist(workspace, &session)?;
    print_recorded(&session, workspace)
}

pub(super) fn render_session(
    workspace: &Path,
    selector: &str,
    format: InspectorFormat,
) -> Result<()> {
    let session = load_session(workspace, selector)?;
    match format {
        InspectorFormat::Json => println!("{}", serde_json::to_string_pretty(&session)?),
        InspectorFormat::Jsonl => {
            println!(
                "{}",
                serde_json::json!({
                    "type": "session",
                    "sessionId": session.session_id,
                    "target": session.target,
                    "capture": session.capture,
                    "graphRevision": session.graph_revision,
                    "retention": session.retention,
                })
            );
            for event in &session.events {
                println!(
                    "{}",
                    serde_json::json!({"type": "runtime-event", "value": event})
                );
            }
        }
        InspectorFormat::Human => render_human(&session, None),
    }
    Ok(())
}

pub(super) fn render_trace(
    workspace: &Path,
    trace_id: &str,
    format: InspectorFormat,
) -> Result<()> {
    let session = latest_session_containing(workspace, trace_id)?;
    let events = session.traces(trace_id);
    if events.is_empty() {
        bail!("no observed runtime trace matches `{trace_id}`");
    }
    match format {
        InspectorFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "format": RUNTIME_EVIDENCE_FORMAT,
                "sessionId": session.session_id,
                "graphRevision": session.graph_revision,
                "traceId": trace_id,
                "events": events,
            }))?
        ),
        InspectorFormat::Jsonl => {
            for event in events {
                println!(
                    "{}",
                    serde_json::json!({"type": "runtime-event", "value": event})
                );
            }
        }
        InspectorFormat::Human => render_human(&session, Some(trace_id)),
    }
    Ok(())
}

pub(super) fn export_bundle(workspace: &Path, selector: &str, output: &Path) -> Result<()> {
    let session = load_session(workspace, selector)?;
    let bundle = AuditBundle::new(session)?;
    let bytes = bundle.canonical_bytes()?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, bytes).with_context(|| format!("write {}", output.display()))?;
    println!("Exported canonical redacted audit {}", output.display());
    println!("Runtime SHA-256 {}", bundle.runtime_sha256);
    Ok(())
}

pub(super) fn load_latest(workspace: &Path) -> Result<Option<RuntimeSession>> {
    match load_session(workspace, "latest") {
        Ok(session) => Ok(Some(session)),
        Err(error) if error.to_string().contains("no runtime sessions") => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn persist(workspace: &Path, session: &RuntimeSession) -> Result<()> {
    session.validate()?;
    let root = runtime_root(workspace)?;
    let directory = root.join(validate_session_id(&session.session_id)?);
    if directory.exists() {
        bail!("runtime session `{}` already exists", session.session_id);
    }
    fs::create_dir_all(&directory)?;
    let mut events = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join(EVENT_FILE))?;
    for event in &session.events {
        events.write_all(&serde_jcs::to_vec(event)?)?;
        events.write_all(b"\n")?;
    }
    events.sync_all()?;
    fs::write(directory.join(SESSION_FILE), session.canonical_bytes()?)?;
    fs::write(root.join("latest"), session.session_id.as_bytes())?;
    Ok(())
}

pub(super) fn runtime_root(workspace: &Path) -> Result<PathBuf> {
    let workspace = fs::canonicalize(workspace)
        .with_context(|| format!("resolve workspace {}", workspace.display()))?;
    let workspace = if workspace.is_file() {
        workspace.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        workspace
    };
    Ok(workspace.join(".axiom/inspector/runtime"))
}

fn load_session(workspace: &Path, selector: &str) -> Result<RuntimeSession> {
    let root = runtime_root(workspace)?;
    let id = if selector == "latest" {
        fs::read_to_string(root.join("latest")).context("no runtime sessions have been recorded")?
    } else {
        selector.to_owned()
    };
    let id = validate_session_id(id.trim())?;
    let bytes = fs::read(root.join(id).join(SESSION_FILE))
        .with_context(|| format!("read runtime session `{id}`"))?;
    let session = RuntimeSession::decode(&bytes)?;
    verify_append_log(&root.join(id).join(EVENT_FILE), &session)?;
    Ok(session)
}

fn latest_session_containing(workspace: &Path, trace_id: &str) -> Result<RuntimeSession> {
    let root = runtime_root(workspace)?;
    let mut candidates = fs::read_dir(&root)
        .context("no runtime sessions have been recorded")?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect::<Vec<_>>();
    candidates.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
    for candidate in candidates.into_iter().rev() {
        let path = candidate.path().join(SESSION_FILE);
        if let Ok(bytes) = fs::read(path) {
            if let Ok(session) = RuntimeSession::decode(&bytes) {
                if !session.traces(trace_id).is_empty() {
                    return Ok(session);
                }
            }
        }
    }
    bail!("no observed runtime trace matches `{trace_id}`")
}

fn normalize_input(
    input: &Path,
    session_id: &str,
    evidence: &ApplicationEvidence,
) -> Result<RuntimeSession> {
    let bytes = fs::read(input).with_context(|| format!("read {}", input.display()))?;
    let value: Value = serde_json::from_slice(&bytes).context("parse runtime audit JSON")?;
    match value.get("format").and_then(Value::as_str) {
        Some(RUNTIME_EVIDENCE_FORMAT) => RuntimeSession::decode(&bytes),
        Some(AUDIT_BUNDLE_FORMAT) => {
            let bundle: AuditBundle = serde_json::from_value(value)?;
            bundle.canonical_bytes()?;
            Ok(bundle.runtime)
        }
        Some("axiom-extension-audit-export/v1" | "axiom-extension-native-audit/v2") => {
            normalize_extension_audit(
                &value,
                session_id,
                &evidence.graph_revision,
                env!("CARGO_PKG_VERSION"),
            )
        }
        Some(format) => bail!("unsupported runtime input format `{format}`"),
        None => bail!("runtime input has no format discriminator"),
    }
}

fn verify_append_log(path: &Path, session: &RuntimeSession) -> Result<()> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let records = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<Vec<axiom_lib::runtime_evidence::RuntimeEvent>, _>>()?;
    if records != session.events {
        bail!("runtime append log does not match its canonical session");
    }
    Ok(())
}

fn application_name(evidence: &ApplicationEvidence) -> String {
    evidence
        .nodes
        .iter()
        .find(|node| node.kind == EvidenceNodeKind::Application)
        .map(|node| node.label.clone())
        .unwrap_or_else(|| evidence.workspace.clone())
}

fn reidentify_events(session: &mut RuntimeSession) {
    let id = session.session_id.clone();
    session.reidentify(id);
    session.retention.retained_records = session.events.len() as u64;
}

fn validate_session_id(id: &str) -> Result<&str> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!("runtime session ID must contain only ASCII letters, digits, '-' or '_'");
    }
    Ok(id)
}

fn print_recorded(session: &RuntimeSession, workspace: &Path) -> Result<()> {
    println!("Runtime session {}", session.session_id);
    println!("Target {}  Capture {:?}", session.target, session.capture);
    println!(
        "Observed events {}  Dropped {}  Complete {}",
        session.events.len(),
        session.retention.dropped_records,
        session.retention.complete
    );
    println!("Graph revision {}", session.graph_revision);
    println!("Stored in {}", runtime_root(workspace)?.display());
    Ok(())
}

fn render_human(session: &RuntimeSession, trace: Option<&str>) {
    let events = trace
        .map(|trace| session.traces(trace))
        .unwrap_or_else(|| session.events.clone());
    println!("Axiom Inspector Runtime");
    println!("Session {}  Target {}", session.session_id, session.target);
    println!("Graph revision {}", session.graph_revision);
    println!(
        "Capture {:?}  Retained {}  Dropped {}  Complete {}",
        session.capture,
        session.retention.retained_records,
        session.retention.dropped_records,
        session.retention.complete
    );
    if let Some(trace) = trace {
        println!("Trace {trace}");
    }
    if events.is_empty() {
        match session.capture {
            RuntimeCaptureState::Disabled => {
                println!("Capture was disabled; absence of events is not evidence of inactivity.")
            }
            _ => println!("Capture was enabled, but no matching event was observed."),
        }
    } else {
        println!();
        for event in events {
            println!(
                "{:>4}  {:<22?} {:<14} {}",
                event.sequence, event.kind, event.outcome, event.source
            );
            if let Some(trace) = event.trace_id {
                println!("      trace {trace}");
            }
            if !event.attributes.is_empty() {
                println!(
                    "      {}",
                    serde_json::to_string(&event.attributes).unwrap_or_default()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_session_names() {
        assert!(validate_session_id("../escape").is_err());
        assert!(validate_session_id("ok-session_1").is_ok());
    }
}
