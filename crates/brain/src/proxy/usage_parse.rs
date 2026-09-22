//! Token/cost accounting from upstream responses, both dialects, streamed or
//! not. OpenRouter adds `usage.cost` (USD) when asked; otherwise the cost is
//! estimated from the tier prices by the caller.

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    /// USD as reported by OpenRouter, if present.
    pub cost_usd: Option<f64>,
    /// Who actually served the request: OpenRouter's `provider`. The prompt
    /// cache lives in that backend, so a provider change mid-session means the
    /// whole prefix is paid again (docs/architecture/cache.md).
    pub provider: Option<String>,
    pub seen: bool,
}

impl Usage {
    /// Non-streaming JSON body of either dialect.
    pub fn from_body(dialect: Dialect, body: &Value) -> Usage {
        let mut u = Usage {
            provider: provider_of(body),
            ..Usage::default()
        };
        if let Some(usage) = body.get("usage") {
            u.absorb(dialect, usage);
        }
        u
    }

    /// Merges one `usage` object (OpenAI or Anthropic shape) into the accumulator.
    pub fn absorb(&mut self, dialect: Dialect, usage: &Value) {
        let g = |k: &str| usage.get(k).and_then(Value::as_i64);
        match dialect {
            Dialect::OpenAi => {
                if let Some(v) = g("prompt_tokens") {
                    self.input_tokens = v;
                }
                if let Some(v) = g("completion_tokens") {
                    self.output_tokens = v;
                }
                if let Some(v) = usage
                    .get("prompt_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(Value::as_i64)
                {
                    self.cache_read_tokens = v;
                }
                if let Some(v) = usage
                    .get("prompt_tokens_details")
                    .and_then(|d| d.get("cache_write_tokens"))
                    .and_then(Value::as_i64)
                {
                    self.cache_write_tokens = v;
                }
            }
            Dialect::Anthropic => {
                if let Some(v) = g("input_tokens") {
                    self.input_tokens = v;
                }
                if let Some(v) = g("output_tokens") {
                    self.output_tokens = v;
                }
                if let Some(v) = g("cache_read_input_tokens") {
                    self.cache_read_tokens = v;
                }
                if let Some(v) = g("cache_creation_input_tokens") {
                    self.cache_write_tokens = v;
                }
            }
        }
        if let Some(c) = usage.get("cost").and_then(Value::as_f64) {
            self.cost_usd = Some(c);
        }
        self.seen = true;
    }

    /// One SSE `data:` payload. OpenAI: the last chunk carries `usage`.
    /// Anthropic: `message_start` has input/cache tokens, `message_delta` the
    /// output tokens (and OpenRouter's cost, when present).
    pub fn absorb_sse_data(&mut self, dialect: Dialect, data: &str) {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if self.provider.is_none() {
            self.provider = provider_of(&v);
        }
        match dialect {
            Dialect::OpenAi => {
                if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                    self.absorb(dialect, u);
                }
            }
            Dialect::Anthropic => {
                if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                    self.absorb(dialect, u);
                }
                if let Some(u) = v.get("usage") {
                    // message_delta: Anthropic reports output_tokens here; OpenRouter
                    // reports the full usage (input, cache, cost) here and zeros in
                    // message_start, so absorb everything that is present.
                    self.absorb(dialect, u);
                }
            }
        }
    }

    /// Estimated USD from tier prices; cache reads at 0.25×, writes as input.
    pub fn estimate_cost(&self, input_usd_per_m: f64, output_usd_per_m: f64) -> f64 {
        let input = (self.input_tokens + self.cache_write_tokens) as f64
            + self.cache_read_tokens as f64 * 0.25;
        input / 1e6 * input_usd_per_m + self.output_tokens as f64 / 1e6 * output_usd_per_m
    }
}

/// OpenRouter names the serving backend in `provider`: at the top level of a
/// body or of an OpenAI chunk, and inside `message` in an Anthropic
/// `message_start`.
fn provider_of(v: &Value) -> Option<String> {
    let p = v
        .get("provider")
        .or_else(|| v.get("message").and_then(|m| m.get("provider")))
        .and_then(Value::as_str)?
        .trim();
    (!p.is_empty()).then(|| p.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    OpenAi,
    Anthropic,
}

impl Dialect {
    pub fn as_str(self) -> &'static str {
        match self {
            Dialect::OpenAi => "openai",
            Dialect::Anthropic => "anthropic",
        }
    }
}

