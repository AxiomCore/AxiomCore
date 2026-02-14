use anyhow::Ok;
use crossterm::event::KeyCode;

use crate::state::InitStep;
use std::fs;

pub async fn handle_init() -> anyhow::Result<()> {
    let mut state = crate::state::State::new();
    state.init_context.version = "v0.0.1".to_string(); // Default

    let mut tui = crate::tui::Tui::new().map_err(|e| anyhow::anyhow!(e))?;
    tui.enter().map_err(|e| anyhow::anyhow!(e))?;

    loop {
        tui.draw(|f| crate::components::init_wizard::render_init_wizard(f, f.size(), &state))?;

        if let Some(crate::tui::Event::Key(key)) = tui.event_rx.recv().await {
            match key.code {
                KeyCode::Esc => break,
                KeyCode::Up => match state.init_context.step {
                    InitStep::Language => {
                        state.init_context.selected_language =
                            state.init_context.selected_language.saturating_sub(1)
                    }
                    InitStep::Framework => {
                        state.init_context.selected_framework =
                            state.init_context.selected_framework.saturating_sub(1)
                    }
                    _ => {}
                },
                KeyCode::Down => {
                    match state.init_context.step {
                        InitStep::Language => {
                            state.init_context.selected_language =
                                (state.init_context.selected_language + 1).min(0)
                        } // Only python active
                        InitStep::Framework => {
                            state.init_context.selected_framework =
                                (state.init_context.selected_framework + 1).min(0)
                        } // Only FastAPI active
                        _ => {}
                    }
                }
                KeyCode::Enter => match state.init_context.step {
                    InitStep::Language => state.init_context.step = InitStep::Framework,
                    InitStep::Framework => state.init_context.step = InitStep::ProjectName,
                    InitStep::ProjectName => {
                        if !state.init_context.project_name.is_empty() {
                            state.init_context.step = InitStep::Version
                        }
                    }
                    InitStep::Version => state.init_context.step = InitStep::Entrypoint,
                    InitStep::Entrypoint => {
                        generate_acore_file(&state)?;
                        state.init_context.step = InitStep::Success;
                    }
                    InitStep::Success => break,
                },
                KeyCode::Char(c) => match state.init_context.step {
                    InitStep::ProjectName => state.init_context.project_name.push(c),
                    InitStep::Version => state.init_context.version.push(c),
                    InitStep::Entrypoint => state.init_context.entrypoint.push(c),
                    InitStep::Success if c == 'q' => break,
                    _ => {}
                },
                KeyCode::Backspace => match state.init_context.step {
                    InitStep::ProjectName => {
                        state.init_context.project_name.pop();
                    }
                    InitStep::Version => {
                        state.init_context.version.pop();
                    }
                    InitStep::Entrypoint => {
                        state.init_context.entrypoint.pop();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    tui.exit().map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

fn generate_acore_file(state: &crate::state::State) -> anyhow::Result<()> {
    let content = format!(
        r#"amends "axiom-python:{entrypoint}"

project {{
  id = "{name}"
  version = "{version}"
}}

variants {{
  ["default"] {{
    include = new {{ "*" }}
  }}
}}

"#,
        entrypoint = state.init_context.entrypoint,
        name = state.init_context.project_name,
        version = state.init_context.version
    );

    fs::write("axiom.acore", content)?;
    Ok(())
}
