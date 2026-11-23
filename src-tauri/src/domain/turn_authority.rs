//! Host-owned revocation shared by a turn and its already dispatched work.
//!
//! Persistence guards check this after acquiring their writer. Native effect
//! ports check it after their own serialized waits. No platform handle or
//! model-provided identity is needed at either boundary.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use super::types::AppError;

#[derive(Clone)]
pub struct TurnAuthority(Arc<AtomicBool>);

impl TurnAuthority {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub(crate) fn revoke(&self) {
        self.0.store(false, Ordering::Release);
    }

    pub(crate) fn check_live(&self) -> Result<(), AppError> {
        if self.0.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(AppError::new("turn_expired", "The bound turn was revoked."))
        }
    }
}

impl std::fmt::Debug for TurnAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnAuthority")
            .field("live", &self.0.load(Ordering::Acquire))
            .finish()
    }
}

impl PartialEq for TurnAuthority {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for TurnAuthority {}
