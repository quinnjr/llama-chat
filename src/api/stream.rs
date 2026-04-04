use crate::api::types::ChatResponse;

pub fn parse_sse_line(line: &str) -> Option<ChatResponse> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return None;
    }
    let data = line.strip_prefix("data: ")?;
    if data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_content_delta() {
        let line = r#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":null}]}"#;
        let resp = parse_sse_line(line).unwrap();
        assert_eq!(
            resp.choices[0].delta.as_ref().unwrap().content.as_deref(),
            Some("Hi")
        );
    }

    #[test]
    fn parse_done_sentinel() {
        assert!(parse_sse_line("data: [DONE]").is_none());
    }

    #[test]
    fn parse_empty_and_comment() {
        assert!(parse_sse_line("").is_none());
        assert!(parse_sse_line(": keep-alive").is_none());
    }

    #[test]
    fn parse_malformed_json() {
        assert!(parse_sse_line("data: {not json}").is_none());
    }
}
