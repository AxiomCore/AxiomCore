//! X9d: `axiom extensions replay <tape>` (contract `x9d-replay-tape.md`).
//!
//! Re-runs a dev replay tape against the *current* build of the extension
//! through the real kernel and broker, with a tape-backed host (`TapeHost`:
//! every host call is answered from the tape; no real host call, no network,
//! no state, no secrets). The replay is recorded in memory and compared with
//! the tape: plans (hash and content), outcomes, then the final result.
//! Exit code 0 when identical; a difference fails with
//! `AXIOM_EXTENSION_REPLAY_DIFFERS` and names the first differing call.
//!
//! `--to-scenario <file>` turns the tape into a scenario file in the format
//! `axiom extensions run --scenario` accepts (the X7 golden-harness format,
//! extended with a `replay` member holding the sealed tape), so a recorded
//! interaction becomes a regression scenario.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, Context, Result};
use axiom_extension_broker::{
    diff_tapes, write_private_file, InvocationContext, RecorderConfig, ReplayTape, TapeHost,
    TapeRecorder, TapeStatus,
};
use axiom_extension_host::LifecycleEvent;
use axiom_extension_kernel::CancellationToken;
use axiom_lib::{
    extension_workflow::{load_verified_extension_target, LoadedExtensionTarget},
    package::PackageTarget,
};

use super::extensions::{
    load_reference_host, now_unix_ms, parse_target, value_to_json, ReferenceHost,
};

/// Read and verify a tape file; a redacted tape is refused (it cannot replay).
fn read_tape(path: &Path) -> Result<ReplayTape> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("AXIOM_EXTENSION_REPLAY_TAPE: read {}", path.display()))?;
    let tape = ReplayTape::from_bytes(&bytes)
        .map_err(|error| anyhow::anyhow!("{error} ({})", path.display()))?;
    require_replayable(&tape)?;
    Ok(tape)
}

fn require_replayable(tape: &ReplayTape) -> Result<()> {
    if !tape.header.replayable {
        bail!(
            "AXIOM_EXTENSION_REPLAY_TAPE: the tape has {} redacted value(s) (sensitive outcomes) and cannot be replayed or turned into a scenario",
            tape.header.redactions.len()
        );
    }
    Ok(())
}

/// The scenario file for `tape` (see the module docs).
pub(crate) fn tape_scenario(tape: &ReplayTape) -> Result<serde_json::Value> {
    require_replayable(tape)?;
    let sealed: serde_json::Value = serde_json::from_slice(&tape.to_bytes()?)?;
    let digest = sealed["integrity"]["digest"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(8)
        .collect::<String>();
    Ok(serde_json::json!({
        "alias": tape.identity.alias.clone().unwrap_or_else(|| tape.identity.extension.clone()),
        "scenarios": [{
            "name": format!("replay-{}-{digest}", tape.invocation.export),
            "export": tape.invocation.export,
            // Readable view only; the run uses the tape's exact ABI input.
            "input": value_to_json(tape.invocation.input.clone()),
            "replay": sealed,
        }],
    }))
}

/// What a replay found.
pub(crate) struct ReplayOutcome {
    pub report: serde_json::Value,
    pub identical: bool,
    pub diff: Option<String>,
}

/// Replay `tape` against the verified build `loaded` (see the module docs).
pub(crate) async fn replay_against(
    loaded: &LoadedExtensionTarget,
    target: PackageTarget,
    tape: &ReplayTape,
    deadline_ms: u64,
) -> Result<ReplayOutcome> {
    let services = Arc::new(TapeHost::new(tape));
    let ReferenceHost {
        host,
        kernel: _kernel,
        sdk,
        module_sha256,
    } = load_reference_host(loaded, target, Arc::clone(&services), deadline_ms)?;
    // In memory only; nothing is written and no real host data exists here.
    let recorder = Arc::new(TapeRecorder::new(RecorderConfig::replay_comparison())?);
    host.set_tape_recorder(Some(Arc::clone(&recorder)));
    let token = CancellationToken::default();
    if tape.terminal.status == TapeStatus::Cancelled {
        if tape.host_exchanges.is_empty() {
            token.cancel();
        } else {
            services.cancel_when_drained(token.clone());
        }
    }
    let result = host
        .invoke_controlled(
            tape.invocation
                .to_invocation(now_unix_ms().saturating_add(deadline_ms)),
            InvocationContext::default(),
            None,
            token,
        )
        .await;
    let _ = host.lifecycle(LifecycleEvent::AppShutdown);
    let replayed = recorder.tapes().pop().with_context(|| {
        format!(
            "AXIOM_EXTENSION_REPLAY: the replay was not recorded ({:?})",
            recorder.last_outcome()
        )
    })?;
    let replayed = ReplayTape::from_bytes(&replayed)?;
    let diff = diff_tapes(tape, &replayed);
    let unmatched = services.unmatched();
    let identical = diff.is_none() && unmatched.is_empty();
    let report = serde_json::json!({
        "format": "axiom-extension-replay-report/v1",
        "alias": loaded.alias,
        "export": tape.invocation.export,
        "abi": tape.abi,
        "identical": identical,
        "difference": diff.as_ref().map(|d| serde_json::json!({"at": d.at, "recorded": d.recorded, "replayed": d.replayed})),
        "recorded": {"status": tape.terminal.status, "yields": tape.rounds.len(), "moduleSha256": tape.identity.module_sha256, "sdk": tape.identity.sdk},
        "replayed": {"status": replayed.terminal.status, "yields": replayed.rounds.len(), "moduleSha256": module_sha256, "sdk": sdk,
                     "error": result.as_ref().err().map(|error| error.to_string())},
        "moduleChanged": tape.identity.module_sha256.as_deref().is_some_and(|recorded| recorded != module_sha256),
        "budgetsChanged": tape.budgets != replayed.budgets,
        "unmatchedHostCalls": unmatched,
        "unusedHostReplies": services.unused(),
    });
    Ok(ReplayOutcome {
        report,
        identical,
        diff: diff.map(|d| d.to_string()),
    })
}

pub async fn handle_replay(
    tape_path: PathBuf,
    manifest: PathBuf,
    alias: Option<String>,
    target: String,
    to_scenario: Option<PathBuf>,
    deadline_ms: u64,
) -> Result<()> {
    let tape = read_tape(&tape_path)?;
    if let Some(out) = to_scenario {
        let scenario = tape_scenario(&tape)?;
        let mut bytes = serde_json::to_vec_pretty(&scenario)?;
        bytes.push(b'\n');
        write_private_file(&out, &bytes)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "format": "axiom-extension-replay-scenario/v1",
                "scenario": out,
                "alias": scenario["alias"],
                "name": scenario["scenarios"][0]["name"],
                "run": format!("axiom extensions run {} --scenario {}", scenario["alias"].as_str().unwrap_or_default(), out.display()),
            }))?
        );
        return Ok(());
    }
    let alias = alias.or_else(|| tape.identity.alias.clone()).unwrap_or_else(|| tape.identity.extension.clone());
    let target = parse_target(&target)?;
    if target != PackageTarget::Server {
        bail!("AXIOM_EXTENSION_REPLAY: replay runs in the server reference host (--target server)");
    }
    let loaded = load_verified_extension_target(&manifest, &alias, target)?;
    let outcome = replay_against(&loaded, target, &tape, deadline_ms).await?;
    println!("{}", serde_json::to_string_pretty(&outcome.report)?);
    if !outcome.identical {
        bail!(
            "AXIOM_EXTENSION_REPLAY_DIFFERS: {}",
            outcome.diff.unwrap_or_else(|| format!(
                "host calls not on the tape: {}",
                outcome.report["unmatchedHostCalls"]
            ))
        );
    }
    Ok(())
}

