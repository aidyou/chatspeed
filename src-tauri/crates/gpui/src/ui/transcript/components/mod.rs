//! Transcript-specific cards shared by the main and delegated timelines.

mod completion;
mod cost;
mod sub_agent;

pub(crate) use completion::{finish_task_card, snapshot_card};

pub(crate) use cost::cost_card;
pub(crate) use sub_agent::{child_loading, sub_agent_card, SubAgentDetails};
