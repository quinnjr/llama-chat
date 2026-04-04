pub mod chat;
pub mod header;
pub mod input;
pub mod prompt;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::app::App;
use crate::config::theme::Theme;

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(f.area());

    header::draw(f, app, theme, chunks[0]);
    chat::draw(f, app, theme, chunks[1]);

    if app.pending_permission.is_some() {
        prompt::draw(f, app, theme, chunks[2]);
    } else {
        input::draw(f, app, theme, chunks[2]);
    }
}
