use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{App, ChatEntry};
use crate::config::theme::Theme;

#[cfg(not(tarpaulin_include))]
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
            ChatEntry::Thinking(text) => {
                for think_line in text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  \u{1f4ad} {}", think_line),
                        Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC),
                    )));
                }
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

    // Streaming tool output (displayed before the command finishes)
    if !app.tool_output_buffer.is_empty() {
        for line in app.tool_output_buffer.lines() {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(theme.muted),
            )));
        }
    }

    // Streaming assistant response with think-tag parsing for live display
    if !app.streaming_buffer.is_empty() {
        let mut remaining = app.streaming_buffer.as_str();
        while !remaining.is_empty() {
            if let Some(think_start) = remaining.find("<think>") {
                let before = &remaining[..think_start];
                if !before.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", app.active_model),
                            Style::default().fg(theme.assistant_text),
                        ),
                        Span::styled(before, Style::default().fg(theme.fg)),
                    ]));
                }
                remaining = &remaining[think_start + "<think>".len()..];
                if let Some(think_end) = remaining.find("</think>") {
                    let thinking = &remaining[..think_end];
                    for think_line in thinking.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  \u{1f4ad} {}", think_line),
                            Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC),
                        )));
                    }
                    remaining = &remaining[think_end + "</think>".len()..];
                } else {
                    // Still in thinking (unclosed tag — thinking is streaming)
                    for think_line in remaining.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  \u{1f4ad} {}", think_line),
                            Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC),
                        )));
                    }
                    remaining = "";
                }
            } else {
                // No think tags — normal assistant content
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}: ", app.active_model),
                        Style::default().fg(theme.assistant_text),
                    ),
                    Span::styled(remaining, Style::default().fg(theme.fg)),
                ]));
                remaining = "";
            }
        }
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
