use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::Path;
use std::time::SystemTime;

// These structs must match the ones in `axiom-runtime/src/cache/sled_store.rs`
#[derive(Serialize, Deserialize, Debug)]
struct CacheEntry {
    payload: Vec<u8>,
    headers: Vec<(String, String)>,
    created_at: SystemTime,
    ttl_secs: u64,
}

fn open_db(db_path: &Path) -> Result<Db> {
    sled::open(db_path)
        .with_context(|| format!("Failed to open sled database at '{}'", db_path.display()))
}

pub async fn handle_ls(db_path: &Path) -> Result<()> {
    let db = open_db(db_path)?;
    println!("Inspecting cache at: {}\n", db_path.display());

    if db.is_empty() {
        println!("Cache is empty.");
        return Ok(());
    }

    println!("{:<64} {:<10} {:<25}", "Key", "Status", "Expires At");
    println!("{:-<105}", ""); // Divider line

    for item in db.iter() {
        if let Ok((key_bytes, value_bytes)) = item {
            let key = String::from_utf8_lossy(&key_bytes);

            // Deserialize to get metadata
            if let Ok(entry) = bincode::deserialize::<CacheEntry>(&value_bytes) {
                let now = SystemTime::now();
                let created_dt: DateTime<Utc> = entry.created_at.into();
                let expires_at = created_dt + std::time::Duration::from_secs(entry.ttl_secs);

                let is_stale =
                    now > entry.created_at + std::time::Duration::from_secs(entry.ttl_secs);
                let status = if is_stale { "STALE" } else { "FRESH" };

                // Attempt to show a snippet of the payload if it's JSON
                let payload_summary = if let Ok(json_val) =
                    serde_json::from_slice::<serde_json::Value>(&entry.payload)
                {
                    serde_json::to_string(&json_val)?
                        .chars()
                        .take(80)
                        .collect::<String>()
                } else {
                    format!("[{} bytes of binary data]", entry.payload.len())
                };

                println!(
                    "{:<64} {:<10} {:<25}",
                    &key[..12],
                    status,
                    expires_at.to_rfc2822()
                );
                println!("  └─ Payload: {}...", payload_summary);
            } else {
                println!("{:<64} (corrupted entry)", key);
            }
        }
    }
    Ok(())
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
