use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::config::theme::Theme;

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let prompt_char = Span::styled("▸ ", Style::default().fg(theme.muted));
    let input_text = Span::styled(app.input_buffer.as_str(), Style::default().fg(theme.fg));

    let hints = "/help · /model · /server";
    let hints_span = Span::styled(hints, Style::default().fg(theme.border));

    let line = Line::from(vec![prompt_char, input_text]);

    let input = Paragraph::new(vec![line, Line::from(hints_span)])
        .wrap(Wrap { trim: false });
    f.render_widget(input, area);

    let width = area.width as usize;
    if width > 0 {
        let total_offset = "▸ ".width() + app.input_buffer.width();
        let cursor_x = area.x + (total_offset % width) as u16;
        let cursor_y = area.y + (total_offset / width) as u16;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}
