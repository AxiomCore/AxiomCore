use std::process::{Command, Stdio};

use axiom_extractor::IR;

pub async fn run_extractor(entrypoint: &str) -> anyhow::Result<IR> {
    let framework = "fastapi";

    // 2. Check if plugin exists
    let home = dirs::home_dir().unwrap();
    let plugin_path = home.join(".axiom/plugins").join(framework);

    if !plugin_path.exists() {
        anyhow::bail!(
            "❌ Extractor for '{}' not installed.\nRun `axiom install --extractor {}`",
            framework,
            framework
        );
    }

    // 3. Run the Extractor Binary
    let output = Command::new(&plugin_path)
        .arg("--app")
        .arg(entrypoint) // Passed from `axiom build --entrypoint`
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()) // Stream logs to user
        .output()?;

    if !output.status.success() {
        anyhow::bail!("Extractor failed.");
    }

    // 4. Parse JSON from STDOUT directly into memory
    let ir_json = String::from_utf8(output.stdout)?;
    let ir: IR = serde_json::from_str(&ir_json)?;

    Ok(ir)
}
