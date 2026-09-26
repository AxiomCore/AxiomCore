use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    schema_version: u8,
    cli_version: &'static str,
    contract: String,
    variant: String,
    iterations: usize,
    warmup_iterations: usize,
    build_ms: Summary,
    artifact_decode_ms: Summary,
    artifact_bytes: u64,
    endpoints: usize,
}

#[derive(Debug, Serialize)]
struct Summary {
    min: f64,
    median: f64,
    p95: f64,
    max: f64,
}

/// Benchmark the local build and artifact-load boundaries that AxiomCore owns.
/// The compiled artifact is restored on every exit path, so profiling does not
/// dirty a developer's release candidate.
pub async fn handle_benchmark(
    file: String,
    variant: Option<String>,
    iterations: usize,
    warmup: usize,
    output: PathBuf,
    json: bool,
) -> Result<()> {
    if iterations == 0 {
        anyhow::bail!("--iterations must be at least 1");
    }
    if iterations > 1000 || warmup > 1000 {
        anyhow::bail!(
            "--iterations and --warmup are capped at 1000 to prevent accidental long runs"
        );
    }
    let variant = variant.unwrap_or_else(|| "default".to_string());
    let config_path = PathBuf::from(&file);
    if !config_path.is_file() {
        anyhow::bail!("Contract source not found: {}", config_path.display());
    }

    let artifact_path = std::env::current_dir()?.join("axiom.axiom");
    let _artifact_guard = ArtifactGuard::preserve(&artifact_path)?;

    for _ in 0..warmup {
        build_once(&file, &variant).await?;
    }

    let mut build_samples = Vec::with_capacity(iterations);
    let mut decode_samples = Vec::with_capacity(iterations);
    let mut artifact_bytes = 0;
    let mut endpoints = 0;
    for _ in 0..iterations {
        let build_start = Instant::now();
        let artifact = build_once(&file, &variant).await?;
        build_samples.push(duration_ms(build_start.elapsed()));

        artifact_bytes = fs::metadata(&artifact)?.len();
        let decode_start = Instant::now();
        let decoded = axiom_lib::unpackager::unpack_axiom_file(&artifact)?;
        decode_samples.push(duration_ms(decode_start.elapsed()));
        endpoints = decoded.endpoints.len();
    }

    let report = BenchmarkReport {
        schema_version: 1,
        cli_version: env!("CARGO_PKG_VERSION"),
        contract: config_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("axiom.acore")
            .to_string(),
        variant,
        iterations,
        warmup_iterations: warmup,
        build_ms: summarize(&build_samples),
        artifact_decode_ms: summarize(&decode_samples),
        artifact_bytes,
        endpoints,
    };

    let serialized = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, &serialized)
        .with_context(|| format!("Could not write benchmark report to {}", output.display()))?;

    if json {
        println!("{serialized}");
    } else {
        println!("AxiomCore benchmark");
        println!(
            "  Build: median {:.2} ms, p95 {:.2} ms",
            report.build_ms.median, report.build_ms.p95
        );
        println!(
            "  Artifact decode: median {:.2} ms, p95 {:.2} ms",
            report.artifact_decode_ms.median, report.artifact_decode_ms.p95
        );
        println!(
            "  Artifact: {} bytes, {} endpoint(s)",
            report.artifact_bytes, report.endpoints
        );
        println!("  Report: {}", output.display());
    }
    Ok(())
}

async fn build_once(file: &str, variant: &str) -> Result<PathBuf> {
    let output = axiom_build::core::build::handle_build(file, variant, "", "", None).await?;
    Ok(PathBuf::from(output))
}

fn duration_ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn summarize(samples: &[f64]) -> Summary {
    let mut values = samples.to_vec();
    values.sort_by(|a, b| a.total_cmp(b));
    let last = values.len().saturating_sub(1);
    let percentile = |fraction: f64| values[((last as f64 * fraction).ceil() as usize).min(last)];
    Summary {
        min: values[0],
        median: percentile(0.50),
        p95: percentile(0.95),
        max: values[last],
    }
}

struct ArtifactGuard {
    path: PathBuf,
    original: Option<Vec<u8>>,
}

impl ArtifactGuard {
    fn preserve(path: &Path) -> Result<Self> {
        let original = if path.exists() {
            Some(fs::read(path).with_context(|| format!("Could not preserve {}", path.display()))?)
        } else {
            None
        };
        Ok(Self {
            path: path.to_path_buf(),
            original,
        })
    }
}

impl Drop for ArtifactGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(bytes) => {
                let _ = fs::write(&self.path, bytes);
            }
            None => {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_uses_a_conservative_nearest_rank_percentile() {
        let summary = summarize(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(summary.median, 3.0);
        assert_eq!(summary.p95, 5.0);
    }
}
