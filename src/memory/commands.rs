//! Slash-command handlers for /remember, /forget, /memory.

use crate::memory::types::{Kind, MemoryError, Scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Remember { content: String, scope: Scope, kind: Kind },
    RememberThis { scope: Scope, kind: Kind },
    Forget { id: i64, scope: Scope },
    List { scope: Option<Scope> },
    Reindex,
    Accept,
    Disable,
}

/// Parse a full command line, e.g. "/remember --global --kind=feedback foo".
/// Returns None if the command is not ours (the app keeps dispatching).
pub fn parse(line: &str) -> Option<Result<Command, String>> {
    let line = line.trim();
    let (cmd, rest) = match line.split_once(' ') {
        Some((c, r)) => (c, r.trim()),
        None => (line, ""),
    };
    match cmd {
        "/remember" => Some(parse_remember(rest, /*this=*/ false)),
        "/remember-this" => Some(parse_remember(rest, /*this=*/ true)),
        "/forget" => Some(parse_forget(rest)),
        "/memory" => Some(parse_memory_sub(rest)),
        _ => None,
    }
}

fn parse_remember(rest: &str, is_this: bool) -> Result<Command, String> {
    let mut scope = Scope::Project;
    let mut kind = Kind::Project;
    let mut content_parts: Vec<&str> = Vec::new();
    for tok in rest.split_whitespace() {
        if tok == "--global" { scope = Scope::Global; continue; }
        if tok == "--project" { scope = Scope::Project; continue; }
        if let Some(k) = tok.strip_prefix("--kind=") {
            kind = Kind::parse(k).ok_or_else(|| format!("unknown kind: {k}"))?;
            continue;
        }
        content_parts.push(tok);
    }
    let content = content_parts.join(" ");
    if is_this {
        Ok(Command::RememberThis { scope, kind })
    } else if content.is_empty() {
        Err("usage: /remember [--global] [--kind=K] <text>".into())
    } else {
        Ok(Command::Remember { content, scope, kind })
    }
}

fn parse_forget(rest: &str) -> Result<Command, String> {
    let mut scope = Scope::Project;
    let mut id: Option<i64> = None;
    for tok in rest.split_whitespace() {
        if tok == "--global" { scope = Scope::Global; continue; }
        if tok == "--project" { scope = Scope::Project; continue; }
        id = tok.parse().ok().or(id);
    }
    id.map(|id| Command::Forget { id, scope })
        .ok_or_else(|| "usage: /forget [--global] <id>".into())
}

fn parse_memory_sub(rest: &str) -> Result<Command, String> {
    let (sub, tail) = match rest.split_once(' ') {
        Some((s, t)) => (s, t.trim()), None => (rest, ""),
    };
    match sub {
        "list" => {
            let mut scope: Option<Scope> = None;
            for tok in tail.split_whitespace() {
                if let Some(v) = tok.strip_prefix("--scope=") {
                    scope = match v { "global" => Some(Scope::Global),
                                      "project" => Some(Scope::Project),
                                      _ => None };
                }
            }
            Ok(Command::List { scope })
        }
        "reindex" => Ok(Command::Reindex),
        "accept" => Ok(Command::Accept),
        "disable" => Ok(Command::Disable),
        _ => Err(format!("unknown: /memory {sub}")),
    }
}

/// Human-readable summary after a successful save.
pub fn save_ack(id: i64, scope: Scope, kind: Kind) -> String {
    format!("memory #{id} saved (scope={}, kind={})",
            match scope { Scope::Global => "global", Scope::Project => "project" },
            kind.as_str())
}

#[allow(dead_code)]
fn _error_to_msg(e: &MemoryError) -> String { format!("memory error: {e}") }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_non_memory_returns_none() {
        assert!(parse("/exit").is_none());
        assert!(parse("hello").is_none());
    }

    #[test]
    fn parse_remember_defaults() {
        let cmd = parse("/remember I prefer terse replies").unwrap().unwrap();
        match cmd {
            Command::Remember { content, scope, kind } => {
                assert_eq!(content, "I prefer terse replies");
                assert_eq!(scope, Scope::Project);
                assert_eq!(kind, Kind::Project);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_remember_global_feedback() {
        let cmd = parse("/remember --global --kind=feedback no emojis").unwrap().unwrap();
        match cmd {
            Command::Remember { scope, kind, content } => {
                assert_eq!(scope, Scope::Global);
                assert_eq!(kind, Kind::Feedback);
                assert_eq!(content, "no emojis");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_remember_empty_errors() {
        let err = parse("/remember").unwrap().unwrap_err();
        assert!(err.contains("usage"));
    }

    #[test]
    fn parse_forget_needs_id() {
        assert!(parse("/forget").unwrap().is_err());
        assert_eq!(
            parse("/forget 42").unwrap().unwrap(),
            Command::Forget { id: 42, scope: Scope::Project },
        );
        assert_eq!(
            parse("/forget --global 7").unwrap().unwrap(),
            Command::Forget { id: 7, scope: Scope::Global },
        );
    }

    #[test]
    fn parse_memory_subs() {
        assert_eq!(parse("/memory reindex").unwrap().unwrap(), Command::Reindex);
        assert_eq!(parse("/memory accept").unwrap().unwrap(), Command::Accept);
        assert_eq!(parse("/memory disable").unwrap().unwrap(), Command::Disable);
        match parse("/memory list --scope=global").unwrap().unwrap() {
            Command::List { scope } => assert_eq!(scope, Some(Scope::Global)),
            _ => panic!(),
        }
    }
}
