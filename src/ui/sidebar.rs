use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::config::theme::Theme;

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    if app.todo_items.is_empty() {
        lines.push(Line::from(Span::styled(
            "No tasks",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
    } else {
        for (i, item) in app.todo_items.iter().enumerate() {
            let checkbox = if item.done { "[x]" } else { "[ ]" };
            let style = if item.done {
                Style::default().fg(theme.tool_ok)
            } else {
                Style::default().fg(theme.fg)
            };

            // Truncate text to fit sidebar (30 width - 2 border - 4 checkbox - 3 index)
            let max_text = area.width.saturating_sub(2) as usize;
            let prefix = format!("{checkbox} {}. ", i);
            let available = max_text.saturating_sub(prefix.len());
            let text = if item.text.len() > available && available > 3 {
                format!("{}...", &item.text[..available - 3])
            } else {
                item.text.clone()
            };

            lines.push(Line::from(vec![
                Span::styled(format!("{checkbox} "), style),
                Span::styled(format!("{i}. "), Style::default().fg(theme.muted)),
                Span::styled(text, style),
            ]));
        }
    }

    let block = Block::default()
        .title("Todo")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}
