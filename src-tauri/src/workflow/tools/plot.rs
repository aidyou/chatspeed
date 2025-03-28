use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::{
    http::chp::Chp,
    workflow::{
        error::WorkflowError,
        function_manager::{FunctionDefinition, FunctionResult, FunctionType},
    },
};

/// A function implementation for fetching data from a remote URL.
///
/// This function supports HTTP and HTTPS protocols, and can handle
/// various types of requests (GET, POST, etc.) with custom headers and body.
pub struct Plot {
    // chatspeed bot server url
    chp_server: String,
}

impl Plot {
    pub fn new(chp_server: String) -> Self {
        Self { chp_server }
    }
}

#[async_trait]
impl FunctionDefinition for Plot {
    /// Returns the name of the function.
    fn name(&self) -> &str {
        "plot"
    }

    /// Returns the type of the function.
    fn function_type(&self) -> FunctionType {
        FunctionType::CHP
    }

    /// Returns the description of the function.
    fn description(&self) -> &str {
        "Plots a graph based on the given data."
    }

    /// Returns the function calling specification in JSON format.
    ///
    /// This method provides detailed information about the function
    /// in a format compatible with function calling APIs.
    ///
    /// # Returns
    /// * `Value` - The function specification in JSON format.
    fn function_calling_spec(&self) -> Value {
        json!({
            "name": self.name(),
            "description": self.description(),
            "parameters": {
                "type": "object",
                "properties": {
                    "plot_type": {"type": "string","enum": ["line","bar","pie"], "description": "Plot type"},
                    "title": {"type": "string", "description": "Plot title"},
                    "x_label": {"type": "string", "description": "X-axis label"},
                    "y_label": {"type": "string", "description": "Y-axis label"},
                    "data": {"type": "object", "description": "Plot data","properties": {
                        "x": {"type": "array", "items": {"type": "number|string"}, "description": "X-axis data for line/bar"},
                        "y": {"type": "array", "items": {"type": "number"}, "description": "Y-axis data for line/bar"},
                        "values": {"type": "array", "items": {"type": "number"}, "description": "Values for pie"},
                        "labels": {"type": "array", "items": {"type": "string"}, "description": "Labels for pie"}
                    }}
                },
                "required": ["plot_type","data"]
            },
            "responses": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Plot image URL"
                    }
                },
                "description": "Generated plot result"
            }
        })
    }

    /// Executes the fetch function.
    ///
    /// # Arguments
    /// * `params` - The parameters to pass to the function, including the URL.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    async fn execute(&self, params: Value) -> FunctionResult {
        // Get the URL from the parameters
        let plot_type = params["plot_type"].as_str().ok_or_else(|| {
            WorkflowError::FunctionParamError("plot_type must be a string".to_string())
        })?;
        let title = params["title"].as_str().unwrap_or_default();
        let x_label = params["x_label"].as_str().unwrap_or_default();
        let y_label = params["y_label"].as_str().unwrap_or_default();

        // Check if the URL is empty
        if plot_type.is_empty() {
            return Err(WorkflowError::FunctionParamError(
                t!("workflow.chp_function_must_not_be_empty").to_string(),
            ));
        }

        let data = params["data"].as_object().ok_or_else(|| {
            WorkflowError::FunctionParamError("data must be an object".to_string())
        })?;
        if plot_type == "pie" {
            // verify values is present and not empty
            if data
                .get("values")
                .and_then(|v| v.as_array())
                .map_or(true, |arr| arr.is_empty())
            {
                return Err(WorkflowError::FunctionParamError(
                    "values must be a non-empty array for pie plot".to_string(),
                ));
            }
            // verify labels is present and not empty
            if data
                .get("labels")
                .and_then(|v| v.as_array())
                .map_or(true, |arr| arr.is_empty())
            {
                return Err(WorkflowError::FunctionParamError(
                    "labels must be a non-empty array for pie plot".to_string(),
                ));
            }
        } else if plot_type == "line" || plot_type == "bar" {
            // verify x is present and not empty
            if data
                .get("x")
                .and_then(|v| v.as_array())
                .map_or(true, |arr| arr.is_empty())
            {
                return Err(WorkflowError::FunctionParamError(
                    "x must be a non-empty array for line or bar plot".to_string(),
                ));
            }
            // verify y is present and not empty
            if data
                .get("y")
                .and_then(|v| v.as_array())
                .map_or(true, |arr| arr.is_empty())
            {
                return Err(WorkflowError::FunctionParamError(
                    "y must be a non-empty array for line or bar plot".to_string(),
                ));
            }
        }

        // Create a new crawler instance
        let chp = Chp::new(
            self.chp_server.clone(),
            Some("http://127.0.0.1:15158".to_string()),
        );

        // Plot the data from the URL
        let results = chp
            .call(
                "plot",
                "post",
                Some(json!({
                    "plot_type": plot_type,
                    "title": title,
                    "x_label": x_label,
                    "y_label": y_label,
                    "data": data,
                })),
                None,
            )
            .await
            .map_err(|e| WorkflowError::Execution(e.to_string()))?;

        if results.is_null() || results.get("url").is_none() {
            return Err(WorkflowError::Execution(
                "Failed to generate plot".to_string(),
            ));
        }

        // Return the results as JSON
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_plot() {
        let plot = Plot::new("http://127.0.0.1:12321".to_string());
        let result = plot
            .execute(json!({
                "plot_type": "line",
                "title": "Test Plot",
                "x_label": "X Axis",
                "y_label": "Y Axis",
                "data": {
                    "x": [1, 2, 3, 4, 5],
                    "y": [10, 20, 30, 40, 50]
                }
            }))
            .await;
        println!("{:?}", result);
        assert!(result.is_ok());
    }
}
