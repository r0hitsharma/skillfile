// Re-export all patch utilities from core so the rest of the cli crate
// can continue using `crate::patch::*` without breaking changes.
pub use skillfile_core::patch::*;
