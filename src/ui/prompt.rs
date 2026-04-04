use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::config::theme::Theme;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let perm = match &app.pending_permission {
        Some(p) => p,
        None => return,
    };

    if let Some(ref pattern_buf) = app.pattern_input {
        let input_line = Line::from(vec![
            Span::styled(
                "Pattern: ",
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(pattern_buf.as_str(), Style::default().fg(theme.fg)),
            Span::styled("\u{25CF}", Style::default().fg(theme.fg)),
        ]);
        let hint_line = Line::from(Span::styled(
            "Enter to save, Esc to cancel. Use * as wildcard (e.g. 'git *', 'cargo *')",
            Style::default().fg(theme.muted),
        ));
        let prompt = Paragraph::new(vec![input_line, hint_line]);
        f.render_widget(prompt, area);
        return;
    }

    let tool_line = Line::from(vec![
        Span::styled(
            format!("⚙ {} ", perm.tool_name),
            Style::default().fg(theme.tool_name),
        ),
        Span::styled(perm.command.as_str(), Style::default().fg(theme.fg)),
    ]);

    let options_line = Line::from(vec![
        Span::styled(
            "[A]",
            Style::default().fg(theme.tool_ok).add_modifier(Modifier::BOLD),
        ),
        Span::styled("llow  ", Style::default().fg(theme.fg)),
        Span::styled(
            "[D]",
            Style::default().fg(theme.tool_denied).add_modifier(Modifier::BOLD),
        ),
        Span::styled("eny  ", Style::default().fg(theme.fg)),
        Span::styled(
            "[S]",
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("ave always  ", Style::default().fg(theme.fg)),
        Span::styled(
            "[P]",
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("attern...", Style::default().fg(theme.fg)),
    ]);

    let prompt = Paragraph::new(vec![tool_line, options_line]);
    f.render_widget(prompt, area);
}
