use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::config::theme::Theme;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let prompt_char = Span::styled("▸ ", Style::default().fg(theme.muted));
    let input_text = Span::styled(app.input_buffer.as_str(), Style::default().fg(theme.fg));

    let hints = "/help · /model · /server";
    let hints_span = Span::styled(hints, Style::default().fg(theme.border));

    let line = Line::from(vec![prompt_char, input_text]);

    let input = Paragraph::new(vec![line, Line::from(hints_span)]);
    f.render_widget(input, area);

    let cursor_x = area.x + 2 + app.input_buffer.len() as u16;
    let cursor_y = area.y;
    f.set_cursor_position((cursor_x, cursor_y));
}
