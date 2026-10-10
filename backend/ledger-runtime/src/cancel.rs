//! A request to stop what is running. Every clone of a token sees the same request.

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Debug, Clone)]
pub struct CancelToken {
    requested: Arc<watch::Sender<bool>>,
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            requested: Arc::new(watch::channel(false).0),
        }
    }

    pub fn cancel(&self) {
        self.requested.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.requested.borrow()
    }

    /// Completes when the token is cancelled, at once if it already was.
    pub async fn cancelled(&self) {
        let mut requested = self.requested.subscribe();
        // The sender lives as long as this token, so waiting cannot fail.
        let _ = requested.wait_for(|cancelled| *cancelled).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_cancelled_token_completes_at_once_for_every_clone() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());

        token.cancel();

        assert!(clone.is_cancelled());
        clone.cancelled().await;
        token.cancelled().await;
    }

    #[tokio::test]
    async fn waiting_ends_when_the_token_is_cancelled_later() {
        let token = CancelToken::new();
        let waiting = tokio::spawn({
            let token = token.clone();
            async move { token.cancelled().await }
        });

        token.cancel();

        waiting.await.unwrap();
    }
}
