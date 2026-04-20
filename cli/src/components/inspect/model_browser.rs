use crate::state::State;
use ratatui::{prelude::*, widgets::*};

pub fn render_model_browser(f: &mut Frame, area: Rect, state: &State) {
    let Some(contract) = state.inspect_context.contract.as_ref() else {
        return;
    };

    // 1. Create a unified list of Types (Models + Enums)
    let mut all_types: Vec<(String, bool)> = Vec::new(); // (Name, IsModel)

    for name in contract.ir.models.keys() {
        all_types.push((name.clone(), true));
    }
    for name in contract.ir.enums.keys() {
        all_types.push((name.clone(), false));
    }

    // Sort alphabetically so the list is stable
    all_types.sort_by(|a, b| a.0.cmp(&b.0));

    if all_types.is_empty() {
        f.render_widget(
            Paragraph::new("No models or enums defined in IR")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }

    let chunks =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).split(area);

    // --- Left Pane: The Unified List ---
    let items: Vec<ListItem> = all_types
        .iter()
        .enumerate()
        .map(|(i, (name, is_model))| {
            let style = if i == state.inspect_context.selected_model_idx {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let tag = if *is_model {
                Span::styled("[M] ", Style::default().fg(Color::Blue).dim())
            } else {
                Span::styled("[E] ", Style::default().fg(Color::Magenta).dim())
            };

            ListItem::new(Line::from(vec![tag, Span::styled(name, style)]))
        })
        .collect();

    f.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Types (Models & Enums) ")
                .borders(Borders::ALL),
        ),
        chunks[0],
    );

    // --- Right Pane: Details ---
    let right_chunks = Layout::vertical([
        Constraint::Length(12), // Definition structure
        Constraint::Min(0),     // Validator YAML
    ])
    .split(chunks[1]);

    let (selected_name, is_model) = &all_types[state.inspect_context.selected_model_idx];

    // 2. Render Structure (Fields for Model, Variants for Enum)
    let mut structure_info = vec![];

    if *is_model {
        // Handle Model Rendering
        if let Some(model_def) = contract.ir.models.get(selected_name) {
            for field in &model_def.fields {
                structure_info.push(Line::from(vec![
                    Span::styled(
                        format!("{:<15}", field.1.name),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!("{:?}", field.1.type_ref),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(if field.1.is_optional {
                        " (Optional)"
                    } else {
                        ""
                    }),
                ]));
            }
        }
    } else {
        // Handle Enum Rendering
        if let Some(enum_def) = contract.ir.enums.get(selected_name) {
            structure_info.push(Line::from(Span::styled(
                "Variants:",
                Style::default().dim().add_modifier(Modifier::ITALIC),
            )));
            for value in &enum_def.values {
                structure_info.push(Line::from(vec![
                    Span::styled(" • ", Style::default().fg(Color::Magenta)),
                    Span::styled(
                        value,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        }
    }

    let title = if *is_model {
        " Model Structure "
    } else {
        " Enum Variants "
    };
    f.render_widget(
        Paragraph::new(structure_info).block(Block::default().title(title).borders(Borders::ALL)),
        right_chunks[0],
    );

    // 3. Render Normalized Validator (Always show YAML if exists)
    let validator_yaml = if let Some(spec) = contract.validators.get(selected_name) {
        // Use the normalization helper from previous step
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(spec) {
            serde_yaml::to_string(&value).unwrap_or_else(|_| spec.clone())
        } else {
            spec.clone()
        }
    } else {
        "  constraints: []\n  info: \"No additional validation logic\"".to_string()
    };

    let validator_pane = Paragraph::new(validator_yaml)
        .block(
            Block::default()
                .title(format!(" Validator Details: {} ", selected_name))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::Green))
        .wrap(Wrap { trim: false });

    f.render_widget(validator_pane, right_chunks[1]);
}
