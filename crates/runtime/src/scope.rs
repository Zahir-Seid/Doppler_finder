//! Structured task ownership.

use std::future::Future;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// A set of child tasks with one shared cancellation token.
///
/// `Scope` replaces detached `tokio::spawn`: a task started here is owned by
/// the scope, cancelled when the scope closes, and awaited before `close`
/// returns (SDD section 11.6).
#[derive(Debug)]
pub struct Scope {
    token: CancellationToken,
    tasks: JoinSet<()>,
}

impl Scope {
    /// Create a scope with its own token.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            tasks: JoinSet::new(),
        }
    }

    /// Create a scope whose token is cancelled with its parent's.
    pub fn child_of(parent: &CancellationToken) -> Self {
        Self {
            token: parent.child_token(),
            tasks: JoinSet::new(),
        }
    }

    /// Return a clone of the scope's cancellation token.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Spawn a child task owned by this scope.
    ///
    /// The returned join result is observed by [`Self::close`].
    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tasks.spawn(future);
    }

    /// Cancel every child and wait for all of them to finish.
    pub async fn close(mut self) {
        self.token.cancel();
        while self.tasks.join_next().await.is_some() {}
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}
