// FILE: axiom-cli/src/commands/init.rs

use crate::components::init_wizard::render_init_wizard;
use crate::state::{InitStep, State};
use anyhow::Result;
use crossterm::event::KeyCode;
use std::fs;
use std::path::Path;

pub async fn handle_init(entrypoint_arg: Option<String>, module_arg: Option<String>) -> Result<()> {
    // If arguments are explicitly passed, skip TUI
    if let (Some(ep), Some(modu)) = (entrypoint_arg, module_arg) {
        let project_name = get_default_project_name();
        write_acore_file(&modu, &ep, &project_name)?;
        println!("✅ Created axiom.acore successfully!");
        return Ok(());
    }

    let mut state = State::new();
    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    // ✨ AUTO-DETECT ENVIRONMENT
    let is_go = Path::new("go.mod").exists() || Path::new("main.go").exists();
    let is_py = Path::new("requirements.txt").exists()
        || Path::new("pyproject.toml").exists()
        || Path::new("main.py").exists();

    // ✨ FIX: Safely check if the language exists before manipulating the array
    if is_go {
        if let Some(idx) = state.init_context.languages.iter().position(|l| l == "Go") {
            let item = state.init_context.languages.remove(idx);
            state.init_context.languages.insert(0, item);
        }
    } else if is_py {
        if let Some(idx) = state
            .init_context
            .languages
            .iter()
            .position(|l| l == "Python")
        {
            let item = state.init_context.languages.remove(idx);
            state.init_context.languages.insert(0, item);
        }
    }

    // Default project name
    state.init_context.project_name = get_default_project_name();
    state.init_context.cursor_position = state.init_context.project_name.len();

    loop {
        tui.draw(|f| render_init_wizard(f, f.size(), &state))?;

        if let Some(crate::tui::Event::Key(key)) = tui.event_rx.recv().await {
            match key.code {
                KeyCode::Esc => break,
                KeyCode::Char('q') if state.init_context.step == InitStep::Success => break,
                KeyCode::Char('c')
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    break
                }
                _ => {}
            }

            match state.init_context.step {
                InitStep::Language => match key.code {
                    KeyCode::Up => {
                        let i = state.init_context.selected_language;
                        state.init_context.selected_language = if i == 0 {
                            state.init_context.languages.len() - 1
                        } else {
                            i - 1
                        };
                    }
                    KeyCode::Down => {
                        let i = state.init_context.selected_language;
                        state.init_context.selected_language =
                            (i + 1) % state.init_context.languages.len();
                    }
                    KeyCode::Enter => {
                        let selected =
                            &state.init_context.languages[state.init_context.selected_language];
                        if selected == "Python" {
                            state.init_context.step = InitStep::Framework;
                            state.init_context.current_step = 2;
                            state.init_context.total_steps = 4;
                        } else if selected == "Go" {
                            // ✨ SKIP FRAMEWORK FOR GO!
                            state.init_context.step = InitStep::ProjectName;
                            state.init_context.current_step = 2;
                            state.init_context.total_steps = 3;
                        }
                    }
                    _ => {}
                },
                InitStep::Framework => match key.code {
                    KeyCode::Up => {
                        let i = state.init_context.selected_framework;
                        state.init_context.selected_framework = if i == 0 { 2 } else { i - 1 };
                        // Assuming 3 frameworks
                    }
                    KeyCode::Down => {
                        let i = state.init_context.selected_framework;
                        state.init_context.selected_framework = (i + 1) % 3;
                    }
                    KeyCode::Enter => {
                        if state.init_context.selected_framework == 0 {
                            // FastAPI
                            state.init_context.step = InitStep::ProjectName;
                            state.init_context.current_step += 1;
                        }
                    }
                    _ => {}
                },
                InitStep::ProjectName => match key.code {
                    KeyCode::Char(' ') => {} // Block spaces
                    KeyCode::Char(c) => {
                        let pos = state.init_context.cursor_position;
                        state.init_context.project_name.insert(pos, c);
                        state.init_context.cursor_position += 1;
                    }
                    KeyCode::Backspace => {
                        let pos = state.init_context.cursor_position;
                        if pos > 0 {
                            state.init_context.project_name.remove(pos - 1);
                            state.init_context.cursor_position -= 1;
                        }
                    }
                    KeyCode::Left => {
                        if state.init_context.cursor_position > 0 {
                            state.init_context.cursor_position -= 1;
                        }
                    }
                    KeyCode::Right => {
                        if state.init_context.cursor_position
                            < state.init_context.project_name.len()
                        {
                            state.init_context.cursor_position += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if !state.init_context.project_name.is_empty() {
                            state.init_context.step = InitStep::Entrypoint;
                            state.init_context.current_step += 1;

                            // Dynamic entrypoint suggestion
                            let is_go = state.init_context.languages
                                [state.init_context.selected_language]
                                == "Go";
                            state.init_context.entrypoint = if is_go {
                                "./main.go".into()
                            } else {
                                "./main.py:app".into()
                            };
                            state.init_context.cursor_position =
                                state.init_context.entrypoint.len();
                        }
                    }
                    _ => {}
                },
                InitStep::Entrypoint => match key.code {
                    KeyCode::Char(' ') => {} // Block spaces
                    KeyCode::Char(c) => {
                        let pos = state.init_context.cursor_position;
                        state.init_context.entrypoint.insert(pos, c);
                        state.init_context.cursor_position += 1;
                    }
                    KeyCode::Backspace => {
                        let pos = state.init_context.cursor_position;
                        if pos > 0 {
                            state.init_context.entrypoint.remove(pos - 1);
                            state.init_context.cursor_position -= 1;
                        }
                    }
                    KeyCode::Left => {
                        if state.init_context.cursor_position > 0 {
                            state.init_context.cursor_position -= 1;
                        }
                    }
                    KeyCode::Right => {
                        if state.init_context.cursor_position < state.init_context.entrypoint.len()
                        {
                            state.init_context.cursor_position += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if !state.init_context.entrypoint.is_empty() {
                            // WRITE FILE AND GO TO SUCCESS
                            let is_go = state.init_context.languages
                                [state.init_context.selected_language]
                                == "Go";
                            let module = if is_go { "axiom-go" } else { "axiom-fastapi" };

                            if write_acore_file(
                                module,
                                &state.init_context.entrypoint,
                                &state.init_context.project_name,
                            )
                            .is_ok()
                            {
                                state.init_context.step = InitStep::Success;
                            }
                        }
                    }
                    _ => {}
                },
                InitStep::Success => {} // Handled by 'q' logic at top
            }
        }
    }

    tui.exit().map_err(|e| anyhow::anyhow!(e))
}

fn get_default_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "my-axiom-project".to_string())
        .replace(" ", "-")
}

fn write_acore_file(module: &str, entrypoint: &str, project_name: &str) -> Result<()> {
    let content = format!(
        r#"amends "{}:{}"

project {{
  id = "{}"
  version = "v0.0.1"
}}
"#,
        module, entrypoint, project_name
    );
    fs::write("axiom.acore", content)?;
    Ok(())
}
