use crate::state::{PullStep, State};
use ratatui::{prelude::*, widgets::*};

pub fn render_pull_wizard(f: &mut Frame, area: Rect, state: &State) {
    let area = centered_rect(60, 50, area);
    let block = Block::default()
        .title(" Axiom Setup Wizard ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(2), // Header
        Constraint::Min(0),    // Content
        Constraint::Length(2), // Footer
    ])
    .margin(1)
    .split(inner);

    // 1. Header
    let header_text = match state.pull_context.step {
        PullStep::SourceSelection => "Where is your contract source?",
        PullStep::ProjectLink => "Select Remote Project",
        PullStep::LocalPathInput => "Enter path to .axiom file",
        PullStep::FrontendSelection => "Select Client SDK",
        PullStep::Processing => "Setting up workspace...",
        PullStep::Success => "Setup Complete",
    };
    f.render_widget(
        Paragraph::new(header_text)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::BOLD)),
        chunks[0],
    );

    // 2. Content
    match state.pull_context.step {
        PullStep::SourceSelection => {
            let options = vec!["☁️  Remote Project (Axiom Cloud)", "📂 Local File (.axiom)"];
            render_list(f, chunks[1], options, state.pull_context.source_mode);
        }
        PullStep::ProjectLink => {
            if state.pull_context.available_projects.is_empty() {
                f.render_widget(
                    Paragraph::new("Loading projects...").alignment(Alignment::Center),
                    chunks[1],
                );
            } else {
                let items: Vec<String> = state
                    .pull_context
                    .available_projects
                    .iter()
                    .map(|(id, name)| format!("{} ({})", name, id))
                    .collect();
                render_list(
                    f,
                    chunks[1],
                    items.iter().map(|s| s.as_str()).collect(),
                    state.pull_context.selected_project_idx,
                );
            }
        }
        PullStep::LocalPathInput => {
            let p = Paragraph::new(format!("> {}_", state.pull_context.local_file_path))
                .block(Block::default().borders(Borders::ALL).title(" File Path "));
            f.render_widget(p, centered_rect(90, 20, chunks[1]));
        }
        PullStep::FrontendSelection => {
            let frameworks = vec![
                "Flutter",
                "Swift (Coming Soon)",
                "Kotlin (Coming Soon)",
                "React (Coming Soon)",
            ];
            render_list(
                f,
                chunks[1],
                frameworks,
                state.pull_context.selected_framework,
            );
        }
        PullStep::Processing => {
            f.render_widget(
                Paragraph::new("⟳ Generating configuration...")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::Yellow)),
                chunks[1],
            );
        }
        PullStep::Success => {
            let msg = vec![
                Line::from("✅ axiom.yaml created."),
                Line::from("✅ Project linked."),
                Line::from(""),
                Line::from("Starting pull..."),
            ];
            f.render_widget(
                Paragraph::new(msg)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::Green)),
                chunks[1],
            );
        }
    }

    // 3. Footer
    f.render_widget(
        Paragraph::new("[Up/Down] Select  [Enter] Confirm  [Esc] Quit")
            .style(Style::default().dim())
            .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_list(f: &mut Frame, area: Rect, items: Vec<&str>, selected: usize) {
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().dim()
            };
            let prefix = if i == selected { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, s)).style(style)
        })
        .collect();

    f.render_widget(List::new(list_items).highlight_symbol(""), area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}
