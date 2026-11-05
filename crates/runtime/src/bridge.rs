//! Bridge from tokio cancellation to the synchronous cancel flag.

use core_types::Cancel;

use crate::scope::Scope;

/// Spawn a child task that sets `cancel` when the scope's token fires.
///
/// The task lives in `scope`, so it is cancelled and awaited with the job.
pub fn spawn_cancel_bridge(scope: &mut Scope, cancel: Cancel) {
    let token = scope.token();
    scope.spawn(async move {
        token.cancelled().await;
        cancel.cancel();
    });
}
