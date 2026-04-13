// src/error_reporter.rs
use regex::Regex;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ErrorDiagnostic {
    pub title: String,
    pub message: String,
    pub file_path: Option<PathBuf>,
    pub line: Option<usize>,
    pub code_snippet: Option<Vec<String>>,
    pub direction: String,
}

impl ErrorDiagnostic {
    pub fn from_anyhow(err: &anyhow::Error) -> Self {
        let err_str = format!("{:?}", err);

        // Regex to find "path/to/file.ext:line" or "At path/to/file.ext:line"
        let re = Regex::new(r"([a-zA-Z0-9._/-]+\.[a-z]+):(\d+)").unwrap();

        let (file_path, line) = if let Some(caps) = re.captures(&err_str) {
            let path = PathBuf::from(&caps[1]);
            let line_num = caps[2].parse::<usize>().unwrap_or(0);
            (Some(path), Some(line_num))
        } else {
            (None, None)
        };

        let code_snippet = if let (Some(ref path), Some(line_num)) = (&file_path, line) {
            fs::read_to_string(path).ok().map(|content| {
                content
                    .lines()
                    .enumerate()
                    .filter(|(idx, _)| *idx >= line_num.saturating_sub(3) && *idx <= line_num + 1)
                    .map(|(idx, content)| format!("{:3} | {}", idx + 1, content))
                    .collect()
            })
        } else {
            None
        };

        let direction = if err_str.contains("pkl") || err_str.contains(".acore") {
            "Check your axiom.acore configuration and Pkl syntax.".to_string()
        } else if err_str.contains("cargo") || err_str.contains("rust") {
            "Native runtime compilation failed. Ensure your backend types are compatible."
                .to_string()
        } else {
            "Check the logs above for more details.".to_string()
        };

        Self {
            title: "Build Pipeline Error".to_string(),
            message: err.to_string(),
            file_path,
            line,
            code_snippet,
            direction,
        }
    }
}
