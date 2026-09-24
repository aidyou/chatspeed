//! Internal typed decision service, separate from chat protocol dispatch.
mod catalog;
mod system_one;
mod types;

pub(crate) use system_one::{evaluate, list_models};
pub(crate) use types::{Answer, DecisionRequest, Question};
