//! Rolling-window chunker used to slice assistant/user messages into
//! embeddable pieces. Approximates tokens by whitespace-separated words
//! — good enough for chunk boundaries, and avoids pulling in a BPE crate.

const WINDOW_TOKENS: usize = 500;
const OVERLAP_TOKENS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSlice {
    pub text: String,
    pub token_count: usize,
}

/// Split `input` into overlapping ~500-token windows. Inputs shorter than
/// one window produce a single chunk. Empty input produces no chunks.
pub fn split(input: &str) -> Vec<ChunkSlice> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    if tokens.len() <= WINDOW_TOKENS {
        return vec![ChunkSlice {
            text: input.to_string(),
            token_count: tokens.len(),
        }];
    }

    let mut out = Vec::new();
    let step = WINDOW_TOKENS - OVERLAP_TOKENS;
    let mut start = 0usize;
    while start < tokens.len() {
        let end = (start + WINDOW_TOKENS).min(tokens.len());
        let slice = &tokens[start..end];
        out.push(ChunkSlice {
            text: slice.join(" "),
            token_count: slice.len(),
        });
        if end == tokens.len() { break; }
        start += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_produces_nothing() {
        assert!(split("").is_empty());
        assert!(split("   \t\n").is_empty());
    }

    #[test]
    fn short_input_one_chunk() {
        let chunks = split("hello world this is short");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].token_count, 5);
    }

    #[test]
    fn exactly_window_one_chunk() {
        let text: String = (0..WINDOW_TOKENS).map(|i| format!("w{i} ")).collect();
        let chunks = split(text.trim());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].token_count, WINDOW_TOKENS);
    }

    #[test]
    fn longer_than_window_overlaps() {
        let text: String = (0..WINDOW_TOKENS + 100).map(|i| format!("w{i} ")).collect();
        let chunks = split(text.trim());
        assert!(chunks.len() >= 2);
        // Check overlap: last 50 tokens of chunk 0 == first 50 tokens of chunk 1
        let c0: Vec<&str> = chunks[0].text.split_whitespace().collect();
        let c1: Vec<&str> = chunks[1].text.split_whitespace().collect();
        let tail = &c0[c0.len() - OVERLAP_TOKENS..];
        let head = &c1[..OVERLAP_TOKENS];
        assert_eq!(tail, head);
    }

    #[test]
    fn utf8_safe() {
        let text = "αβγ δεζ ηθι κλμ";
        let chunks = split(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].token_count, 4);
    }
}
