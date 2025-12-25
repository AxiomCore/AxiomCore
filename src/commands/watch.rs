use crate::commands::pull::handle_pull_path;
use anyhow::{Context, Result};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;
use tokio::time::sleep;

/// Handles the `axiom watch` command.
pub async fn handle_watch(axiom_path: &Path, runtime_source: Option<&str>) -> Result<()> {
    // Convert path to an absolute path to avoid ambiguity
    let absolute_axiom_path = axiom_path
        .canonicalize()
        .with_context(|| format!("Failed to find file at '{}'", axiom_path.display()))?;

    println!(
        "👁️  Watching for changes in '{}'...",
        absolute_axiom_path.display()
    );

    // --- Perform an initial pull ---
    println!("\nPerforming initial pull...");
    if let Err(e) = handle_pull_path(absolute_axiom_path.to_str().unwrap(), runtime_source).await {
        eprintln!("Initial pull failed: {:?}", e);
    } else {
        println!("✅ Initial pull successful.");
    }
    println!("\n----------------------------------------");

    // --- Set up the file watcher ---
    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher =
        Watcher::new(tx, notify::Config::default()).context("Failed to create file watcher")?;

    // Watch the specific file. We need to watch its parent directory to be robust.
    if let Some(parent) = absolute_axiom_path.parent() {
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .with_context(|| format!("Failed to watch directory '{}'", parent.display()))?;
    } else {
        return Err(anyhow::anyhow!("Cannot watch root directory"));
    }

    // --- Main Watch Loop ---
    // This loop blocks, which is what we want for a `watch` command.
    loop {
        match rx.recv() {
            Ok(Ok(Event { kind, paths, .. })) => {
                // Check if the event is for our specific file
                if paths.iter().any(|p| p == &absolute_axiom_path) {
                    if kind.is_modify() || kind.is_create() {
                        println!(
                            "\n🔄 Change detected in '{}'. Re-running pull...",
                            absolute_axiom_path.display()
                        );

                        // Debounce: Wait a moment to ensure file writing is complete
                        sleep(Duration::from_millis(500)).await;

                        if let Err(e) =
                            handle_pull_path(absolute_axiom_path.to_str().unwrap(), runtime_source)
                                .await
                        {
                            eprintln!("Pull failed after change: {:?}", e);
                        } else {
                            println!("✅ Pull successful.");
                        }
                        println!("\n----------------------------------------");
                        println!(
                            "👁️  Watching for changes in '{}'...",
                            absolute_axiom_path.display()
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("File watch error: {:?}", e);
            }
            Err(e) => {
                eprintln!("File watch channel disconnected: {:?}", e);
                break; // Exit the loop if the channel is broken
            }
        }
    }

    Ok(())
}
