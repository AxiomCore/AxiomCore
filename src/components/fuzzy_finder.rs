use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::{prelude::*, widgets::*};
use tui_input::Input;

pub struct ProjectItem {
    pub id: String,
    pub name: String,
}

pub fn render_fuzzy_finder(
    f: &mut Frame,
    area: Rect,
    input: &Input,
    projects: &[ProjectItem],
    selected_index: usize,
) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // Search box
        Constraint::Min(0),    // List
    ])
    .split(area);

    // 1. Search Box
    let search_block = Block::default()
        .borders(Borders::ALL)
        .title(" Search Projects (Type to filter) ")
        .border_style(Style::default().fg(Color::Cyan));

    let input_text = Paragraph::new(input.value()).block(search_block);
    f.render_widget(input_text, chunks[0]);

    // 2. Filtered List
    let matcher = SkimMatcherV2::default();
    let filtered: Vec<&ProjectItem> = projects
        .iter()
        .filter(|p| {
            input.value().is_empty() || matcher.fuzzy_match(&p.name, input.value()).is_some()
        })
        .collect();

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == selected_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<20} ", p.name), style),
                Span::styled(format!(" ({})", p.id), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM))
        .highlight_symbol(">> ");

    f.render_widget(list, chunks[1]);
}
