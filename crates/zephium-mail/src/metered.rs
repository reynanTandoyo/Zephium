//! A byte budget for what a mail server sends. The IMAP client library has no
//! cap on a literal or on the length of a list, so a hostile or broken server
//! could make one command read without end. [`Metered`] wraps the connection,
//! counts every byte read and, once the budget armed for the current command
//! is spent, fails every further read. The session then reads as dead and the
//! engine drops it.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The shared counter of one connection.
#[derive(Debug)]
pub struct ReadBudget {
    used: AtomicU64,
    limit: AtomicU64,
    tripped: AtomicBool,
}

impl ReadBudget {
    /// A budget that allows `limit` bytes until it is armed again.
    #[must_use]
    pub fn new(limit: u64) -> Arc<Self> {
        Arc::new(Self {
            used: AtomicU64::new(0),
            limit: AtomicU64::new(limit),
            tripped: AtomicBool::new(false),
        })
    }

    /// Starts a command: counts from zero against `limit`. A tripped budget
    /// stays tripped, because the stream is no longer in step with the client.
    pub fn arm(&self, limit: u64) {
        self.used.store(0, Ordering::Relaxed);
        self.limit.store(limit, Ordering::Relaxed);
    }

    /// True once a read was refused for going over the budget.
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.tripped.load(Ordering::Relaxed)
    }

    /// Bytes read since the last [`arm`](Self::arm).
    #[must_use]
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }
}

/// Follows the server's bytes just far enough to see literal announcements
/// (`{123}` or `{123+}` at the end of a line) and the literal bytes that follow.
/// The client library allocates what a literal announces before the bytes
/// arrive, so an announcement larger than the command's budget is refused here,
/// before the client sees it.
#[derive(Debug, Default)]
struct LiteralScanner {
    remaining: u64,
    tail: Vec<u8>,
}

impl LiteralScanner {
    /// False when `bytes` announce a literal bigger than `limit`.
    fn feed(&mut self, mut bytes: &[u8], limit: u64) -> bool {
        while !bytes.is_empty() {
            if self.remaining > 0 {
                let skip =
                    usize::try_from(self.remaining).map_or(bytes.len(), |n| n.min(bytes.len()));
                self.remaining -= skip as u64;
                bytes = &bytes[skip..];
                continue;
            }
            let Some(end) = bytes.iter().position(|b| *b == b'\n') else {
                self.tail.extend_from_slice(bytes);
                let excess = self.tail.len().saturating_sub(32);
                self.tail.drain(..excess);
                return true;
            };
            self.tail.extend_from_slice(&bytes[..end]);
            bytes = &bytes[end + 1..];
            let announced = literal_size(&self.tail);
            self.tail.clear();
            if let Some(size) = announced {
                if size > limit {
                    return false;
                }
                self.remaining = size;
            }
        }
        true
    }
}

/// The size in a line that ends with `{n}` or `{n+}` (before CR), if any.
fn literal_size(line: &[u8]) -> Option<u64> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let line = line.strip_suffix(b"}")?;
    let line = line.strip_suffix(b"+").unwrap_or(line);
    let digits = line.iter().rev().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || line.len() <= digits || line[line.len() - digits - 1] != b'{' {
        return None;
    }
    // More than 19 digits cannot be a size we would accept either.
    std::str::from_utf8(&line[line.len() - digits..])
        .ok()?
        .parse()
        .ok()
        .or(Some(u64::MAX))
}

/// A stream whose reads are counted against a [`ReadBudget`]. Writes pass through.
#[derive(Debug)]
pub struct Metered<S> {
    inner: S,
    budget: Arc<ReadBudget>,
    scanner: LiteralScanner,
}

impl<S> Metered<S> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(inner: S, budget: Arc<ReadBudget>) -> Self {
        Self {
            inner,
            budget,
            scanner: LiteralScanner::default(),
        }
    }
}

fn over_budget() -> io::Error {
    // `InvalidData` reads as a protocol failure, not as a lost connection.
    io::Error::new(io::ErrorKind::InvalidData, "read budget exceeded")
}

