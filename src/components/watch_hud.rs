use crate::state::State;
use ratatui::{prelude::*, widgets::*};

pub fn render_watch_hud(f: &mut Frame, area: Rect, state: &State) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Status Line
        Constraint::Min(0),    // IR Diff / Change Log
        Constraint::Length(4), // Build Status (Conditional)
    ])
    .split(area);

    // 1. Status Line
    let status_color = if state.is_rebuilding {
        Color::Yellow
    } else {
        Color::Green
    };
    let status_text = if state.is_rebuilding {
        " ⟳ REBUILDING CONTRACT... "
    } else {
        " 👁 WATCHING FOR CHANGES "
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            status_text,
            Style::default()
                .bg(status_color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" Last Sync: {} ", state.last_sync_time)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // 2. IR Diff View
    let mut diff_items = vec![ListItem::new(Span::styled(
        "--- Changes in Contract ---",
        Style::default().dim(),
    ))];

    for add in &state.watch_diff.added_endpoints {
        diff_items.push(ListItem::new(Span::styled(
            format!(" + [NEW] {}", add),
            Style::default().fg(Color::Green),
        )));
    }
    for rem in &state.watch_diff.removed_endpoints {
        diff_items.push(ListItem::new(Span::styled(
            format!(" - [REM] {}", rem),
            Style::default().fg(Color::Red),
        )));
    }

    let diff_list = List::new(diff_items).block(
        Block::default()
            .title(" Contract Impact ")
            .borders(Borders::ALL),
    );
    f.render_widget(diff_list, chunks[1]);

    // 3. Build Status (Only if --build is passed)
    if state.watch_build_enabled {
        let build_msg = format!(
            "Local Artifact: {} (Hash: {})",
            ".axiom", state.last_schema_hash
        );
        let build_pane = Paragraph::new(build_msg)
            .block(
                Block::default()
                    .title(" Local Build ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .style(Style::default().fg(Color::Gray));
        f.render_widget(build_pane, chunks[2]);
    }
}
