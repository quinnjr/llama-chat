use ratatui::style::Color;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub user_text: Color,
    pub assistant_text: Color,
    pub tool_name: Color,
    pub tool_ok: Color,
    pub tool_denied: Color,
    pub code_bg: Color,
    pub border: Color,
    pub muted: Color,
    pub bg: Color,
    pub fg: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            accent: Color::Rgb(129, 140, 248),
            user_text: Color::Rgb(156, 163, 175),
            assistant_text: Color::Rgb(129, 140, 248),
            tool_name: Color::Rgb(192, 132, 252),
            tool_ok: Color::Rgb(52, 211, 153),
            tool_denied: Color::Rgb(248, 113, 113),
            code_bg: Color::Rgb(13, 17, 23),
            border: Color::Rgb(55, 65, 81),
            muted: Color::Rgb(107, 114, 128),
            bg: Color::Rgb(17, 24, 39),
            fg: Color::Rgb(229, 231, 235),
        }
    }

    pub fn light() -> Self {
        Self {
            accent: Color::Rgb(79, 70, 229),
            user_text: Color::Rgb(107, 114, 128),
            assistant_text: Color::Rgb(79, 70, 229),
            tool_name: Color::Rgb(124, 58, 237),
            tool_ok: Color::Rgb(5, 150, 105),
            tool_denied: Color::Rgb(220, 38, 38),
            code_bg: Color::Rgb(249, 250, 251),
            border: Color::Rgb(209, 213, 219),
            muted: Color::Rgb(156, 163, 175),
            bg: Color::Rgb(250, 250, 250),
            fg: Color::Rgb(31, 41, 55),
        }
    }

    pub fn from_config(preset: &str, overrides: &HashMap<String, String>) -> Self {
        let mut theme = match preset {
            "light" => Self::light(),
            _ => Self::dark(),
        };
        for (key, hex) in overrides {
            if let Some(color) = parse_hex(hex) {
                match key.as_str() {
                    "accent" => theme.accent = color,
                    "user_text" => theme.user_text = color,
                    "assistant_text" => theme.assistant_text = color,
                    "tool_name" => theme.tool_name = color,
                    "tool_ok" => theme.tool_ok = color,
                    "tool_denied" => theme.tool_denied = color,
                    "code_bg" => theme.code_bg = color,
                    "border" => theme.border = color,
                    "muted" => theme.muted = color,
                    "bg" => theme.bg = color,
                    "fg" => theme.fg = color,
                    _ => {}
                }
            }
        }
        theme
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_hex() {
        assert_eq!(parse_hex("#ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Some(Color::Rgb(0, 255, 0)));
    }

    #[test]
    fn parse_invalid_hex() {
        assert_eq!(parse_hex("#xyz"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn overrides_apply_to_preset() {
        let mut overrides = HashMap::new();
        overrides.insert("accent".into(), "#ff0000".into());
        let theme = Theme::from_config("dark", &overrides);
        assert_eq!(theme.accent, Color::Rgb(255, 0, 0));
        assert_eq!(theme.tool_ok, Color::Rgb(52, 211, 153));
    }
}
