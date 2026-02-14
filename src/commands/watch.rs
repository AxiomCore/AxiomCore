use crate::commands::pull::handle_pull_auto;
use crate::components::watch_hud::render_watch_hud;
use crate::state::{IRDiff, State};
use axiom_extractor::evaluate_acore_config;
use crossterm::event::KeyCode;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;

pub fn get_watch_context() -> Option<(PathBuf, Vec<PathBuf>)> {
    let acore_path = PathBuf::from("axiom.acore");
    if !acore_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&acore_path).ok()?;
    // Simple regex/parsing to find amends "axiom-python:./main.py:app"
    let re = regex::Regex::new(r#"amends\s+"[^:]+:([^:]+):[^"]+""#).unwrap();

    let mut targets = vec![acore_path.clone()];
    if let Some(caps) = re.captures(&content) {
        targets.push(PathBuf::from(&caps[1]));
    }

    Some((acore_path, targets))
}

pub async fn handle_watch_dynamic(build_flag: bool) -> anyhow::Result<()> {
    let context = get_watch_context();

    if context.is_none() {
        println!("🚀 Frontend Watch Mode: Coming Soon to Production!");
        return Ok(());
    }

    let (acore_path, targets) = context.unwrap();
    let mut state = crate::state::State::new();
    state.watch_build_enabled = build_flag;

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

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

    loop {
        tui.draw(|f| render_watch_hud(f, f.size(), &state))?;

        tokio::select! {
            // Handle Keyboard
            Some(event) = tui.event_rx.recv() => {
                if let crate::tui::Event::Key(key) = event {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('r') => { /* Trigger manual build */ },
                        _ => {}
                    }
                }
            }
            // Handle File Changes
            Some(res) = rx.recv() => {
                if let Ok(_) = res {
                    state.is_rebuilding = true;
                    // 1. Evaluate config via Pkl
                    let new_config = evaluate_acore_config(acore_path.to_str().unwrap())?;
                    let new_ir = new_config.ir.clone().unwrap();

                    // 2. Calculate Diff
                    state.watch_diff = IRDiff::from_irs(&state.previous_ir, &new_ir);
                    state.previous_ir = Some(new_ir);

                    // 3. Build local file if flag is set
                    if build_flag {
                         axiom_build::core::build::handle_build("default", "", "", None).await?;
                    }

                    state.is_rebuilding = false;
                    state.last_sync_time = chrono::Local::now().format("%H:%M:%S").to_string();
                }
            }
        }
    }

    tui.exit().map_err(|e| anyhow::anyhow!(e))
}

pub async fn handle_watch_consumer() -> anyhow::Result<()> {
    // 1. Ensure config exists (Run pull once)
    handle_pull_auto(None).await?;

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    let mut state = State::new();

    // Load initial state for diffing
    if let Ok(bytes) = std::fs::read("project.axiom") {
        if let Ok(file) = axiom_build::core::unpackager::unpack_axiom_bytes(&bytes) {
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
                        if let Ok(file) = axiom_build::core::unpackager::unpack_axiom_bytes(&bytes) {
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
