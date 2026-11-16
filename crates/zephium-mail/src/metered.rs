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

/// A stream whose reads are counted against a [`ReadBudget`]. Writes pass through.
#[derive(Debug)]
pub struct Metered<S> {
    inner: S,
    budget: Arc<ReadBudget>,
}

impl<S> Metered<S> {
    /// Wraps `inner`.
    #[must_use]
    pub fn new(inner: S, budget: Arc<ReadBudget>) -> Self {
        Self { inner, budget }
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