/// Feeds raw SSE bytes line by line; keeps a partial line across chunks.
#[derive(Default)]
pub struct SseScanner {
    partial: Vec<u8>,
}

impl SseScanner {
    pub fn feed(&mut self, dialect: Dialect, chunk: &[u8], usage: &mut Usage) {
        self.partial.extend_from_slice(chunk);
        while let Some(pos) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(data) = line.trim_end().strip_prefix("data:") {
                usage.absorb_sse_data(dialect, data);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_body_and_stream_usage() {
        let body = json!({"usage": {"prompt_tokens": 100, "completion_tokens": 20, "prompt_tokens_details": {"cached_tokens": 60}, "cost": 0.0012}});
        let u = Usage::from_body(Dialect::OpenAi, &body);
        assert_eq!(
            (
                u.input_tokens,
                u.output_tokens,
                u.cache_read_tokens,
                u.cost_usd,
                u.seen
            ),
            (100, 20, 60, Some(0.0012), true)
        );
        let mut acc = Usage::default();
        let mut sc = SseScanner::default();
        sc.feed(Dialect::OpenAi, b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}],\"usage\":null}\n\ndata: {\"choices\":[],\"usa", &mut acc);
        assert!(!acc.seen, "partial line not parsed yet");
        sc.feed(Dialect::OpenAi, b"ge\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"cost\":0.00001}}\n\ndata: [DONE]\n", &mut acc);
        assert_eq!(
            (acc.input_tokens, acc.output_tokens, acc.cost_usd),
            (5, 2, Some(0.00001))
        );
    }

    #[test]
    fn anthropic_body_and_stream_usage() {
        let body = json!({"usage": {"input_tokens": 300, "output_tokens": 25, "cache_read_input_tokens": 200, "cache_creation_input_tokens": 50}});
        let u = Usage::from_body(Dialect::Anthropic, &body);
        assert_eq!(
            (
                u.input_tokens,
                u.cache_read_tokens,
                u.cache_write_tokens,
                u.output_tokens
            ),
            (300, 200, 50, 25)
        );
        let mut acc = Usage::default();
        let mut sc = SseScanner::default();
        let stream = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":40,\"cache_read_input_tokens\":30,\"output_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"x\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":17}}\n\n";
        sc.feed(Dialect::Anthropic, stream.as_bytes(), &mut acc);
        assert_eq!(
            (
                acc.input_tokens,
                acc.cache_read_tokens,
                acc.output_tokens,
                acc.seen
            ),
            (40, 30, 17, true)
        );
        assert!(acc.cost_usd.is_none());
        let est = acc.estimate_cost(0.04, 0.08);
        let expected = ((40.0 + 30.0 * 0.25) / 1e6) * 0.04 + 17.0 / 1e6 * 0.08;
        assert!((est - expected).abs() < 1e-15);
    }

    #[test]
    fn provider_is_read_from_the_body_and_from_the_stream_in_both_dialects() {
        let b = json!({"provider": "StreamLake", "usage": {"prompt_tokens": 1}});
        assert_eq!(
            Usage::from_body(Dialect::OpenAi, &b).provider.as_deref(),
            Some("StreamLake")
        );
        let b = json!({"provider": "Baidu", "usage": {"input_tokens": 1}});
        assert_eq!(
            Usage::from_body(Dialect::Anthropic, &b).provider.as_deref(),
            Some("Baidu")
        );
        assert!(
            Usage::from_body(Dialect::Anthropic, &json!({"provider": "  "}))
                .provider
                .is_none(),
            "blank is no provider"
        );

        // Anthropic stream: `message_start` carries it inside `message`
        let mut acc = Usage::default();
        SseScanner::default().feed(
            Dialect::Anthropic,
            b"data: {\"type\":\"message_start\",\"message\":{\"provider\":\"StreamLake\",\"usage\":{\"input_tokens\":3}}}\n",
            &mut acc,
        );
        assert_eq!(acc.provider.as_deref(), Some("StreamLake"));

        // OpenAI stream: every chunk carries it; a mid-stream change cannot happen,
        // the first one is the backend that served the whole answer
        let mut acc = Usage::default();
        SseScanner::default().feed(
            Dialect::OpenAi,
            b"data: {\"provider\":\"SiliconFlow\",\"choices\":[]}\ndata: {\"provider\":\"Novita\",\"choices\":[]}\n",
            &mut acc,
        );
        assert_eq!(acc.provider.as_deref(), Some("SiliconFlow"));
    }
}
