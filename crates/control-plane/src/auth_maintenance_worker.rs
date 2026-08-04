//! Periodic removal of expired node-authentication state.
//!
//! Nonce reservations and pairing rows both expire on a clock rather than on a
//! request. Sweeping them here keeps the per-request authentication path free
//! of table-wide deletes, and stops unauthenticated pairing attempts from
//! accumulating rows indefinitely.

use piqae_storage_postgres::{PostgresStore, PurgedAuthState, StorageError};

#[derive(Clone, Debug)]
pub struct AuthMaintenanceWorker {
    store: PostgresStore,
}

impl AuthMaintenanceWorker {
    #[must_use]
    pub const fn new(store: PostgresStore) -> Self {
        Self { store }
    }

    /// Removes one batch of expired nonces and finished pairing rows.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the sweep cannot complete. Callers retry on
    /// the next tick; nothing is lost by a failed sweep beyond delayed cleanup.
    pub async fn run_once(&self) -> Result<PurgedAuthState, StorageError> {
        self.store.purge_expired_authentication_state().await
    }
}
