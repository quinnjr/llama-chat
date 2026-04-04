use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::config::theme::Theme;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let left = Span::styled(
        "llama-chat",
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
    );

    let model_span = Span::styled(
        format!("[{}]", app.active_model),
        Style::default().fg(theme.tool_ok),
    );
    let server_span = Span::styled(
        format!(" [{}]", app.active_server_name),
        Style::default().fg(theme.muted),
    );
    let tool_span = Span::styled(
        format!(" [{} tools]", app.tool_count),
        Style::default().fg(theme.muted),
    );

    let right_text = format!(
        "[{}] [{}] [{} tools]",
        app.active_model, app.active_server_name, app.tool_count
    );
    let padding = area.width.saturating_sub(11 + right_text.len() as u16);

    let line = Line::from(vec![
        left,
        Span::raw(" ".repeat(padding as usize)),
        model_span,
        server_span,
        tool_span,
    ]);

    let header = Paragraph::new(line).style(Style::default().bg(theme.bg).fg(theme.fg));
    f.render_widget(header, area);
}
