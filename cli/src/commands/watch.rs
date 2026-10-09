use crate::commands::pull::handle_pull;
use crate::components::watch_hud::render_watch_hud;
use crate::state::{IRDiff, State};
use axiom_extractor::evaluate_acore_config_with_json;
use crossterm::event::KeyCode;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};

pub fn get_watch_context() -> Option<(PathBuf, Vec<PathBuf>)> {
    let acore_path = PathBuf::from("axiom.acore");
    if !acore_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&acore_path).ok()?;
    let targets = watch_targets(&acore_path, &content);
    Some((acore_path, targets))
}

fn watch_targets(source: &Path, content: &str) -> Vec<PathBuf> {
    let re = regex::Regex::new(r#"amends\s+"axiom-(fastapi|go):([^"]+)""#).unwrap();
    let mut targets = vec![source.to_path_buf()];
    if let Some(caps) = re.captures(&content) {
        let entrypoint = &caps[2];
        let file = if &caps[1] == "fastapi" {
            entrypoint
                .rsplit_once(':')
                .map_or(entrypoint, |(file, _)| file)
        } else {
            entrypoint
        };
        targets.push(source.parent().unwrap_or(Path::new(".")).join(file));
    }
    if let Ok(absolute) = source.canonicalize() {
        let uri = format!("file://{}", absolute.display());
        if let Some((manifest, _)) = acore::backend::audience::manifest(&uri, &Default::default()) {
            if let Some(path) = acore::backend::inspection::source_path(&manifest) {
                targets.push(path);
            }
        }
    }
    targets
}

pub async fn handle_watch_dynamic(build_flag: bool, variant: Option<String>) -> anyhow::Result<()> {
    let variant = variant.as_deref().unwrap_or("default");
    dotenvy::dotenv().ok();
    let context = get_watch_context();

    if context.is_none() {
        println!("🚀 Frontend Watch Mode: Coming Soon to Production!");
        return Ok(());
    }

    let (acore_path, targets) = context.unwrap();
    let mut state = crate::state::State::new();
    state.watch_build_enabled = build_flag;

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    // Setup Notify
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            tx.blocking_send(res).ok();
        },
        notify::Config::default(),
    )?;
    for path in targets {
        watcher.watch(&path, RecursiveMode::NonRecursive)?;
    }

    tui.enter().map_err(|e| anyhow::anyhow!(e))?;
    let result: anyhow::Result<()> = async {
        loop {
            tui.draw(|f| render_watch_hud(f, f.size(), &state))?;
            tokio::select! {
                Some(event) = tui.event_rx.recv() => {
                    if let crate::tui::Event::Key(key) = event {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Char('r') => {
                                refresh_backend(&mut state, &acore_path, variant, build_flag, Path::new("backend.axiom"));
                            }
                            _ => {}
                        }
                    }
                }
                Some(event) = rx.recv() => {
                    match event {
                        Ok(_) => refresh_backend(&mut state, &acore_path, variant, build_flag, Path::new("backend.axiom")),
                        Err(error) => state.watch_error = Some(error.to_string()),
                    }
                }
            }
        }
        Ok(())
    }.await;

    tui.exit().map_err(|e| anyhow::anyhow!(e))?;
    result
}

fn refresh_backend(state: &mut State, source: &Path, variant: &str, build: bool, output: &Path) {
    state.is_rebuilding = true;
    let result = (|| -> anyhow::Result<axiom_lib::ir::IR> {
        let path = source
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Acore path is not valid UTF-8"))?;
        let evaluated = evaluate_acore_config_with_json(path, Some(variant))?;
        let selected: axiom_lib::config::AxiomConfig = serde_json::from_str(&evaluated.json)?;
        let ir = selected.to_ir();
        if build {
            super::backend::build_evaluated(source, evaluated, variant, None, output, None)?;
        }
        Ok(ir)
    })();
    match result {
        Ok(ir) => {
            state.watch_diff = IRDiff::from_irs(&state.previous_ir, &ir);
            state.previous_ir = Some(ir);
            state.watch_error = None;
            state.last_sync_time = chrono::Local::now().format("%H:%M:%S").to_string();
            if build {
                if let Ok(bytes) = std::fs::read(output) {
                    if let Ok(artifact) = axiom_lib::unpackager::unpack_axiom_bytes(&bytes) {
                        state.last_schema_hash = artifact.project.schema_hash;
                    }
                }
            }
        }
        Err(error) => state.watch_error = Some(format!("{error:#}")),
    }
    state.is_rebuilding = false;
}

