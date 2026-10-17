use crossterm::cursor::SetCursorStyle;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub pretty_errors: bool,
    pub pretty_print: bool,
    pub show_timing: bool,
    pub max_output_lines: usize,
    pub max_output_chars: usize,
    pub cursor_insert: SetCursorStyle,
    pub cursor_normal: SetCursorStyle,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            pretty_errors: true,
            pretty_print: true,
            show_timing: false,
            max_output_lines: 50,
            max_output_chars: 3000,
            cursor_insert: SetCursorStyle::BlinkingBlock,
            cursor_normal: SetCursorStyle::SteadyBlock,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Config::default(),
        };
        let map = parse_ini(&content);
        Config {
            pretty_errors: bool_setting(&map, "pretty_errors", true),
            pretty_print: bool_setting(&map, "pretty_print", true),
            show_timing: bool_setting(&map, "show_timing", false),
            max_output_lines: usize_setting(&map, "max_output_lines", 50),
            max_output_chars: usize_setting(&map, "max_output_chars", 3000),
            cursor_insert: cursor_setting(&map, "cursor_insert", SetCursorStyle::BlinkingBlock),
            cursor_normal: cursor_setting(&map, "cursor_normal", SetCursorStyle::SteadyBlock),
        }
    }
}

impl Config {
    pub fn set(&mut self, key: &str, value: &str) -> std::result::Result<(), String> {
        let normalized = key.replace('-', "_");
        match normalized.as_str() {
            "pretty_errors" => self.pretty_errors = parse_bool(value)?,
            "pretty_print" => self.pretty_print = parse_bool(value)?,
            "show_timing" => self.show_timing = parse_bool(value)?,
            "max_output_lines" => {
                self.max_output_lines = value
                    .parse()
                    .map_err(|_| format!("expected a number, got '{value}'"))?
            }
            "max_output_chars" => {
                self.max_output_chars = value
                    .parse()
                    .map_err(|_| format!("expected a number, got '{value}'"))?
            }
            "cursor_insert" => self.cursor_insert = parse_cursor_style(value)?,
            "cursor_normal" => self.cursor_normal = parse_cursor_style(value)?,
            _ => return Err(format!("unknown setting '{key}'")),
        }
        Ok(())
    }

    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![
            ("pretty_errors", self.pretty_errors.to_string()),
            ("pretty_print", self.pretty_print.to_string()),
            ("show_timing", self.show_timing.to_string()),
            ("max_output_lines", self.max_output_lines.to_string()),
            ("max_output_chars", self.max_output_chars.to_string()),
            (
                "cursor_insert",
                cursor_style_name(self.cursor_insert).into(),
            ),
            (
                "cursor_normal",
                cursor_style_name(self.cursor_normal).into(),
            ),
        ]
    }
}

const CURSOR_STYLES: [(&str, SetCursorStyle); 7] = [
    ("default", SetCursorStyle::DefaultUserShape),
    ("block", SetCursorStyle::SteadyBlock),
    ("bar", SetCursorStyle::SteadyBar),
    ("underline", SetCursorStyle::SteadyUnderScore),
    ("blinking_block", SetCursorStyle::BlinkingBlock),
    ("blinking_bar", SetCursorStyle::BlinkingBar),
    ("blinking_underline", SetCursorStyle::BlinkingUnderScore),
];

fn cursor_style_name(v: SetCursorStyle) -> &'static str {
    CURSOR_STYLES
        .iter()
        .find(|(_, s)| *s == v)
        .map(|(name, _)| *name)
        .unwrap_or("default")
}

fn parse_bool(v: &str) -> std::result::Result<bool, String> {
    match v.to_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(format!("expected true/false, got '{v}'")),
    }
}

fn parse_cursor_style(v: &str) -> std::result::Result<SetCursorStyle, String> {
    let key = v.to_lowercase().replace('-', "_");
    CURSOR_STYLES
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, s)| *s)
        .ok_or_else(|| {
            let names: Vec<_> = CURSOR_STYLES.iter().map(|(name, _)| *name).collect();
            format!("expected one of {}, got '{v}'", names.join("/"))
        })
}

fn bool_setting(map: &HashMap<String, String>, key: &str, default: bool) -> bool {
    let dashed = key.replace('_', "-");
    map.get(key)
        .or_else(|| map.get(&dashed))
        .map(|v| matches!(v.as_str(), "true" | "yes" | "1" | "on"))
        .unwrap_or(default)
}

fn usize_setting(map: &HashMap<String, String>, key: &str, default: usize) -> usize {
    let dashed = key.replace('_', "-");
    map.get(key)
        .or_else(|| map.get(&dashed))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn cursor_setting(
    map: &HashMap<String, String>,
    key: &str,
    default: SetCursorStyle,
) -> SetCursorStyle {
    let dashed = key.replace('_', "-");
    map.get(key)
        .or_else(|| map.get(&dashed))
        .and_then(|v| parse_cursor_style(v).ok())
        .unwrap_or(default)
}

fn config_path() -> PathBuf {
    if let Ok(cfg_home) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(cfg_home).join("ghcitty/ghcitty")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".ghcitty")
    } else {
        PathBuf::from(".ghcitty")
    }
}

fn parse_ini(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = trimmed.split_once('=') {
            map.insert(key.trim().to_lowercase(), val.trim().to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ini_basic() {
        let content = "# comment\npretty_errors = false\n";
        let map = parse_ini(content);
        assert_eq!(map.get("pretty_errors").unwrap(), "false");
    }

    #[test]
    fn test_parse_ini_whitespace() {
        let content = "  pretty-errors  =  yes  \n";
        let map = parse_ini(content);
        assert_eq!(map.get("pretty-errors").unwrap(), "yes");
    }

    #[test]
    fn test_config_default() {
        let c = Config::default();
        assert!(c.pretty_errors);
    }

    #[test]
    fn test_set_bool_and_dashed() {
        let mut c = Config::default();
        c.set("pretty_print", "false").unwrap();
        assert!(!c.pretty_print);
        c.set("show-timing", "yes").unwrap();
        assert!(c.show_timing);
    }

    #[test]
    fn test_set_max_output_lines() {
        let mut c = Config::default();
        c.set("max_output_lines", "5").unwrap();
        assert_eq!(c.max_output_lines, 5);
        assert!(c.set("max_output_lines", "abc").is_err());
    }

    #[test]
    fn test_set_cursor_style() {
        let mut c = Config::default();

        c.set("cursor_insert", "default").unwrap();
        assert_eq!(c.cursor_insert, SetCursorStyle::DefaultUserShape);

        c.set("cursor-normal", "Blinking-Underline").unwrap();
        assert_eq!(c.cursor_normal, SetCursorStyle::BlinkingUnderScore);

        assert!(c.set("cursor_insert", "BlinkingBlock").is_err());
    }

    #[test]
    fn test_cursor_style_names_round_trip() {
        for (name, style) in CURSOR_STYLES {
            assert_eq!(cursor_style_name(style), name);
            assert_eq!(parse_cursor_style(name), Ok(style));
        }
    }

    #[test]
    fn test_set_unknown_key() {
        let mut c = Config::default();
        assert!(c.set("nope", "true").is_err());
    }
}