impl<S: AsyncRead + Unpin> AsyncRead for Metered<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let budget = self.budget.clone();
        if budget.tripped() || budget.used() >= budget.limit.load(Ordering::Relaxed) {
            budget.tripped.store(true, Ordering::Relaxed);
            return Poll::Ready(Err(over_budget()));
        }
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if polled.is_ready() {
            let read = (buf.filled().len() - before) as u64;
            budget.used.fetch_add(read, Ordering::Relaxed);
            let limit = budget.limit.load(Ordering::Relaxed);
            let fresh = &buf.filled()[before..];
            if !self.scanner.feed(fresh, limit) {
                // Refuse before the client allocates for the announced literal.
                buf.set_filled(before);
                budget.tripped.store(true, Ordering::Relaxed);
                return Poll::Ready(Err(over_budget()));
            }
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Metered<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn reads_pass_until_the_budget_is_spent_and_then_fail_for_good() {
        let (client, mut server) = tokio::io::duplex(4096);
        let budget = ReadBudget::new(100);
        let mut metered = Metered::new(client, budget.clone());
        server.write_all(&[7u8; 300]).await.unwrap();
        let mut got = 0usize;
        let mut buf = [0u8; 64];
        let error = loop {
            match metered.read(&mut buf).await {
                Ok(n) => got += n,
                Err(error) => break error,
            }
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        // Over by less than one read buffer, never by the whole stream.
        assert!((100..100 + 64).contains(&got), "{got}");
        assert!(budget.tripped());
        // Arming again does not revive a stream that fell out of step.
        budget.arm(1000);
        assert!(metered.read(&mut buf).await.is_err());
    }

    #[tokio::test]
    async fn arming_resets_the_count_for_the_next_command() {
        let (client, mut server) = tokio::io::duplex(4096);
        let budget = ReadBudget::new(100);
        let mut metered = Metered::new(client, budget.clone());
        let mut buf = [0u8; 60];
        for _ in 0..5 {
            budget.arm(100);
            server.write_all(&[1u8; 60]).await.unwrap();
            metered.read_exact(&mut buf).await.unwrap();
            assert_eq!(budget.used(), 60);
        }
        assert!(!budget.tripped());
    }

    #[test]
    fn literal_announcements_are_found_across_chunks_and_literal_bytes_are_skipped() {
        let mut scan = LiteralScanner::default();
        // A line that looks like `{99999}` inside literal data is data, not an announcement.
        assert!(scan.feed(b"* 1 FETCH (BODY[] {5}\r\n", 100));
        assert!(scan.feed(b"{99999}\r\n)\r\n", 100));
        // An announcement split over chunks is still seen.
        assert!(scan.feed(b"* 2 FETCH (BODY[] {1", 100));
        assert!(!scan.feed(b"01}\r\n", 100));
        assert_eq!(literal_size(b"x {12+}\r"), Some(12));
        assert_eq!(literal_size(b"x {12}"), Some(12));
        assert_eq!(literal_size(b"x 12}"), None);
        assert_eq!(literal_size(b"{}"), None);
        assert_eq!(literal_size(b"{99999999999999999999999}\r"), Some(u64::MAX));
    }

    #[tokio::test]
    async fn a_huge_announced_literal_is_refused_before_any_of_it_is_read_on() {
        let (client, mut server) = tokio::io::duplex(4096);
        let budget = ReadBudget::new(1000);
        let mut metered = Metered::new(client, budget.clone());
        server
            .write_all(b"* 1 FETCH (BODY[] {536870912}\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 64];
        let error = metered.read(&mut buf).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(budget.tripped());
    }

    #[tokio::test]
    async fn writes_are_not_counted() {
        let (client, mut server) = tokio::io::duplex(4096);
        let budget = ReadBudget::new(10);
        let mut metered = Metered::new(client, budget.clone());
        metered.write_all(&[0u8; 500]).await.unwrap();
        let mut sink = [0u8; 500];
        server.read_exact(&mut sink).await.unwrap();
        assert_eq!(budget.used(), 0);
        assert!(!budget.tripped());
    }
}
