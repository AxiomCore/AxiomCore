use crate::error_reporter::ErrorDiagnostic;
use ratatui::{prelude::*, widgets::*};

pub fn render_error_overlay(f: &mut Frame, diag: &ErrorDiagnostic) {
    let area = centered_rect(80, 60, f.size());
    f.render_widget(Clear, area); // Block out the background

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " ERROR ",
                Style::default()
                    .bg(Color::Red)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" {} ", diag.title)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .border_type(BorderType::Thick);

    let chunks = Layout::vertical([
        Constraint::Length(3), // Message
        Constraint::Min(5),    // Code Snippet
        Constraint::Length(3), // Direction
    ])
    .margin(1)
    .split(area);

    // 1. Error Message
    f.render_widget(
        Paragraph::new(diag.message.as_str()).wrap(Wrap { trim: true }),
        chunks[0],
    );

    // 2. Code Snippet
    if let Some(ref snippet) = diag.code_snippet {
        let snippet_text = snippet.join("\n");
        let path_text = format!(
            "--> {}:{}",
            diag.file_path.as_ref().unwrap().display(),
            diag.line.unwrap()
        );

        let snippet_widget = Paragraph::new(format!("{}\n\n{}", path_text, snippet_text))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Source Context ")
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .style(Style::default().fg(Color::Gray));
        f.render_widget(snippet_widget, chunks[1]);
    }

    // 3. Direction
    let direction_widget = Paragraph::new(format!("💡 Direction: {}", diag.direction)).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    );
    f.render_widget(direction_widget, chunks[2]);

    f.render_widget(block, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
