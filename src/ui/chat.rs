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

/// Format tool command for display — extract the most useful information
/// from tool call arguments rather than showing raw JSON.
fn format_tool_command(name: &str, raw: &str) -> String {
    // For shell commands, the command_display is already extracted by app.rs
    // For other tools, try to parse JSON and show key fields concisely
    match name {
        "write_file" | "read_file" | "edit_file" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw)
                && let Some(path) = v.get("path").and_then(|p| p.as_str())
            {
                if name == "write_file" {
                    let size = v
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map(|c| c.len())
                        .unwrap_or(0);
                    return format!("{path} ({size} bytes)");
                } else {
                    return path.to_string();
                }
            }
            truncate_display(raw, 120)
        }
        "list_files" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw)
                && let Some(path) = v.get("path").and_then(|p| p.as_str())
            {
                return path.to_string();
            }
            truncate_display(raw, 120)
        }
        "todo" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw)
                && let Some(items) = v.get("items").and_then(|i| i.as_array())
            {
                return format!("{} items", items.len());
            }
            truncate_display(raw, 120)
        }
        _ => truncate_display(raw, 120),
    }
}

fn truncate_display(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Parse streaming buffer into thinking blocks and assistant text.
/// Returns (thinking_parts: Vec<(content, is_closed)>, combined_text: String).
fn parse_streaming_think_tags(buffer: &str) -> (Vec<(String, bool)>, String) {
    let mut thinking_parts = Vec::new();
    let mut text = String::new();
    let mut remaining = buffer;

    while !remaining.is_empty() {
        if let Some(think_start) = remaining.find("<think>") {
            text.push_str(&remaining[..think_start]);
            remaining = &remaining[think_start + "<think>".len()..];

            if let Some(think_end) = remaining.find("</think>") {
                thinking_parts.push((remaining[..think_end].to_string(), true));
                remaining = &remaining[think_end + "</think>".len()..];
            } else {
                // Unclosed think tag — still streaming
                thinking_parts.push((remaining.to_string(), false));
                remaining = "";
            }
        } else {
            text.push_str(remaining);
            remaining = "";
        }
    }

    (thinking_parts, text)
}

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
                for (i, line) in text.lines().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled("you: ", Style::default().fg(theme.user_text)),
                            Span::styled(line, Style::default().fg(theme.fg)),
                        ]));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("      {line}"),
                            Style::default().fg(theme.fg),
                        )));
                    }
                }
                lines.push(Line::raw(""));
            }
            ChatEntry::Assistant(text) => {
                let prefix = format!("{}: ", app.active_model);
                let indent: String = " ".repeat(prefix.len());
                for (i, line) in text.lines().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(
                                prefix.clone(),
                                Style::default().fg(theme.assistant_text),
                            ),
                            Span::styled(line, Style::default().fg(theme.fg)),
                        ]));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("{indent}{line}"),
                            Style::default().fg(theme.fg),
                        )));
                    }
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
                // Show a concise version of the command — for shell tools
                // show just the command string, for file tools show path only,
                // and truncate very long argument strings.
                let display = format_tool_command(name, command);
                for cmd_line in display.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {cmd_line}"),
                        Style::default().fg(theme.muted),
                    )));
                }
            }
            ChatEntry::ToolOutput(text) => {
                let output_lines: Vec<&str> = text.lines().collect();
                let max_display = 30;
                let truncated = output_lines.len() > max_display;
                let display_lines = if truncated {
                    &output_lines[..max_display]
                } else {
                    &output_lines[..]
                };
                for output_line in display_lines {
                    lines.push(Line::from(Span::styled(
                        format!("  {output_line}"),
                        Style::default().fg(theme.muted),
                    )));
                }
                if truncated {
                    lines.push(Line::from(Span::styled(
                        format!("  ... ({} more lines)", output_lines.len() - max_display),
                        Style::default()
                            .fg(theme.muted)
                            .add_modifier(Modifier::ITALIC),
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
                    "  \u{2514}\u{2500}\u{2500}".to_string(),
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
            ChatEntry::SubagentOutput { index, text } => {
                for line in text.lines() {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("[agent-{index}] "),
                            Style::default().fg(theme.muted),
                        ),
                        Span::styled(line, Style::default().fg(theme.fg)),
                    ]));
                }
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
    if !app.streaming_buffer.is_empty() {
        let (thinking_parts, combined_text) = parse_streaming_think_tags(&app.streaming_buffer);

        // Render thinking blocks
        if app.show_thinking {
            for (content, closed) in &thinking_parts {
                lines.push(Line::from(Span::styled(
                    format!("{} {} \u{2500}", THINKING_HEADER, if *closed { COLLAPSE_ICON } else { "..." }),
                    Style::default()
                        .fg(theme.thinking_header)
                        .add_modifier(Modifier::BOLD),
                )));
                for think_line in content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  \u{2502} {}", think_line),
                        Style::default()
                            .fg(theme.thinking_text)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                if *closed {
                    lines.push(Line::from(Span::styled(
                        "  \u{2514}\u{2500}\u{2500}".to_string(),
                        Style::default().fg(theme.thinking_border),
                    )));
                }
            }
        }

        // Render assistant text (combined non-think content)
        if !combined_text.trim().is_empty() {
            let prefix = format!("{}: ", app.active_model);
            let indent: String = " ".repeat(prefix.len());
            for (i, line) in combined_text.lines().enumerate() {
                if i == 0 {
                    lines.push(Line::from(vec![
                        Span::styled(prefix.clone(), Style::default().fg(theme.assistant_text)),
                        Span::styled(line.to_string(), Style::default().fg(theme.fg)),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("{indent}{line}"),
                        Style::default().fg(theme.fg),
                    )));
                }
            }
        }
    }

    let visible_height = area.height.saturating_sub(2) as usize;

    let chat = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(theme.border)),
        )
        .wrap(Wrap { trim: false });

    let total_visual_lines = chat.line_count(area.width) as usize;
    let scroll = if total_visual_lines > visible_height {
        (total_visual_lines - visible_height) as u16
    } else {
        0
    };

    f.render_widget(chat.scroll((scroll, 0)), area);
}
