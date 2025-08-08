use rust_i18n::t;
use std::fmt;

use crate::workflow::error::WorkflowError;

/// Model name, it's used to identify the model type.
/// The reasoning model is used for planning and analysis,
/// and the general model is used for text processing or general task.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModelName {
    Reasoning,
    General,
}

impl fmt::Display for ModelName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelName::Reasoning => write!(f, "reasoning"),
            ModelName::General => write!(f, "general"),
        }
    }
}

impl AsRef<str> for ModelName {
    fn as_ref(&self) -> &str {
        match self {
            ModelName::Reasoning => "reasoning",
            ModelName::General => "general",
        }
    }
}

impl TryFrom<&str> for ModelName {
    type Error = WorkflowError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "reasoning" => Ok(ModelName::Reasoning),
            "general" => Ok(ModelName::General),
            _ => Err(WorkflowError::Initialization(
                t!("tools.invalid_model_name", model_name = value).to_string(),
            )),
        }
    }
}
