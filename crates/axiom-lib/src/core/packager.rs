use anyhow::{Context, Result};
use axiom_extractor::IR;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

const MAGIC_BYTES: &[u8; 4] = b"AXOM";
const FORMAT_VERSION: u32 = 1;
const OBFUSCATION_KEY: &[u8] = b"AxiomCoreSecretKey2025!";

#[derive(Serialize, Deserialize, Debug)]
struct TocEntry {
    name: String,
    offset: u64,
    size: u64,
}

/// Helper function to apply a simple XOR cipher to a byte slice.
fn xor_cipher(data: &mut [u8]) {
    for (i, byte) in data.iter_mut().enumerate() {
        *byte ^= OBFUSCATION_KEY[i % OBFUSCATION_KEY.len()];
    }
}

/// Packages the IR and binary into a single, obfuscated .axiom binary blob.
pub async fn package_axiom_file(
    ir: &IR,
    binary_bytes: &[u8],
    fbs_schema_bytes: &[u8],
    output_path_str: &str,
) -> Result<()> {
    let mut file = File::create(output_path_str).context(format!(
        "Failed to create .axiom file at '{}'",
        output_path_str
    ))?;

    // 1. Serialize & obfuscate blobs
    let mut ir_bytes = serde_json::to_vec(ir).context("Failed to serialize IR to JSON bytes")?;
    xor_cipher(&mut ir_bytes);

    let mut runtime_bytes = binary_bytes.to_vec();
    xor_cipher(&mut runtime_bytes);

    let mut fbs_bytes = fbs_schema_bytes.to_vec();
    xor_cipher(&mut fbs_bytes);

    // 2. Reserve header
    let header_size: u64 = (MAGIC_BYTES.len() + 4 + 8) as u64;
    file.seek(SeekFrom::Start(header_size))
        .context("Failed to seek past header")?;

    // 3. Placeholder TOC to get size (3 entries now)
    let placeholder_toc = vec![
        TocEntry {
            name: "ir.json".to_string(),
            offset: 0,
            size: 0,
        },
        TocEntry {
            name: "runtime.bin".to_string(),
            offset: 0,
            size: 0,
        },
        TocEntry {
            name: "schema.fbs".to_string(),
            offset: 0,
            size: 0,
        },
    ];
    let toc_bytes_placeholder = bincode::serialize(&placeholder_toc)?;
    file.write_all(&vec![0; toc_bytes_placeholder.len()])?;

    // 4. Write data blobs & record positions
    let ir_offset = file.stream_position()?;
    file.write_all(&ir_bytes)?;
    let ir_size = ir_bytes.len() as u64;

    let runtime_offset = file.stream_position()?;
    file.write_all(&runtime_bytes)?;
    let runtime_size = runtime_bytes.len() as u64;

    let fbs_offset = file.stream_position()?;
    file.write_all(&fbs_bytes)?;
    let fbs_size = fbs_bytes.len() as u64;

    // 5. Final TOC
    let final_toc = vec![
        TocEntry {
            name: "ir.json".to_string(),
            offset: ir_offset,
            size: ir_size,
        },
        TocEntry {
            name: "runtime.bin".to_string(),
            offset: runtime_offset,
            size: runtime_size,
        },
        TocEntry {
            name: "schema.fbs".to_string(),
            offset: fbs_offset,
            size: fbs_size,
        },
    ];
    let final_toc_bytes = bincode::serialize(&final_toc)?;

    if final_toc_bytes.len() != toc_bytes_placeholder.len() {
        anyhow::bail!("Internal error: Final TOC size does not match placeholder size.");
    }

    // 6. Header + TOC
    file.seek(SeekFrom::Start(0))?;
    file.write_all(MAGIC_BYTES)?;
    file.write_all(&FORMAT_VERSION.to_le_bytes())?;
    file.write_all(&(final_toc_bytes.len() as u64).to_le_bytes())?;

    file.seek(SeekFrom::Start(header_size))?;
    file.write_all(&final_toc_bytes)?;
    file.flush()?;

    Ok(())
}
