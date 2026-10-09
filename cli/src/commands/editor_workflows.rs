//! Explicit IDE workflows are introspected from the real Clap command tree.
//! This validator parses argv but never dispatches a command or opens a provider.
use anyhow::{ensure, Context, Result};
use clap::{ArgAction, Command, CommandFactory, Parser};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    id: String,
    flag: Option<String>,
    position: Option<usize>,
    required: bool,
    multiple: bool,
    repeat_flag: bool,
    min_values: usize,
    max_values: usize,
    value_type: String,
    choices: Vec<String>,
    defaults: Vec<String>,
    description: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    id: String,
    command: Vec<String>,
    description: String,
    inputs: Vec<Input>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub workflow: String,
    pub inputs: BTreeMap<String, Value>,
}

fn walk(command: &Command, path: Vec<String>, out: &mut Vec<Workflow>) {
    if command.get_subcommands().next().is_some() {
        for child in command.get_subcommands().filter(|c| c.get_name() != "help") {
            let mut p = path.clone();
            p.push(child.get_name().into());
            walk(child, p, out);
        }
        return;
    }
    let mut inputs = vec![];
    for a in command
        .get_arguments()
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
    {
        let values = a.get_num_args().unwrap_or_default();
        let tid = a.get_value_parser().type_id();
        let value_type = if matches!(a.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
            "boolean"
        } else if tid == std::any::TypeId::of::<PathBuf>()
            || (path == ["build"] && a.get_id() == "file")
            || (path == ["inspect", "source"] && a.get_id() == "entry")
            || (path.first().is_some_and(|p| p == "dependencies") && a.get_id() == "path")
        {
            "path"
        } else if [
            std::any::TypeId::of::<u8>(),
            std::any::TypeId::of::<u16>(),
            std::any::TypeId::of::<u32>(),
            std::any::TypeId::of::<u64>(),
            std::any::TypeId::of::<usize>(),
            std::any::TypeId::of::<i32>(),
        ]
        .iter()
        .any(|id| tid == *id)
        {
            "integer"
        } else {
            "string"
        };
        inputs.push(Input {
            id: a.get_id().as_str().into(),
            flag: a
                .get_long()
                .map(|s| format!("--{s}"))
                .or_else(|| a.get_short().map(|c| format!("-{c}"))),
            position: a.get_index(),
            required: a.is_required_set(),
            multiple: matches!(a.get_action(), ArgAction::Append) || values.max_values() > 1,
            repeat_flag: matches!(a.get_action(), ArgAction::Append) && values.max_values() == 1,
            min_values: values.min_values(),
            max_values: if matches!(a.get_action(), ArgAction::Append) {
                65536
            } else {
                values.max_values().min(65536)
            },
            value_type: value_type.into(),
            choices: a
                .get_value_parser()
                .possible_values()
                .map(|v| {
                    v.filter(|v| !v.is_hide_set())
                        .map(|v| v.get_name().into())
                        .collect()
                })
                .unwrap_or_default(),
            defaults: a
                .get_default_values()
                .iter()
                .map(|v| v.to_string_lossy().into())
                .collect(),
            description: a.get_help().map(ToString::to_string).unwrap_or_default(),
        });
    }
    out.push(Workflow {
        id: path.join("/"),
        command: path,
        description: command
            .get_about()
            .map(ToString::to_string)
            .unwrap_or_default(),
        inputs,
    });
}
pub fn workflows() -> Vec<Workflow> {
    let mut root = crate::Cli::command();
    root.build();
    let mut result = vec![];
    if let Some(editor) = root.get_subcommands().find(|c| c.get_name() == "editor") {
        if let Some(report) = editor.get_subcommands().find(|c| c.get_name() == "report") {
            walk(report, vec!["editor".into(), "report".into()], &mut result);
        }
    }
    for c in root.get_subcommands().filter(|c| {
        matches!(
            c.get_name(),
            "check"
                | "explain"
                | "server"
                | "test"
                | "scenario"
                | "serve"
                | "mock"
                | "build"
                | "inspect"
                | "release"
                | "package"
                | "contract"
                | "packages"
                | "database"
                | "extensions"
                | "dependencies"
                | "ui"
                | "run"
                | "doctor"
                | "diff"
        )
    }) {
        walk(c, vec![c.get_name().into()], &mut result);
    }
    result
}
pub fn catalog() -> Value {
    json!({"format":"axiom-editor-workflows/v1","cliVersion":env!("CARGO_PKG_VERSION"),"compilerVersion":acore::editor::project::compiler_identity(),"workflows":workflows()})
}
fn value(input: &Input, v: &Value, root: &Path) -> Result<String> {
    let mut s = match input.value_type.as_str() {
        "integer" => {
            ensure!(
                v.as_i64().is_some() || v.as_u64().is_some(),
                "{} requires an integer",
                input.id
            );
            v.to_string()
        }
        _ => v
            .as_str()
            .context("workflow input requires a string")?
            .into(),
    };
    ensure!(
        s.len() <= 8192 && !s.chars().any(char::is_control),
        "invalid workflow input text"
    );
    ensure!(
        input.choices.is_empty() || input.choices.contains(&s),
        "{} is not one of the supported values",
        input.id
    );
    if input.value_type == "path" {
        let p = Path::new(&s);
        let p = if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        };
        let mut suffix = vec![];
        let mut parent = p.as_path();
        while !parent.exists() {
            suffix.push(
                parent
                    .file_name()
                    .context("invalid workflow path")?
                    .to_owned(),
            );
            parent = parent.parent().context("invalid workflow path")?;
        }
        let mut clean = parent.canonicalize()?;
        for part in suffix.into_iter().rev() {
            clean.push(part)
        }
        ensure!(
            clean.starts_with(root)
                && !clean
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "workflow path must stay in the owning project"
        );
        for parent in clean
            .ancestors()
            .skip(usize::from(!clean.is_dir()))
            .take_while(|p| *p != root)
        {
            ensure!(
                !parent.join("AxiomDeps.toml").is_file(),
                "workflow input belongs to a different nested Axiom project"
            );
        }
        s = clean.to_string_lossy().into();
    }
    Ok(s)
}
pub fn validate(request: &str, root: &Path) -> Result<Value> {
    ensure!(request.len() <= 65536, "workflow request exceeds 64 KiB");
    let request: Request = serde_json::from_str(request)?;
    let root = root.canonicalize()?;
    ensure!(root.is_dir(), "owning project root must be a directory");
    let w = workflows()
        .into_iter()
        .find(|w| w.id == request.workflow)
        .context("unsupported workflow")?;
    for key in request.inputs.keys() {
        ensure!(
            w.inputs.iter().any(|i| &i.id == key),
            "unsupported workflow option: {key}"
        );
    }
    let mut argv = w.command.clone();
    let mut positional = BTreeMap::new();
    for i in &w.inputs {
        let Some(v) = request.inputs.get(&i.id) else {
            continue;
        };
        if i.value_type == "boolean" {
            let b = v.as_bool().context("flag requires true or false")?;
            if b {
                argv.push(
                    i.flag
                        .clone()
                        .context("boolean positional is unsupported")?,
                )
            }
            continue;
        }
        let values = if i.multiple {
            v.as_array().cloned().unwrap_or_else(|| vec![v.clone()])
        } else {
            ensure!(!v.is_array(), "{} takes one value", i.id);
            vec![v.clone()]
        };
        ensure!(
            values.len() >= i.min_values && values.len() <= i.max_values.max(1),
            "invalid value count for {}",
            i.id
        );
        let mut values = values
            .iter()
            .map(|v| value(i, v, &root))
            .collect::<Result<Vec<_>>>()?;
        if let Some(flag) = &i.flag {
            if i.max_values <= 1 || i.repeat_flag {
                for v in values {
                    argv.push(format!("{flag}={v}"));
                }
            } else {
                argv.push(flag.clone());
                argv.append(&mut values);
            }
        } else {
            positional.insert(i.position.context("missing positional index")?, values);
        }
    }
    if !positional.is_empty() {
        argv.push("--".into());
        for values in positional.into_values() {
            argv.extend(values)
        }
    }
    crate::Cli::try_parse_from(std::iter::once("axiom".to_string()).chain(argv.clone()))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let application_suite = w.id == "ui/test" && request.inputs.contains_key("suite");
    let label = if w.id == "scenario/run" {
        "Development scenario assertions (compiler-host; synthetic read providers)"
    } else if w.id == "ui/test" {
        if application_suite {
            "Application suite assertions"
        } else {
            "Compiler smoke (zero application assertions)"
        }
    } else {
        w.description.as_str()
    };
    Ok(
        json!({"format":"axiom-editor-workflow-validation/v1","workflow":w.id,"root":root,"argv":argv,"label":label,"reportKind":if w.id=="scenario/run"{"development-scenario"}else if application_suite{"application-assertions"}else if w.id=="ui/test"{"compiler-smoke"}else{"cli-workflow"},"compilerVersion":acore::editor::project::compiler_identity(),"execution":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_covers_current_database_server_ui_commands() {
        let c = workflows();
        for id in [
            "database/runtime/prepare",
            "database/fleet/prepare",
            "ui/test",
            "serve",
            "mock",
            "scenario/catalog",
            "scenario/check",
            "scenario/run",
            "doctor",
        ] {
            assert!(c.iter().any(|w| w.id == id), "{id}")
        }
    }
    #[test]
    fn rejects_unknown_options_before_dispatch() {
        assert!(validate(
            r#"{"workflow":"doctor","inputs":{"shell":"rm"}}"#,
            Path::new(".")
        )
        .is_err());
    }
    #[test]
    fn actual_clap_parser_rejects_bad_choices() {
        assert!(validate(
            r#"{"workflow":"mock","inputs":{"profile":"invented"}}"#,
            Path::new(".")
        )
        .is_err());
    }
}
