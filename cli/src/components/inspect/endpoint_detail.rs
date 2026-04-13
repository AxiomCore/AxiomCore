use crate::{normalize_validator_yaml, state::State};
use axiom_lib::config::{Backoff, CacheStrategy};
use ratatui::{prelude::*, widgets::*};

pub fn render_endpoint_detail(f: &mut Frame, area: Rect, state: &State) {
    let Some(contract) = state.inspect_context.contract.as_ref() else {
        return;
    };

    if contract.endpoints.is_empty()
        || state.inspect_context.selected_endpoint_idx >= contract.endpoints.len()
    {
        return;
    }

    let endpoint = &contract.endpoints[state.inspect_context.selected_endpoint_idx];

    let chunks = Layout::vertical([
        Constraint::Length(3),  // Summary Bar
        Constraint::Min(8),     // Parameters Table
        Constraint::Length(10), // Policies (Retry/Cache)
        Constraint::Length(4),  // Validation Status
    ])
    .split(area);

    // --- 1. Summary Bar ---
    let method_color = match endpoint.method.as_str() {
        "GET" => Color::Green,
        "POST" => Color::Blue,
        _ => Color::Yellow,
    };
    let summary = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {} ", endpoint.method.as_str()),
            Style::default()
                .bg(method_color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            &endpoint.path,
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(summary, chunks[0]);

    // --- 2. Parameters Table ---
    let mut rows = Vec::new();
    if let Some(ir_ep) = contract
        .ir
        .endpoints
        .iter()
        .find(|e| e.name == endpoint.name)
    {
        for param in &ir_ep.parameters {
            rows.push(Row::new(vec![
                Cell::from(param.name.clone()),
                Cell::from(format!("{:?}", param.source)),
                Cell::from(format!("{:?}", param.type_ref)),
                Cell::from(if param.is_optional {
                    "Optional"
                } else {
                    "Required"
                }),
            ]));
        }
    }
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ],
    )
    .header(
        Row::new(vec!["Parameter", "Source", "Type", "Status"])
            .style(Style::default().fg(Color::Cyan)),
    )
    .block(
        Block::default()
            .title(" Input Schema ")
            .borders(Borders::ALL),
    );
    f.render_widget(table, chunks[1]);

    // --- 3. Policies (Retry & Caching) ---
    let policy_chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[2]);

    // Retry Column
    let mut retry_lines = vec![];
    if let Some(retry) = &endpoint.retry_policy {
        retry_lines.push(Line::from(vec![
            Span::raw("Attempts: "),
            Span::styled(
                retry.max_attempts.to_string(),
                Style::default().fg(Color::Yellow),
            ),
        ]));

        let backoff_str = match &retry.backoff {
            Backoff::Fixed { initial_delay, .. } => format!("Fixed ({})", initial_delay),
            Backoff::Exponential {
                initial_delay,
                multiplier,
                ..
            } => format!("Exp ({}ms, x{})", initial_delay, multiplier),
            Backoff::Linear {
                initial_delay,
                step,
                ..
            } => format!("Linear ({}ms, +{})", initial_delay, step),
        };
        retry_lines.push(Line::from(vec![
            Span::raw("Backoff:  "),
            Span::styled(backoff_str, Style::default().fg(Color::Yellow)),
        ]));
        retry_lines.push(Line::from(vec![
            Span::raw("Timeout:  "),
            Span::styled(
                if retry.retry_on_timeout { "Yes" } else { "No" },
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        retry_lines.push(Line::from(Span::styled(
            "No Retry Policy",
            Style::default().dim(),
        )));
    }
    f.render_widget(
        Paragraph::new(retry_lines).block(
            Block::default()
                .title(" Retry Strategy ")
                .borders(Borders::ALL),
        ),
        policy_chunks[0],
    );

    // Cache Column
    let mut cache_lines = vec![];
    if let Some(cache) = &endpoint.cache_policy {
        let strategy_color = match cache.strategy {
            CacheStrategy::NetworkOnly => Color::Red,
            _ => Color::Green,
        };
        cache_lines.push(Line::from(vec![
            Span::raw("Strategy: "),
            Span::styled(
                format!("{:?}", cache.strategy),
                Style::default().fg(strategy_color),
            ),
        ]));
        cache_lines.push(Line::from(vec![
            Span::raw("TTL:      "),
            Span::styled(
                format!("{} seconds", cache.ttl_seconds),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        cache_lines.push(Line::from(Span::styled(
            "No Caching (Network Only)",
            Style::default().dim(),
        )));
    }
    f.render_widget(
        Paragraph::new(cache_lines).block(
            Block::default()
                .title(" Caching Policy ")
                .borders(Borders::ALL),
        ),
        policy_chunks[1],
    );

    // --- 4. Validation Status ---
    let mut validator_text = String::from("No validation rules defined.");

    // Check if the Return Type has a validator
    if let axiom_lib::ir::TypeRef::Named(ref name) = endpoint.return_type {
        if let Some(spec) = contract.validators.get(name) {
            validator_text = format!("Model: {}\n---\n{}", name, normalize_validator_yaml(spec));
        }
    }

    let validation_pane = Paragraph::new(validator_text)
        .block(
            Block::default()
                .title(Line::from(vec![Span::styled(
                    " Validation Rules ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )]))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default().fg(Color::Gray))
        .wrap(Wrap { trim: false }); // Ensure it wraps instead of truncating with ...

    f.render_widget(validation_pane, chunks[3]);
}
