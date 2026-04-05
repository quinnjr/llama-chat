use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::config::theme::Theme;

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let version = env!("CARGO_PKG_VERSION");

    // Health indicator
    let (health_dot, health_label, health_color) = match app.server_healthy {
        Some(true) => ("●", "Online", theme.tool_ok),
        Some(false) => ("●", "Offline", theme.tool_denied),
        None => ("●", "Checking", theme.thinking_header),
    };

    // Line 1: model, server, health
    let line1 = Line::from(vec![
        Span::styled("Model: ", Style::default().fg(theme.muted)),
        Span::styled(&app.active_model, Style::default().fg(theme.tool_ok)),
        Span::styled("  Server: ", Style::default().fg(theme.muted)),
        Span::styled(&app.active_server_name, Style::default().fg(theme.fg)),
        Span::styled("  Status: ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("{health_dot} {health_label}"),
            Style::default().fg(health_color),
        ),
    ]);

    // Line 2: token usage
    let line2 = if let Some(ref usage) = app.last_token_usage {
        Line::from(vec![
            Span::styled("Tokens: ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{} prompt", usage.prompt_tokens),
                Style::default().fg(theme.fg),
            ),
            Span::styled(" / ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{} completion", usage.completion_tokens),
                Style::default().fg(theme.fg),
            ),
            Span::styled(" / ", Style::default().fg(theme.muted)),
            Span::styled(
                format!("{} total", usage.total_tokens),
                Style::default().fg(theme.fg),
            ),
        ])
    } else {
        Line::from(Span::styled(
            "Tokens: \u{2014}",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        ))
    };

    let block = Block::default()
        .title(format!("llama-chat v{version}"))
        .title_style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let header = Paragraph::new(vec![line1, line2])
        .block(block)
        .style(Style::default().bg(theme.bg));

    f.render_widget(header, area);
}
