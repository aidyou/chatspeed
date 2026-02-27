use crate::tools::ToolError;
use serde_json::Value;

pub struct ObservationReinforcer;

impl ObservationReinforcer {
    /// Reinforces the tool result with heuristic hints to better guide the AI
    pub fn reinforce(tool_name: &str, result: &Result<Value, ToolError>) -> String {
        match result {
            Ok(val) => {
                let raw_res = serde_json::to_string(val).unwrap_or_default();
                if raw_res == "[]" || raw_res == "{}" || raw_res.is_empty() {
                    format!("Tool '{}' executed successfully but returned no data. <system-reminder>If you expected data, try adjusting your search terms or checking if the target exists.</system-reminder>", tool_name)
                } else if raw_res.len() > 5000 {
                    format!("[Result too long, truncated] {}\n<system-reminder>The output was truncated due to length. Use more specific search patterns or read the file in parts if needed.</system-reminder>", &raw_res[..5000])
                } else {
                    raw_res
                }
            }
            Err(err) => {
                let err_msg = err.to_string();
                match err {
                    ToolError::Security(_) => {
                        format!("Action failed: Permission denied. <system-reminder>The path is outside your authorized workspace. Please use 'list_dir' to verify valid paths or ask the user to add the directory to 'allowed_paths' in settings.</system-reminder> Error: {}", err_msg)
                    }
                    ToolError::IoError(_) => {
                        match tool_name {
                            "read_file" | "list_dir" | "edit_file" | "write_file" => {
                                format!("Action failed: I/O error. <system-reminder>Verify if the file path exists and is correct. Use 'list_dir' to explore the directory structure if you are unsure.</system-reminder> Error: {}", err_msg)
                            }
                            _ => format!("Action failed: {}. <system-reminder>Analyze the error and try a different approach if necessary.</system-reminder>", err_msg)
                        }
                    }
                    ToolError::InvalidParams(_) => {
                        format!("Action failed: Invalid parameters. <system-reminder>Check the tool's input schema and ensure all required fields are provided with correct types.</system-reminder> Error: {}", err_msg)
                    }
                    _ => {
                        match tool_name {
                            "bash" => {
                                format!("Shell command failed: {}. <system-reminder>Check your command syntax and ensure all required dependencies are installed in the user's environment.</system-reminder>", err_msg)
                            }
                            _ => format!("Tool '{}' failed: {}. <system-reminder>Analyze this error to decide your next step.</system-reminder>", tool_name, err_msg)
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_reinforce_success_empty() {
        let res = Ok(json!([]));
        let output = ObservationReinforcer::reinforce("grep", &res);
        assert!(output.contains("returned no data"));
        assert!(output.contains("<system-reminder>"));
    }

    #[test]
    fn test_reinforce_security() {
        let res = Err(ToolError::Security("Unauthorized access".to_string()));
        let output = ObservationReinforcer::reinforce("read_file", &res);
        assert!(output.contains("authorized workspace"));
        assert!(output.contains("<system-reminder>"));
    }

    #[test]
    fn test_reinforce_io_error() {
        let res = Err(ToolError::IoError("File not found".to_string()));
        let output = ObservationReinforcer::reinforce("list_dir", &res);
        assert!(output.contains("Verify if the file path exists"));
        assert!(output.contains("<system-reminder>"));
    }
}
