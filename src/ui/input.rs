use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::config::theme::Theme;

/// Split text into lines at character display-width boundaries.
fn char_wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0;

    for ch in text.chars() {
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_w + ch_w > width && current_w > 0 {
            lines.push(std::mem::take(&mut current));
            current_w = 0;
        }
        current.push(ch);
        current_w += ch_w;
    }
    lines.push(current);
    lines
}

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let width = area.width as usize;
    if width == 0 {
        return;
    }

    let prompt_str = "▸ ";
    let prompt_byte_len = prompt_str.len();
    let full_text = format!("{prompt_str}{}", app.input_buffer);
    let segments = char_wrap(&full_text, width);

    let mut visual_lines: Vec<Line> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        if i == 0 && seg.len() >= prompt_byte_len {
            visual_lines.push(Line::from(vec![
                Span::styled(
                    seg[..prompt_byte_len].to_string(),
                    Style::default().fg(theme.muted),
                ),
                Span::styled(
                    seg[prompt_byte_len..].to_string(),
                    Style::default().fg(theme.fg),
                ),
            ]));
        } else {
            visual_lines.push(Line::from(Span::styled(
                seg.clone(),
                Style::default().fg(if i == 0 { theme.muted } else { theme.fg }),
            )));
        }
    }

    visual_lines.push(Line::from(Span::styled(
        "/help · /model · /server",
        Style::default().fg(theme.border),
    )));

    let input = Paragraph::new(visual_lines);
    f.render_widget(input, area);

    let total_offset = prompt_str.width() + app.input_buffer.width();
    let cursor_x = area.x + (total_offset % width) as u16;
    let cursor_y = area.y + (total_offset / width) as u16;
    f.set_cursor_position((cursor_x, cursor_y));
}
