use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use crossterm::event::KeyCode;
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::Path;
use std::time::SystemTime;

use crate::components::cache_explorer::render_cache_explorer;

// These structs must match the ones in `axiom-runtime/src/cache/sled_store.rs`
#[derive(Serialize, Deserialize, Debug)]
pub struct CacheEntry {
    pub payload: Vec<u8>,                  // Add pub
    pub headers: Vec<(String, String)>,    // Add pub
    pub created_at: std::time::SystemTime, // Add pub
    pub ttl_secs: u64,                     // Add pub
}

fn open_db(db_path: &Path) -> Result<Db> {
    sled::open(db_path)
        .with_context(|| format!("Failed to open sled database at '{}'", db_path.display()))
}

pub async fn handle_ls(db_path: &Path) -> Result<()> {
    let db = open_db(db_path)?;
    let mut entries = Vec::new();

    for item in db.iter() {
        if let Ok((key_bytes, value_bytes)) = item {
            let key = String::from_utf8_lossy(&key_bytes).to_string();
            if let Ok(entry) = bincode::deserialize::<CacheEntry>(&value_bytes) {
                entries.push((key, entry));
            }
        }
    }

    if entries.is_empty() {
        println!("Cache is empty.");
        return Ok(());
    }

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;
    let mut selected = 0;

    loop {
        tui.draw(|f| render_cache_explorer(f, f.size(), &entries, selected))?;
        if let Some(event) = tui.event_rx.recv().await {
            match event {
                crate::tui::Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = selected.saturating_add(1).min(entries.len() - 1)
                    }
                    KeyCode::Char('x') => {
                        // Logic to delete the key from Sled
                        let (key_to_del, _) = &entries[selected];
                        db.remove(key_to_del)?;
                        entries.remove(selected);
                        if selected >= entries.len() && !entries.is_empty() {
                            selected = entries.len() - 1;
                        }
                        if entries.is_empty() {
                            break;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    tui.exit().map_err(|e| anyhow::anyhow!(e))
}

pub async fn handle_get(db_path: &Path, key: &str) -> Result<()> {
    let db = open_db(db_path)?;
    println!(
        "Inspecting key '{}' in cache at: {}\n",
        key,
        db_path.display()
    );

    match db.get(key)? {
        Some(ivec) => {
            let entry: CacheEntry = bincode::deserialize(&ivec)
                .context("Failed to deserialize cache entry. The data may be corrupt.")?;

            let created_dt: DateTime<Utc> = entry.created_at.into();
            let expires_at = created_dt + std::time::Duration::from_secs(entry.ttl_secs);
            let now = SystemTime::now();
            let is_stale = now > entry.created_at + std::time::Duration::from_secs(entry.ttl_secs);

            println!("  Status: {}", if is_stale { "STALE" } else { "FRESH" });
            println!("  Created At: {}", created_dt.to_rfc2822());
            println!("  Expires At: {}", expires_at.to_rfc2822());
            println!("  TTL: {} seconds", entry.ttl_secs);
            println!("  Headers: {:?}", entry.headers);
            println!("  Payload Size: {} bytes", entry.payload.len());

            // Attempt to pretty-print payload if it's JSON
            if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&entry.payload) {
                println!("  Payload (JSON):");
                println!("{}", serde_json::to_string_pretty(&json_val)?);
            } else {
                println!("  Payload (Bytes): {:?}", entry.payload);
            }
        }
        None => {
            return Err(anyhow!("Key '{}' not found in cache.", key));
        }
    }
    Ok(())
}

pub async fn handle_clear(db_path: &Path) -> Result<()> {
    let db = open_db(db_path)?;
    let count = db.len();
    db.clear()?;
    db.flush_async().await?;
    println!(
        "✅ Cleared {} entries from cache at '{}'.",
        count,
        db_path.display()
    );
    Ok(())
}
