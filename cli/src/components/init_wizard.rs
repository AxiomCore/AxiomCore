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
    let inner_area = centered_rect(60, 10, chunks[1]);
    let step_title = format!(
        " Step {}/{} ",
        state.init_context.current_step, state.init_context.total_steps
    );

    match state.init_context.step {
        InitStep::Language => {
            let title = format!(" Select Language ({}) ", step_title);
            let items: Vec<&str> = state
                .init_context
                .languages
                .iter()
                .map(|s| s.as_str())
                .collect();
            render_select_list(
                f,
                inner_area,
                &title,
                &items,
                state.init_context.selected_language,
            );
        }
        InitStep::Framework => {
            let title = format!(" Select Framework ({}) ", step_title);
            let frameworks = vec!["FastAPI", "Flask (Coming soon)", "Django (Coming soon)"];
            render_select_list(
                f,
                inner_area,
                &title,
                &frameworks,
                state.init_context.selected_framework,
            );
        }
        InitStep::ProjectName => {
            let title = format!(" Project Name ({}) ", step_title);
            render_input_box(
                f,
                inner_area,
                &title,
                &state.init_context.project_name,
                "my-axiom-project",
                state.init_context.cursor_position,
            );
        }
        InitStep::Entrypoint => {
            let title = format!(" Backend Entrypoint ({}) ", step_title);
            render_input_box(
                f,
                inner_area,
                &title,
                &state.init_context.entrypoint,
                "./main.py:app",
                state.init_context.cursor_position,
            );
        }
        InitStep::Success => {
            render_success_screen(f, chunks[1], state);
        }
    }

    // 3. Footer
    let help_text = match state.init_context.step {
        InitStep::ProjectName | InitStep::Entrypoint => {
            " [ENTER] Next  [ESC] Quit  [LEFT/RIGHT] Move Cursor "
        }
        InitStep::Success => " [q] Quit ",
        _ => " [ENTER] Next  [ESC] Quit  [UP/DOWN] Navigate ",
    };

    let help = Paragraph::new(help_text)
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
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_symbol(">> ");
    f.render_widget(list, area);
}

fn render_input_box(
    f: &mut Frame,
    area: Rect,
    title: &str,
    value: &str,
    placeholder: &str,
    cursor_pos: usize,
) {
    // Shrink the height to look like a proper text input field
    let input_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 3,
    };

    let text = if value.is_empty() {
        Span::styled(placeholder, Style::default().dim())
    } else {
        Span::raw(value)
    };

    let p = Paragraph::new(text).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(p, input_area);

    // ✨ MAGIC: Place the actual blinking terminal cursor inside the input box!
    f.set_cursor(input_area.x + 1 + cursor_pos as u16, input_area.y + 1);
}

fn render_success_screen(f: &mut Frame, area: Rect, _state: &State) {
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
