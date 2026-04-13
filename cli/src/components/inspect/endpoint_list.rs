use crate::state::State;
use ratatui::{prelude::*, widgets::*};

// src/components/inspect/endpoint_list.rs
pub fn render_endpoint_list(f: &mut Frame, area: Rect, state: &State) {
    let contract = state.inspect_context.contract.as_ref().unwrap();

    let items: Vec<ListItem> = contract
        .endpoints
        .iter()
        .filter(|e| {
            e.name.contains(&state.inspect_context.filter_query)
                || e.path.contains(&state.inspect_context.filter_query)
        })
        .enumerate()
        .map(|(i, e)| {
            let style = if i == state.inspect_context.selected_endpoint_idx {
                Style::default()
                    .bg(Color::Indexed(237))
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let method_color = match e.method.as_str() {
                "GET" => Color::Green,
                "POST" => Color::Blue,
                "DELETE" => Color::Red,
                _ => Color::Yellow,
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<6}", e.method.as_str()),
                    Style::default().fg(method_color),
                ),
                Span::styled(&e.name, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title(" Endpoints ").borders(Borders::ALL))
        .highlight_symbol(">> ");
    f.render_widget(list, area);
}
