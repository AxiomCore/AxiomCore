use crate::state::{LoginStatus, State};
use ratatui::{prelude::*, widgets::*};

pub fn render_login_screen(f: &mut Frame, area: Rect, state: &State) {
    // Create a larger centered area for the login portal
    let centered_area = centered_rect(70, 50, area);

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " AXIOM CLOUD ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" AUTHENTICATION "),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .border_type(BorderType::Thick);

    let inner = block.inner(centered_area);
    f.render_widget(block, centered_area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // Top padding
        Constraint::Min(0),    // Content
        Constraint::Length(1), // Bottom help
    ])
    .margin(1)
    .split(inner);

    match &state.login_context.status {
        LoginStatus::WaitingForUser { code, url } => {
            let msg = vec![
                Line::from(vec![
                    Span::raw("Attempting to authenticate with "),
                    Span::styled(
                        "Axiom Cloud",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from("Please visit the following URL in your browser:"),
                Line::from(Span::styled(
                    url,
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("And enter the authorization code: "),
                    Span::styled(
                        format!(" {} ", code),
                        Style::default()
                            .bg(Color::White)
                            .fg(Color::Black)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled(
                    " ⌛ Waiting for browser authorization... ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                )),
            ];

            // Using Alignment::Center on the whole paragraph makes the block feel balanced
            let content = Paragraph::new(msg)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false });
            f.render_widget(content, chunks[1]);
        }
        LoginStatus::Verifying => {
            let msg = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    " ⟳ Verifying credentials... ",
                    Style::default().fg(Color::Cyan),
                )),
            ])
            .alignment(Alignment::Center);
            f.render_widget(msg, chunks[1]);
        }
        LoginStatus::Success => {
            let msg = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    " ✅ LOGIN SUCCESSFUL ",
                    Style::default()
                        .bg(Color::Green)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("Tokens have been saved to your local config."),
                Line::from(""),
                Line::from(Span::styled(
                    "Press [q] or [Enter] to exit",
                    Style::default().dim(),
                )),
            ])
            .alignment(Alignment::Center);
            f.render_widget(msg, chunks[1]);
        }
        LoginStatus::Error(e) => {
            let msg = Paragraph::new(vec![
                Line::from(Span::styled(
                    " ❌ AUTHENTICATION FAILED ",
                    Style::default()
                        .bg(Color::Red)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(e.as_str()),
                Line::from(""),
                Line::from(Span::styled(
                    "Press [r] to retry or [Esc] to cancel",
                    Style::default().dim(),
                )),
            ])
            .alignment(Alignment::Center);
            f.render_widget(msg, chunks[1]);
        }
        _ => {}
    }

    let footer = Paragraph::new(" [ESC] Cancel  [q] Quit ")
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[2]);
}

// Ensure this helper is available in the file
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
