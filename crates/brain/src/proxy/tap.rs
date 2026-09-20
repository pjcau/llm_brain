//! A pass-through byte stream that watches the SSE lines for `usage` and
//! reports once, when the upstream stream ends or the client goes away.

use super::usage_parse::{Dialect, SseScanner, Usage};
use bytes::Bytes;
use futures_util::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

pub type Recorder = Box<dyn FnOnce(Usage, i64, bool) + Send + 'static>;

pub struct Tap<S> {
    inner: S,
    dialect: Dialect,
    scanner: SseScanner,
    usage: Usage,
    recorder: Option<Recorder>,
    started: Instant,
    completed: bool,
    /// First bytes of the stream, kept for the empty-reply diagnostic.
    head: Vec<u8>,
    /// Request shape for the diagnostic line (messages, last role, tools, sanitized fields).
    pub diag: String,
}

impl<S> Tap<S> {
    /// `recorder(usage, latency_ms, completed)` runs exactly once.
    pub fn new(inner: S, dialect: Dialect, started: Instant, recorder: Recorder) -> Self {
        Self {
            inner,
            dialect,
            scanner: SseScanner::default(),
            usage: Usage::default(),
            recorder: Some(recorder),
            started,
            completed: false,
            head: Vec::new(),
            diag: String::new(),
        }
    }

    fn finish(&mut self) {
        if self.completed && self.usage.seen && self.usage.output_tokens <= 1 {
            // Diagnostic for the "model answers nothing" failure seen with Claude Code:
            // what the upstream actually sent back, and what the request looked like.
            let head = String::from_utf8_lossy(&self.head);
            eprintln!(
                "empty reply [{}] upstream stream head: {}",
                self.diag,
                head.replace('\n', " ")
                    .chars()
                    .take(1500)
                    .collect::<String>()
            );
        }
        if let Some(r) = self.recorder.take() {
            r(
                std::mem::take(&mut self.usage),
                self.started.elapsed().as_millis() as i64,
                self.completed,
            );
        }
    }
}

impl<S, E> Stream for Tap<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    type Item = Result<Bytes, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                if this.head.len() < 4096 {
                    let take = (4096 - this.head.len()).min(chunk.len());
                    this.head.extend_from_slice(&chunk[..take]);
                }
                this.scanner.feed(this.dialect, &chunk, &mut this.usage);
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(e))) => {
                this.finish();
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                this.completed = true;
                this.finish();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> Drop for Tap<S> {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use std::sync::{Arc, Mutex};

    fn chunks(parts: &[&str]) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin {
        futures_util::stream::iter(
            parts
                .iter()
                .map(|p| Ok(Bytes::from(p.to_string())))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn records_usage_once_at_the_end_of_the_stream() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let tap = Tap::new(
            chunks(&[
                "data: {\"choices\":[]}\n\n",
                "data: {\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
                "data: [DONE]\n\n",
            ]),
            Dialect::OpenAi,
            Instant::now(),
            Box::new(move |u, _ms, completed| g.lock().unwrap().push((u, completed))),
        );
        let out: Vec<Bytes> = tap.map(|r| r.unwrap()).collect().await;
        assert_eq!(out.len(), 3, "bytes pass through unchanged");
        let rec = got.lock().unwrap();
        assert_eq!(rec.len(), 1);
        assert_eq!(
            (rec[0].0.input_tokens, rec[0].0.output_tokens, rec[0].1),
            (7, 3, true)
        );
    }

    #[tokio::test]
    async fn records_partial_usage_when_the_client_drops_early() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let mut tap = Tap::new(
            chunks(&[
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9}}}\n\n",
                "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\n",
            ]),
            Dialect::Anthropic,
            Instant::now(),
            Box::new(move |u, _ms, completed| g.lock().unwrap().push((u, completed))),
        );
        let _first = tap.next().await; // read one chunk, then drop the stream
        drop(tap);
        let rec = got.lock().unwrap();
        assert_eq!(rec.len(), 1);
        assert_eq!(
            (rec[0].0.input_tokens, rec[0].0.output_tokens, rec[0].1),
            (9, 0, false)
        );
    }
}
