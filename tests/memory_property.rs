use llama_chat::memory as mem;

#[test]
fn chunker_losslessness() {
    // Property: concatenating chunk[0] with the non-overlapping tails of
    // subsequent chunks reproduces the original token sequence.
    let inputs = [
        "short one",
        "a b c d e f g h i j",
        &"word ".repeat(1200),
    ];
    for input in inputs {
        let chunks = mem_chunk_split(input);
        if chunks.is_empty() {
            assert!(input.trim().is_empty());
            continue;
        }
        let mut reconstructed: Vec<String> = chunks[0].text
            .split_whitespace().map(|s| s.to_string()).collect();
        for c in &chunks[1..] {
            let tail: Vec<&str> = c.text.split_whitespace().skip(50).collect();
            for t in tail { reconstructed.push(t.to_string()); }
        }
        let expected: Vec<String> = input.split_whitespace()
            .map(|s| s.to_string()).collect();
        assert_eq!(reconstructed, expected, "input: {:?}", &input[..input.len().min(40)]);
    }
}

fn mem_chunk_split(input: &str) -> Vec<mem::__test::ChunkSlice> {
    mem::__test::split_chunks(input)
}
