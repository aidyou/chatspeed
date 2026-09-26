//! Typed budget and effect admission domain (Phase 2B).
//!
//! This module is the canonical contract for the four-level experiment
//! budget ledger (`request -> trial -> candidate -> campaign`). It defines
//! the checked-integer resource vector, budget envelope (money-budget vs
//! token/resource-only mode), pricing snapshot, reservation state machine,
//! operation receipts and stable machine errors.
//!
//! Rules enforced here:
//! - All ledger counters are checked unsigned integers; money is expressed
//!   in integer micro units of a profile-declared local currency. Floating
//!   point values are never used as ledger counters.
//! - Omitting a dimension never means "unlimited": every dimension is either
//!   an explicit hard cap or explicitly `not_applicable`.
//! - Unknown effects are never treated as zero cost; the state machine keeps
//!   `unknown` reservations frozen until owner reconciliation.
//!
//! Persistence lives in `crate::db::budget` behind the `MainStore`/`DbRuntime`
//! single writer; this module stays transport- and storage-neutral.

pub mod errors;
pub mod pricing;
pub mod recovery;
pub mod resource;
pub mod types;

pub use errors::AdmissionError;
pub use types::{
    CommitReceipt, InfraReceipt, ReleaseReceipt, Reservation, ReserveEffect, UnknownReceipt,
};
