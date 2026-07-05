pub struct SseParser {
    buf: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.find("\n\n") {
            let block: String = self.buf.drain(..pos + 2).collect();
            let data: Vec<&str> = block
                .lines()
                .filter_map(|l| {
                    l.strip_prefix("data:")
                        .map(|d| d.strip_prefix(' ').unwrap_or(d))
                })
                .collect();
            if !data.is_empty() {
                events.push(data.join("\n"));
            }
        }
        events
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_event() {
        let mut p = SseParser::new();
        let out = p.push("data: {\"progress\": 10}\n\n");
        assert_eq!(out, vec!["{\"progress\": 10}".to_string()]);
    }

    #[test]
    fn handles_chunk_splits_and_comments() {
        let mut p = SseParser::new();
        assert!(p.push("data: {\"a\":").is_empty());
        assert!(p.push(" 1}\n").is_empty());
        let out = p.push("\n: keepalive\n\ndata: {\"b\": 2}\n\n");
        assert_eq!(
            out,
            vec!["{\"a\": 1}".to_string(), "{\"b\": 2}".to_string()]
        );
    }

    #[test]
    fn joins_multiline_data() {
        let mut p = SseParser::new();
        let out = p.push("data: line1\ndata: line2\n\n");
        assert_eq!(out, vec!["line1\nline2".to_string()]);
    }
}
