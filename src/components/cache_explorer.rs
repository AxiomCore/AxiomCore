use crate::commands::cache::CacheEntry;
use ratatui::{prelude::*, widgets::*};

pub fn render_cache_explorer(
    f: &mut Frame,
    area: Rect,
    entries: &[(String, CacheEntry)],
    selected_index: usize,
) {
    let chunks = Layout::horizontal([
        Constraint::Percentage(40), // Key list
        Constraint::Percentage(60), // Payload viewer
    ])
    .split(area);

    // 1. List of Keys
    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, (key, _))| {
            let style = if i == selected_index {
                Style::default()
                    .bg(Color::Indexed(240))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!(" • {}", key)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Cache Keys ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(list, chunks[0]);

    // 2. Payload Detail
    if let Some((_, entry)) = entries.get(selected_index) {
        let json_payload = serde_json::from_slice::<serde_json::Value>(&entry.payload)
            .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
            .unwrap_or_else(|_| "[Binary Data]".to_string());

        let detail = Paragraph::new(json_payload)
            .block(
                Block::default()
                    .title(" Payload Preview ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(detail, chunks[1]);
    }
}
