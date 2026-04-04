use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, ChatEntry};
use crate::config::theme::Theme;

pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.messages {
        match entry {
            ChatEntry::User(text) => {
                lines.push(Line::from(vec![
                    Span::styled("you: ", Style::default().fg(theme.user_text)),
                    Span::styled(text.as_str(), Style::default().fg(theme.fg)),
                ]));
                lines.push(Line::raw(""));
            }
            ChatEntry::Assistant(text) => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}: ", app.active_model),
                        Style::default().fg(theme.assistant_text),
                    ),
                    Span::styled(text.as_str(), Style::default().fg(theme.fg)),
                ]));
                lines.push(Line::raw(""));
            }
            ChatEntry::ToolCall { name, command, status } => {
                let status_span = match status.as_str() {
                    "allowed" | "ok" => Span::styled(
                        format!("✓ {status}"),
                        Style::default().fg(theme.tool_ok),
                    ),
                    "denied" => Span::styled(
                        format!("✗ {status}"),
                        Style::default().fg(theme.tool_denied),
                    ),
                    _ => Span::styled(
                        format!("⏳ {status}"),
                        Style::default().fg(theme.muted),
                    ),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("⚙ {name} "), Style::default().fg(theme.tool_name)),
                    Span::styled(command.as_str(), Style::default().fg(theme.muted)),
                    Span::raw(" "),
                    status_span,
                ]));
            }
            ChatEntry::ToolOutput(text) => {
                lines.push(Line::from(Span::styled(
                    text.as_str(),
                    Style::default().fg(theme.muted),
                )));
                lines.push(Line::raw(""));
            }
            ChatEntry::System(text) => {
                lines.push(Line::from(Span::styled(
                    text.as_str(),
                    Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC),
                )));
                lines.push(Line::raw(""));
            }
        }
    }

    if !app.streaming_buffer.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}: ", app.active_model),
                Style::default().fg(theme.assistant_text),
            ),
            Span::styled(app.streaming_buffer.as_str(), Style::default().fg(theme.fg)),
        ]));
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let scroll = if lines.len() > visible_height {
        (lines.len() - visible_height) as u16
    } else {
        0
    };

    let chat = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(theme.border)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    f.render_widget(chat, area);
}
