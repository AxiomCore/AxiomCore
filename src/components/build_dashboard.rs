use crate::state::State;
use axiom_lib::action::StepStatus;
use ratatui::{prelude::*, widgets::*};

pub fn render_build_dashboard(f: &mut Frame, area: Rect, state: &State) {
    let build = &state.build_context;

    // Layout
    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(10),   // Main Body (Pipeline + Stats)
        Constraint::Length(5), // Log Summary
    ])
    .split(area);

    let main_body = Layout::horizontal([
        Constraint::Percentage(50), // Pipeline
        Constraint::Percentage(50), // Stats
    ])
    .split(chunks[1]);

    // 1. Header
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " AXIOM BUILD ",
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" Project: {} ", build.project_id)),
        Span::styled(
            format!(" v{} ", build.version),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(format!(" Variant: {} ", build.variant)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(header, chunks[0]);

    // 2. Left Pane: Pipeline Steps
    let steps: Vec<ListItem> = build
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let (icon, color) = match step.status {
                StepStatus::Waiting => ("○", Color::DarkGray),
                StepStatus::Processing => ("▶", Color::Yellow),
                StepStatus::Success => ("✔", Color::Green),
                StepStatus::Failed => ("✘", Color::Red),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icon), Style::default().fg(color)),
                Span::styled(
                    &step.name,
                    Style::default().fg(if i == state.current_step_idx {
                        Color::White
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]))
        })
        .collect();

    let pipeline = List::new(steps).block(
        Block::default()
            .title(" Build Pipeline ")
            .borders(Borders::ALL),
    );
    f.render_widget(pipeline, main_body[0]);

    // 3. Right Pane: Live Stats
    let stats_text = vec![
        Line::from(vec![
            Span::raw("Endpoints Found: "),
            Span::styled(
                build.endpoints_count.to_string(),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::raw("Schema Hash:    "),
            Span::styled(&build.schema_hash, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Target: iOS/Android/Web",
            Style::default().add_modifier(Modifier::ITALIC),
        )),
    ];
    let stats =
        Paragraph::new(stats_text).block(Block::default().title(" Context ").borders(Borders::ALL));
    f.render_widget(stats, main_body[1]);

    // 4. Bottom Pane: Mini-Log
    let recent_logs: Vec<ListItem> = build
        .logs
        .iter()
        .rev()
        .take(3)
        .map(|l| ListItem::new(l.as_str()))
        .collect();
    let logs = List::new(recent_logs)
        .block(
            Block::default()
                .title(" Output ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::Gray));
    f.render_widget(logs, chunks[2]);
}