pub async fn handle_watch_consumer() -> anyhow::Result<()> {
    // 1. Ensure config exists (Run pull once)
    handle_pull(None, None, None, None, None, None).await?;

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    let mut state = State::new();

    // Load initial state for diffing
    if let Ok(bytes) = std::fs::read("project.axiom") {
        if let Ok(file) = axiom_lib::unpackager::unpack_axiom_bytes(&bytes) {
            state.previous_ir = Some(file.ir);
            state.last_schema_hash = file.project.schema_hash;
        }
    }

    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // Watch the artifact (or the directory if file swap happens)
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut watcher = notify::RecommendedWatcher::new(
        move |res| {
            let _ = tx.blocking_send(res);
        },
        notify::Config::default(),
    )?;

    // Watch current dir for .axiom changes
    watcher.watch(&std::env::current_dir()?, RecursiveMode::NonRecursive)?;

    loop {
        tui.draw(|f| render_watch_hud(f, f.size(), &state))?;

        tokio::select! {
            Some(event) = tui.event_rx.recv() => {
                if let crate::tui::Event::Key(key) = event {
                    if key.code == crossterm::event::KeyCode::Char('q') { break; }
                }
            }
            Some(Ok(event)) = rx.recv() => {
                // Check if project.axiom changed
                if event.paths.iter().any(|p| p.ends_with("project.axiom")) {
                    state.is_rebuilding = true;
                    tui.draw(|f| render_watch_hud(f, f.size(), &state))?;

                    // Reload and Diff
                    if let Ok(bytes) = std::fs::read("project.axiom") {
                        if let Ok(file) = axiom_lib::unpackager::unpack_axiom_bytes(&bytes) {
                            state.watch_diff = IRDiff::from_irs(&state.previous_ir, &file.ir);
                            state.previous_ir = Some(file.ir);
                            state.last_schema_hash = file.project.schema_hash;
                            state.last_sync_time = chrono::Local::now().format("%H:%M:%S").to_string();

                            // Re-run Codegen (Headless)
                            // Call post_pull_steps logic here...
                        }
                    }
                    state.is_rebuilding = false;
                }
            }
        }
    }
    tui.exit().map_err(|e| anyhow::anyhow!(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_watch_finds_python_and_go_entrypoints_beside_the_source() {
        let source = Path::new("backend/axiom.acore");
        for (module, expected) in [
            ("axiom-fastapi:main.py:app", "backend/main.py"),
            ("axiom-go:main.go", "backend/main.go"),
        ] {
            assert_eq!(
                watch_targets(source, &format!("profile backend 1\namends \"{module}\"\n")),
                vec![source.to_path_buf(), PathBuf::from(expected)]
            );
        }
    }

    #[test]
    fn backend_watch_recovers_after_invalid_source_without_overwriting_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("axiom.acore");
        let output = directory.path().join("axiom.axiom");
        let valid = r#"
            project { id = "watch-regression" version = "1.0.0" }
            variants { ["mobile"] { include = Listing { "getProject" } } }
            endpoints { ["getProject"] = EndpointDef { name = "getProject" method = "GET" path = "/projects" } }
        "#;
        std::fs::write(&source, valid).unwrap();
        let mut state = State::new();
        refresh_backend(&mut state, &source, "mobile", true, &output);
        assert!(state.watch_error.is_none(), "{:?}", state.watch_error);
        assert_eq!(state.previous_ir.as_ref().unwrap().endpoints.len(), 1);
        let baseline_path = super::super::backend::baseline_path(&source, "mobile");
        let baseline = std::fs::read(&baseline_path).unwrap();

        std::fs::write(&source, "project { id =").unwrap();
        refresh_backend(&mut state, &source, "mobile", true, &output);
        assert!(state.watch_error.is_some());
        assert!(!state.is_rebuilding);
        assert_eq!(std::fs::read(&baseline_path).unwrap(), baseline);
        assert_eq!(state.previous_ir.as_ref().unwrap().endpoints.len(), 1);

        std::fs::write(&source, valid.replace("/projects", "/v2/projects")).unwrap();
        refresh_backend(&mut state, &source, "mobile", true, &output);
        assert!(state.watch_error.is_none(), "{:?}", state.watch_error);
        assert_eq!(
            state.previous_ir.as_ref().unwrap().endpoints["getProject"].path,
            "/v2/projects"
        );
    }
}
