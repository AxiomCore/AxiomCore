use crate::state::{InitStep, State};
use ratatui::{prelude::*, widgets::*};

pub fn render_init_wizard(f: &mut Frame, area: Rect, state: &State) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Title
        Constraint::Min(0),    // Content
        Constraint::Length(3), // Footer/Help
    ])
    .margin(2)
    .split(area);

    // 1. Title
    let title = Paragraph::new(" AXIOM PROJECT INITIALIZATION ")
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    f.render_widget(title, chunks[0]);

    // 2. Content based on step
    let inner_area = centered_rect(60, 50, chunks[1]);

    match state.init_context.step {
        InitStep::Language => {
            let langs = vec![
                "Python",
                "Go (Coming soon)",
                "Rust (Coming soon)",
                "Javascript (Coming soon)",
                "Dart (Coming soon)",
            ];
            render_select_list(
                f,
                inner_area,
                "Select Language",
                &langs,
                state.init_context.selected_language,
            );
        }
        InitStep::Framework => {
            let frameworks = vec!["FastAPI", "Flask (Coming soon)", "Django (Coming soon)"];
            render_select_list(
                f,
                inner_area,
                "Select Framework",
                &frameworks,
                state.init_context.selected_framework,
            );
        }
        InitStep::ProjectName => {
            render_input_box(
                f,
                inner_area,
                "Project Name",
                &state.init_context.project_name,
                "my-axiom-project",
            );
        }
        InitStep::Version => {
            render_input_box(
                f,
                inner_area,
                "Contract Version",
                &state.init_context.version,
                "v0.0.1",
            );
        }
        InitStep::Entrypoint => {
            render_input_box(
                f,
                inner_area,
                "Backend Entrypoint",
                &state.init_context.entrypoint,
                "./main.py:app",
            );
        }
        InitStep::Success => {
            render_success_screen(f, chunks[1], state);
        }
    }

    // 3. Footer
    let help = Paragraph::new(" [ENTER] Next  [ESC] Quit  [UP/DOWN] Navigate ")
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, chunks[2]);
}

fn render_select_list(f: &mut Frame, area: Rect, title: &str, items: &[&str], selected: usize) {
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, &item)| {
            let style = if i == selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if item.contains("(Coming soon)") {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(format!("  {}  ", item)).style(style)
        })
        .collect();

    let list = List::new(list_items)
        .block(
            Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL),
        )
        .highlight_symbol(">> ");
    f.render_widget(list, area);
}

fn render_input_box(f: &mut Frame, area: Rect, title: &str, value: &str, placeholder: &str) {
    let text = if value.is_empty() {
        Span::styled(placeholder, Style::default().dim())
    } else {
        Span::raw(value)
    };

    let p = Paragraph::new(text).block(
        Block::default()
            .title(format!(" {} ", title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, area);
}

fn render_success_screen(f: &mut Frame, area: Rect, state: &State) {
    let msg = vec![
        Line::from(Span::styled(
            "✨ Project Initialized Successfully!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("Created "),
            Span::styled("axiom.acore", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Next Steps:",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )),
        Line::from("  1. axiom build      - Introspect backend and build local contract"),
        Line::from("  2. axiom watch      - Start live development HUD"),
        Line::from("  3. axiom inspect    - Browse your generated API surface"),
        Line::from(""),
        Line::from(Span::styled("Press [q] to exit", Style::default().dim())),
    ];
    f.render_widget(Paragraph::new(msg).alignment(Alignment::Center), area);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}
