pub mod chat;
pub mod header;
pub mod input;
pub mod prompt;
pub mod sidebar;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::config::theme::Theme;

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme) {
    // Horizontal split: main area | sidebar
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(30)])
        .split(f.area());

    let input_height = if app.pending_permission.is_some() {
        3
    } else {
        let total_width = h_chunks[0].width as usize;
        let prompt_len = "▸ ".width() + app.input_buffer.width(); // display width
        let wrapped_lines = if total_width > 0 {
            prompt_len.div_ceil(total_width).max(1)
        } else {
            1
        };
        (wrapped_lines as u16 + 1).max(2) // +1 for hints, at least 2
    };

    // Vertical split for main area: header | chat | input
    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(input_height),
        ])
        .split(h_chunks[0]);

    header::draw(f, app, theme, v_chunks[0]);
    chat::draw(f, app, theme, v_chunks[1]);

    if app.pending_permission.is_some() {
        prompt::draw(f, app, theme, v_chunks[2]);
    } else {
        input::draw(f, app, theme, v_chunks[2]);
    }

    sidebar::draw(f, app, theme, h_chunks[1]);
}
