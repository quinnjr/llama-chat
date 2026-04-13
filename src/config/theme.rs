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
    pub thinking_header: Color,
    pub thinking_text: Color,
    pub thinking_border: Color,
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
            code_bg: Color::Rgb(22, 28, 40),
            border: Color::Rgb(65, 75, 92),
            muted: Color::Rgb(120, 128, 142),
            bg: Color::Rgb(30, 38, 55),
            fg: Color::Rgb(229, 231, 235),
            thinking_header: Color::Rgb(251, 191, 36),
            thinking_text: Color::Rgb(180, 180, 180),
            thinking_border: Color::Rgb(251, 191, 36),
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
            thinking_header: Color::Rgb(217, 119, 6),
            thinking_text: Color::Rgb(87, 87, 87),
            thinking_border: Color::Rgb(217, 119, 6),
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
                    "thinking_header" => theme.thinking_header = color,
                    "thinking_text" => theme.thinking_text = color,
                    "thinking_border" => theme.thinking_border = color,
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

    #[test]
    fn dark_theme_expected_colors() {
        let theme = Theme::dark();
        assert_eq!(theme.accent, Color::Rgb(129, 140, 248));
        assert_eq!(theme.bg, Color::Rgb(30, 38, 55));
        assert_eq!(theme.fg, Color::Rgb(229, 231, 235));
        assert_eq!(theme.tool_ok, Color::Rgb(52, 211, 153));
        assert_eq!(theme.tool_denied, Color::Rgb(248, 113, 113));
        assert_eq!(theme.thinking_header, Color::Rgb(251, 191, 36));
        assert_eq!(theme.thinking_text, Color::Rgb(180, 180, 180));
        assert_eq!(theme.thinking_border, Color::Rgb(251, 191, 36));
    }

    #[test]
    fn light_theme_expected_colors() {
        let theme = Theme::light();
        assert_eq!(theme.accent, Color::Rgb(79, 70, 229));
        assert_eq!(theme.bg, Color::Rgb(250, 250, 250));
        assert_eq!(theme.fg, Color::Rgb(31, 41, 55));
        assert_eq!(theme.tool_ok, Color::Rgb(5, 150, 105));
        assert_eq!(theme.tool_denied, Color::Rgb(220, 38, 38));
        assert_eq!(theme.thinking_header, Color::Rgb(217, 119, 6));
        assert_eq!(theme.thinking_text, Color::Rgb(87, 87, 87));
        assert_eq!(theme.thinking_border, Color::Rgb(217, 119, 6));
    }

    #[test]
    fn from_config_light_preset() {
        let theme = Theme::from_config("light", &HashMap::new());
        assert_eq!(theme.accent, Color::Rgb(79, 70, 229));
        assert_eq!(theme.bg, Color::Rgb(250, 250, 250));
    }

    #[test]
    fn from_config_unknown_preset_falls_back_to_dark() {
        let theme = Theme::from_config("nonexistent", &HashMap::new());
        assert_eq!(theme.accent, Color::Rgb(129, 140, 248));
    }

    #[test]
    fn unknown_color_key_in_overrides_is_ignored() {
        let mut overrides = HashMap::new();
        overrides.insert("nonexistent_key".into(), "#ff0000".into());
        let theme = Theme::from_config("dark", &overrides);
        // Should be unchanged from dark defaults
        assert_eq!(theme.accent, Color::Rgb(129, 140, 248));
    }

    #[test]
    fn invalid_hex_in_overrides_is_ignored() {
        let mut overrides = HashMap::new();
        overrides.insert("accent".into(), "not-hex".into());
        let theme = Theme::from_config("dark", &overrides);
        assert_eq!(theme.accent, Color::Rgb(129, 140, 248));
    }

    #[test]
    fn all_color_overrides() {
        let mut overrides = HashMap::new();
        overrides.insert("user_text".into(), "#aabbcc".into());
        overrides.insert("assistant_text".into(), "#112233".into());
        overrides.insert("tool_name".into(), "#445566".into());
        overrides.insert("tool_ok".into(), "#778899".into());
        overrides.insert("tool_denied".into(), "#aabb00".into());
        overrides.insert("code_bg".into(), "#001122".into());
        overrides.insert("border".into(), "#334455".into());
        overrides.insert("muted".into(), "#667788".into());
        overrides.insert("bg".into(), "#000000".into());
        overrides.insert("fg".into(), "#ffffff".into());
        let theme = Theme::from_config("dark", &overrides);
        assert_eq!(theme.user_text, Color::Rgb(0xaa, 0xbb, 0xcc));
        assert_eq!(theme.assistant_text, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.tool_name, Color::Rgb(0x44, 0x55, 0x66));
        assert_eq!(theme.tool_ok, Color::Rgb(0x77, 0x88, 0x99));
        assert_eq!(theme.tool_denied, Color::Rgb(0xaa, 0xbb, 0x00));
        assert_eq!(theme.code_bg, Color::Rgb(0x00, 0x11, 0x22));
        assert_eq!(theme.border, Color::Rgb(0x33, 0x44, 0x55));
        assert_eq!(theme.muted, Color::Rgb(0x66, 0x77, 0x88));
        assert_eq!(theme.bg, Color::Rgb(0, 0, 0));
        assert_eq!(theme.fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn parse_hex_without_hash() {
        assert_eq!(parse_hex("abcdef"), Some(Color::Rgb(0xab, 0xcd, 0xef)));
    }

    #[test]
    fn parse_hex_wrong_length() {
        assert_eq!(parse_hex("#abc"), None);
        assert_eq!(parse_hex("#abcdefab"), None);
    }
}
