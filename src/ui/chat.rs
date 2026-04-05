use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ChatEntry};
use crate::config::theme::Theme;

const THINKING_HEADER: &str = "Reasoning";
const COLLAPSE_ICON: &str = "[-]";
const EXPAND_ICON: &str = "[+]";

fn get_collapsed_preview(content: &str, max_len: usize) -> String {
    let first_line = content.lines().next().unwrap_or(content);
    let preview = if first_line.len() > max_len {
        format!("{}...", &first_line[..max_len])
    } else {
        first_line.to_string()
    };
    let line_count = content.lines().count();
    if line_count > 1 {
        format!("{preview} ({line_count} lines)")
    } else {
        preview
    }
}

#[cfg(not(tarpaulin_include))]
pub fn draw(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.messages {
        match entry {
            ChatEntry::User(text) => {
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("you: ", Style::default().fg(theme.user_text)),
                        Span::styled(line, Style::default().fg(theme.fg)),
                    ]));
                }
                lines.push(Line::raw(""));
            }
            ChatEntry::Assistant(text) => {
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", app.active_model),
                            Style::default().fg(theme.assistant_text),
                        ),
                        Span::styled(line, Style::default().fg(theme.fg)),
                    ]));
                }
                lines.push(Line::raw(""));
            }
            ChatEntry::ToolCall {
                name,
                command,
                status,
            } => {
                let status_span = match status.as_str() {
                    "allowed" | "ok" => {
                        Span::styled(format!("✓ {status}"), Style::default().fg(theme.tool_ok))
                    }
                    "denied" => Span::styled(
                        format!("✗ {status}"),
                        Style::default().fg(theme.tool_denied),
                    ),
                    _ => Span::styled(format!("⏳ {status}"), Style::default().fg(theme.muted)),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("⚙ {name} "), Style::default().fg(theme.tool_name)),
                    status_span,
                ]));
                for cmd_line in command.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {cmd_line}"),
                        Style::default().fg(theme.muted),
                    )));
                }
            }
            ChatEntry::ToolOutput(text) => {
                for output_line in text.lines() {
                    lines.push(Line::from(Span::styled(
                        output_line,
                        Style::default().fg(theme.muted),
                    )));
                }
                lines.push(Line::raw(""));
            }
            ChatEntry::Thinking { content, collapsed } => {
                if !app.show_thinking {
                    continue;
                }

                let indicator = if *collapsed {
                    EXPAND_ICON
                } else {
                    COLLAPSE_ICON
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} {} ", THINKING_HEADER, indicator),
                        Style::default()
                            .fg(theme.thinking_header)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "\u{2500}".to_string(),
                        Style::default()
                            .fg(theme.thinking_border)
                            .add_modifier(Modifier::REVERSED),
                    ),
                ]));

                if *collapsed {
                    let preview = get_collapsed_preview(content, 60);
                    lines.push(Line::from(Span::styled(
                        format!("  \u{2026} {preview}"),
                        Style::default()
                            .fg(theme.muted)
                            .add_modifier(Modifier::ITALIC),
                    )));
                } else {
                    for think_line in content.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  \u{2502} {}", think_line),
                            Style::default()
                                .fg(theme.thinking_text)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                }

                lines.push(Line::from(Span::styled(
                    format!("  \u{2514}\u{2500}\u{2500}"),
                    Style::default().fg(theme.thinking_border),
                )));
                lines.push(Line::raw(""));
            }
            ChatEntry::System(text) => {
                for line in text.lines() {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(theme.muted)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                lines.push(Line::raw(""));
            }
            ChatEntry::SubagentOutput { .. } => {
                // Subagent rendering not yet implemented — handled in a later task.
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

    // Show a waiting indicator before the LLM has produced any output
    if app.waiting_for_response && app.streaming_buffer.is_empty() {
        let dots = match app.anim_frame % 4 {
            0 => "\u{2022}",
            1 => "\u{2022}\u{2022}",
            2 => "\u{2022}\u{2022}\u{2022}",
            _ => "\u{2022}\u{2022}",
        };
        lines.push(Line::from(Span::styled(
            format!("Thinking {dots}"),
            Style::default()
                .fg(theme.thinking_header)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Streaming assistant response with think-tag parsing for live display
    if !app.streaming_buffer.is_empty() && app.show_thinking {
        let mut remaining = app.streaming_buffer.as_str();
        let mut in_thinking = false;
        let mut thinking_content = String::new();

        while !remaining.is_empty() {
            if let Some(think_start) = remaining.find("<think>") {
                let before = &remaining[..think_start];
                if !before.is_empty() {
                    if in_thinking {
                        for think_line in thinking_content.lines() {
                            lines.push(Line::from(Span::styled(
                                format!("  \u{2502} {}", think_line),
                                Style::default()
                                    .fg(theme.thinking_text)
                                    .add_modifier(Modifier::ITALIC),
                            )));
                        }
                        lines.push(Line::from(Span::styled(
                            format!("  \u{2514}\u{2500}\u{2500}"),
                            Style::default().fg(theme.thinking_border),
                        )));
                        thinking_content.clear();
                    }
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", app.active_model),
                            Style::default().fg(theme.assistant_text),
                        ),
                        Span::styled(before, Style::default().fg(theme.fg)),
                    ]));
                }
                remaining = &remaining[think_start + "<think>".len()..];
                in_thinking = true;

                if let Some(think_end) = remaining.find("</think>") {
                    thinking_content.push_str(&remaining[..think_end]);
                    remaining = &remaining[think_end + "</think>".len()..];
                    for think_line in thinking_content.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  \u{2502} {}", think_line),
                            Style::default()
                                .fg(theme.thinking_text)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                    lines.push(Line::from(Span::styled(
                        format!("  \u{2514}\u{2500}\u{2500}"),
                        Style::default().fg(theme.thinking_border),
                    )));
                    thinking_content.clear();
                    in_thinking = false;
                }
            } else {
                if in_thinking {
                    thinking_content.push_str(remaining);
                    for think_line in thinking_content.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  \u{2502} {}", think_line),
                            Style::default()
                                .fg(theme.thinking_text)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", app.active_model),
                            Style::default().fg(theme.assistant_text),
                        ),
                        Span::styled(remaining, Style::default().fg(theme.fg)),
                    ]));
                }
                remaining = "";
            }
        }

        if in_thinking && !thinking_content.is_empty() {
            lines.insert(
                lines
                    .len()
                    .saturating_sub(if lines.last().map(|l| l.width()) == Some(0) {
                        1
                    } else {
                        0
                    }),
                Line::from(Span::styled(
                    format!("{} {} \u{2500}", THINKING_HEADER, COLLAPSE_ICON),
                    Style::default()
                        .fg(theme.thinking_header)
                        .add_modifier(Modifier::BOLD),
                )),
            );
        }
    } else if !app.streaming_buffer.is_empty() {
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
                    remaining = &remaining[think_end + "</think>".len()..];
                }
            } else {
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

    let content_width = area.width as usize;
    let visible_height = area.height.saturating_sub(2) as usize;
    let total_visual_lines: usize = lines
        .iter()
        .map(|line| {
            let w = line.width();
            if w == 0 || content_width == 0 {
                1
            } else {
                w.div_ceil(content_width)
            }
        })
        .sum();
    let scroll = if total_visual_lines > visible_height {
        (total_visual_lines - visible_height) as u16
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