/// `axiom extensions run --scenario` for a scenario made from a tape.
pub(crate) async fn run_tape_scenario(
    loaded: &LoadedExtensionTarget,
    target: PackageTarget,
    tape: ReplayTape,
    deadline_ms: u64,
) -> Result<()> {
    require_replayable(&tape)?;
    let outcome = replay_against(loaded, target, &tape, deadline_ms).await?;
    let mut report = outcome.report;
    report["format"] = "axiom-extension-scenario-replay/v1".into();
    report["output"] = tape
        .terminal
        .result
        .as_ref()
        .map(|result| value_to_json(result.output.clone()))
        .unwrap_or(serde_json::Value::Null);
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !outcome.identical {
        bail!(
            "AXIOM_EXTENSION_REPLAY_DIFFERS: the scenario's recorded outcome was not reproduced: {}",
            outcome.diff.unwrap_or_else(|| format!("host calls not on the tape: {}", report["unmatchedHostCalls"]))
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenarios_from_redacted_tapes_are_refused() {
        // A minimal sealed tape (no rounds) round-trips into a scenario.
        let tape: ReplayTape = serde_json::from_value(serde_json::json!({
            "format": "axiom-extension-replay-tape", "version": 1,
            "header": {"notice": "n", "contains": [], "sensitive": "none-present", "redactions": [], "replayable": true, "recorder": "t"},
            "identity": {"application": "app", "extension": "pricing"},
            "abi": "1.1", "maxRounds": 1, "hostGroups": false,
            "budgets": {"maxHostCalls": 1, "maxReadBytes": 1, "maxWriteBytes": 1, "maxMutations": 1, "maxEvents": 1, "maxConcurrentStreams": 1},
            "invocation": {"export": "apply", "input": {"Unsigned": 4}, "snapshots": []},
            "rounds": [], "hostExchanges": [],
            "terminal": {"status": "completed", "result": {"output": {"Unsigned": 5}, "patches": [], "transactions": [], "emitted_events": []}, "committed": true},
            "usage": {"rounds": 1, "hostCalls": 0, "readBytes": 0, "writeBytes": 0}
        }))
        .unwrap();
        let scenario = tape_scenario(&tape).unwrap();
        assert_eq!(scenario["alias"], "pricing");
        assert_eq!(scenario["scenarios"][0]["export"], "apply");
        assert_eq!(scenario["scenarios"][0]["input"], 4);
        let embedded = serde_json::to_vec(&scenario["scenarios"][0]["replay"]).unwrap();
        assert_eq!(
            ReplayTape::from_bytes(&embedded).unwrap().invocation,
            tape.invocation
        );
        let mut redacted = tape;
        redacted.header.replayable = false;
        assert!(tape_scenario(&redacted)
            .unwrap_err()
            .to_string()
            .contains("redacted"));
    }
}
