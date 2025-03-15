use log::debug;
use serde_json::{Number, Value};

use super::{context::Context, error::WorkflowError, types::WorkflowResult};

impl Context {
    /// Internal implementation, handle recursive async calls
    pub(crate) async fn resolve_params_inner(&self, params: Value) -> WorkflowResult<Value> {
        match params {
            Value::Object(map) => {
                let mut result = serde_json::Map::new();
                for (key, value) in map {
                    let resolved = Box::pin(self.resolve_params_inner(value)).await?;
                    debug!("Field {} resolved result: {}", key, resolved);
                    result.insert(key, resolved);
                }
                Ok(Value::Object(result))
            }
            Value::Array(arr) => {
                let mut result = Vec::new();
                for (index, item) in arr.iter().enumerate() {
                    let resolved = Box::pin(self.resolve_params_inner(item.clone())).await?;
                    debug!("Array element [{}] resolved result: {}", index, resolved);
                    result.push(resolved);
                }
                Ok(Value::Array(result))
            }
            Value::String(s) => {
                // Check if the string contains references
                if s.starts_with("${") && s.ends_with("}") {
                    // If the entire string is a reference, return the referenced value
                    // Remove ${ and } characters
                    let reference = &s[2..s.len() - 1];
                    debug!("Resolving reference: {}", reference);

                    // Split node ID and path
                    let parts: Vec<&str> = reference.splitn(2, '.').collect();
                    let node_id = parts[0];
                    let path = parts.get(1).copied();

                    debug!("Reference node ID: {}, path: {:?}", node_id, path);

                    // Get node output
                    return self.get_node_output_with_path(node_id, path).await;
                } else if s.contains("${") && s.contains("}") {
                    // For strings containing multiple references, use string operations to handle
                    let mut result = s.clone();
                    let mut start_pos = 0;

                    while let Some(start_idx) = result[start_pos..].find("${") {
                        let real_start = start_pos + start_idx;
                        if let Some(end_idx) = result[real_start..].find("}") {
                            let real_end = real_start + end_idx + 1;

                            // Extract reference content (excluding ${ and })
                            let reference = &result[real_start + 2..real_end - 1];

                            // Split node ID and path
                            let parts: Vec<&str> = reference.splitn(2, '.').collect();
                            if !parts.is_empty() {
                                let node_id = parts[0];
                                let path = parts.get(1).copied();

                                // Get referenced value
                                match self.get_node_output_with_path(node_id, path).await {
                                    Ok(node_output) => {
                                        // Convert value to string and replace
                                        let replacement = match node_output {
                                            Value::String(s) => s,
                                            Value::Null => "null".to_string(),
                                            Value::Bool(b) => b.to_string(),
                                            Value::Number(n) => n.to_string(),
                                            _ => node_output.to_string(),
                                        };

                                        // Replace reference
                                        result.replace_range(real_start..real_end, &replacement);

                                        // Update start position
                                        start_pos = real_start + replacement.len();
                                    }
                                    Err(e) => return Err(e),
                                }
                            } else {
                                // Reference format is incorrect, skip
                                start_pos = real_end;
                            }
                        } else {
                            // No matching closing bracket found, skip
                            break;
                        }
                    }

                    Ok(Value::String(result))
                } else {
                    Ok(Value::String(s))
                }
            }
            // Other types return directly
            _ => Ok(params),
        }
    }

    /// 解析条件表达式并返回布尔结果
    ///
    /// 支持的条件表达式格式：
    /// - 简单条件：${item.score > 0.5}, ${item.value == "test"}, ${item.enabled}
    /// - 复合条件：${item.score > 0.5 && item.value == "test"}
    /// - 复杂条件：${(item.score > 0.5 || item.value == "test") && item.enabled}
    ///
    /// 返回一个布尔值，表示条件是否满足
    pub async fn resolve_condition(&self, condition: &str) -> WorkflowResult<bool> {
        // 如果条件不是引用格式，尝试直接解析为布尔值
        if !condition.starts_with("${") || !condition.ends_with("}") {
            return Ok(condition.parse::<bool>().unwrap_or(false));
        }

        // 移除 ${ 和 } 字符
        let condition = &condition[2..condition.len() - 1];
        debug!("解析条件: {}", condition);

        // 解析复合条件表达式
        self.resolve_compound_condition(condition).await
    }

    /// 解析复合条件表达式，支持 && 和 || 运算符以及括号分组
    async fn resolve_compound_condition(&self, condition: &str) -> WorkflowResult<bool> {
        // 首先处理括号分组，找到最外层的括号对并递归处理
        let mut processed_condition = String::new();
        let mut i = 0;
        let chars: Vec<char> = condition.chars().collect();

        while i < chars.len() {
            if chars[i] == '(' {
                // 找到对应的右括号
                let mut bracket_count = 1;
                let mut j = i + 1;

                while j < chars.len() && bracket_count > 0 {
                    if chars[j] == '(' {
                        bracket_count += 1;
                    } else if chars[j] == ')' {
                        bracket_count -= 1;
                    }
                    j += 1;
                }

                if bracket_count == 0 {
                    // 找到匹配的括号，递归处理括号内的内容
                    let inner_content = &condition[i + 1..j - 1];
                    // 使用 Box::pin 处理递归调用
                    let inner_result =
                        Box::pin(self.resolve_compound_condition(inner_content)).await?;

                    // 将结果替换为字面量 true/false
                    processed_condition.push_str(if inner_result { "true" } else { "false" });

                    i = j; // 跳过已处理的括号部分
                    continue;
                } else {
                    return Err(WorkflowError::Config(format!("括号不匹配: {}", condition)));
                }
            } else {
                processed_condition.push(chars[i]);
            }

            i += 1;
        }

        // 处理逻辑运算符，先处理 && 再处理 ||
        // 先检查是否有逻辑运算符 &&
        if processed_condition.contains("&&") {
            let parts: Vec<&str> = processed_condition.split("&&").collect();
            // 所有条件都必须为真
            for part in parts {
                // 递归调用需要装箱
                let part_result = Box::pin(self.resolve_compound_condition(part.trim())).await?;
                if !part_result {
                    return Ok(false); // 短路计算
                }
            }
            return Ok(true);
        }

        // 然后检查是否有逻辑运算符 ||
        if processed_condition.contains("||") {
            let parts: Vec<&str> = processed_condition.split("||").collect();

            // 只要有一个条件为真即可
            for part in parts {
                // 递归调用需要装箱
                let part_result = Box::pin(self.resolve_compound_condition(part.trim())).await?;
                if part_result {
                    return Ok(true); // 短路计算
                }
            }
            return Ok(false);
        }

        // 如果已经处理为字面量 true/false
        if processed_condition == "true" {
            return Ok(true);
        } else if processed_condition == "false" {
            return Ok(false);
        }

        // 最终调用也需要装箱，避免无限大小的 Future
        Box::pin(self.resolve_simple_condition(&processed_condition)).await
    }

    /// 解析单一条件表达式，如 a > b 或 a==b（支持运算符两侧有无空格）
    async fn resolve_simple_condition(&self, condition: &str) -> WorkflowResult<bool> {
        let processed_condition = condition.trim();

        if let Some((op, left_expr, right_expr)) = self.find_operator_parts(processed_condition) {
            // 解析左值（统一返回Value类型）
            let left_val = if left_expr.starts_with("${") && left_expr.ends_with("}") {
                // 处理嵌套条件表达式
                let condition = &left_expr[2..left_expr.len() - 1];
                let bool_result = self.resolve_compound_condition(condition).await?;
                Value::Bool(bool_result)
            } else {
                // 处理普通变量路径
                self.resolve_params(Value::String(format!("${{{}}}", left_expr)))
                    .await?
            };

            // 解析右值（统一返回Value类型）
            let right_val = if right_expr.starts_with("${") && right_expr.ends_with("}") {
                // 处理嵌套条件表达式
                let condition = &right_expr[2..right_expr.len() - 1];
                let bool_result = self.resolve_compound_condition(condition).await?;
                Value::Bool(bool_result)
            } else {
                // 处理字面量或简单路径
                self.parse_right_value(&right_expr).await?
            };

            // 执行比较操作（更新后的compare_values函数）
            self.compare_values(&left_val, &right_val, op)
        } else {
            // 处理无操作符的纯路径解析
            let value = self
                .resolve_params(Value::String(format!("${{{}}}", processed_condition)))
                .await?;
            value
                .as_bool()
                .ok_or(WorkflowError::Config("非布尔类型值".into()))
        }
    }

    /// 查找条件表达式中的运算符并返回左右两部分
    fn find_operator_parts(&self, condition: &str) -> Option<(&'static str, String, String)> {
        // 调整运算符匹配顺序，确保多字符运算符优先
        let operators = [
            ("&&", "&&"),
            ("||", "||"), // 先处理逻辑运算符
            (">=", ">="),
            ("<=", "<="),
            ("!=", "!="),
            ("==", "=="), // 比较运算符
            (">", ">"),
            ("<", "<"), // 最后处理单字符
        ];

        // 优先尝试无空格匹配
        for &(op_symbol, op_type) in &operators {
            if let Some((left, right)) = self.split_operator(condition, op_symbol) {
                return Some((op_type, left, right));
            }
        }

        // 然后尝试带空格匹配（如 " > "）
        for &(op_symbol, op_type) in &operators {
            let spaced_op = format!(" {} ", op_type);
            if let Some((left, right)) = self.split_operator(condition, &spaced_op) {
                return Some((op_type, left, right));
            }
        }

        None
    }

    fn split_operator(&self, condition: &str, op: &str) -> Option<(String, String)> {
        let condition = condition.trim();

        // 特殊处理无空格情况（如 item.score>0.5）
        if let Some(pos) = condition.find(op) {
            // 确保运算符完整匹配
            if condition.get(pos..pos + op.len()) != Some(op) {
                return None;
            }

            let left = condition[..pos].trim();
            let right = condition[pos + op.len()..].trim();

            // 验证左侧有效性（以标识符字符结尾）
            if left
                .chars()
                .last()
                .map_or(false, |c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                // 验证右侧有效性（以值起始字符开头）
                if right.chars().next().map_or(false, |c| {
                    c.is_alphanumeric() || c == '"' || c == '\'' || c == '('
                }) {
                    return Some((left.to_string(), right.to_string()));
                }
            }
        }
        None
    }

    /// 根据运算符分割条件表达式
    fn split_by_operator(&self, condition: &str, op: &str) -> Option<(String, String)> {
        let condition = condition.trim();

        // 特殊处理属性访问后的运算符（如 item.score>0.5）
        if let Some(pos) = condition.find(op) {
            // 检查运算符是否在属性访问之后
            if pos > 0 {
                let before_op = &condition[..pos];
                let after_op = &condition[pos + op.len()..];

                // 允许属性访问后直接接运算符（如 item.score>0.5）
                if before_op.ends_with(|c: char| c.is_alphanumeric() || c == '_')
                    && after_op.starts_with(|c: char| {
                        c.is_alphanumeric() || c == '_' || c == '"' || c == '\''
                    })
                {
                    return Some((before_op.trim().to_string(), after_op.trim().to_string()));
                }
            }
        }

        None
    }

    /// 解析条件表达式右侧的值
    async fn parse_right_value(&self, right_str: &str) -> WorkflowResult<Value> {
        if right_str.starts_with("${") && right_str.ends_with("}") {
            // 如果是引用，解析引用的值
            self.resolve_params(Value::String(right_str.to_string()))
                .await
        } else {
            // 尝试解析为数字或布尔值，然后回退到字符串
            if let Ok(num) = right_str.parse::<f64>() {
                Ok(Value::Number(
                    Number::from_f64(num).unwrap_or(Number::from_f64(0.0).unwrap()),
                ))
            } else if right_str == "true" {
                Ok(Value::Bool(true))
            } else if right_str == "false" {
                Ok(Value::Bool(false))
            } else {
                // 如果是字符串字面量，移除引号
                if (right_str.starts_with('"') && right_str.ends_with('"'))
                    || (right_str.starts_with('\'') && right_str.ends_with('\''))
                {
                    Ok(Value::String(right_str[1..right_str.len() - 1].to_string()))
                } else {
                    Ok(Value::String(right_str.to_string()))
                }
            }
        }
    }

    /// 辅助函数，用于比较数值
    fn compare_values(&self, left: &Value, right: &Value, op: &str) -> WorkflowResult<bool> {
        match (left, right) {
            // 数值比较
            (Value::Number(a), Value::Number(b)) => {
                let a_num = a
                    .as_f64()
                    .ok_or(WorkflowError::Config("左值非数字".into()))?;
                let b_num = b
                    .as_f64()
                    .ok_or(WorkflowError::Config("右值非数字".into()))?;
                match op {
                    ">" => Ok(a_num > b_num),
                    ">=" => Ok(a_num >= b_num),
                    "<" => Ok(a_num < b_num),
                    "<=" => Ok(a_num <= b_num),
                    "==" => Ok(a_num == b_num),
                    "!=" => Ok(a_num != b_num),
                    _ => Err(WorkflowError::Config(format!("不支持的操作符: {}", op))),
                }
            }
            // 字符串比较
            (Value::String(a), Value::String(b)) => match op {
                "==" => Ok(a == b),
                "!=" => Ok(a != b),
                _ => Err(WorkflowError::Config(format!("字符串不支持 {} 操作符", op))),
            },
            // 布尔值比较
            (Value::Bool(a), Value::Bool(b)) => match op {
                "==" => Ok(a == b),
                "!=" => Ok(a != b),
                _ => Err(WorkflowError::Config(format!("布尔值不支持 {} 操作符", op))),
            },
            // 类型自动转换比较
            (Value::Bool(a), Value::Number(b)) => {
                let b_num = b.as_f64().unwrap_or(0.0);
                match op {
                    "==" => Ok((*a as u8 as f64) == b_num),
                    "!=" => Ok((*a as u8 as f64) != b_num),
                    _ => Err(WorkflowError::Config(format!(
                        "不支持的类型组合操作符: {}",
                        op
                    ))),
                }
            }
            (Value::Number(a), Value::Bool(b)) => {
                let a_num = a.as_f64().unwrap_or(0.0);
                match op {
                    "==" => Ok(a_num == *b as u8 as f64),
                    "!=" => Ok(a_num != *b as u8 as f64),
                    _ => Err(WorkflowError::Config(format!(
                        "不支持的类型组合操作符: {}",
                        op
                    ))),
                }
            }
            _ => Err(WorkflowError::Config(format!(
                "类型不匹配: {} {} {}",
                left, op, right
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::workflow::context::Context;

    #[tokio::test]
    async fn test_resolve_condition() {
        let context = Context::new();

        // 测试简单比较条件
        context
            .set_output("item".to_string(), json!({"score":0.6,"enabled":true}))
            .await
            .unwrap();
        assert!(context
            .resolve_condition("${item.score > 0.5}")
            .await
            .unwrap());
        assert!(!context
            .resolve_condition("${item.score < 0.5}")
            .await
            .unwrap());
        assert!(context
            .resolve_condition("${item.score ==0.6}")
            .await
            .unwrap());

        // 测试复合条件
        context
            .set("item.enabled".to_string(), json!(true))
            .await
            .unwrap();
        assert!(context
            .resolve_condition("${item.score > 0.5&&item.enabled}")
            .await
            .unwrap());
        assert!(!context
            .resolve_condition("${item.score < 0.5 ||item.enabled == false}")
            .await
            .unwrap());

        // 测试嵌套括号的条件
        assert!(context
            .resolve_condition("${(item.score > 0.5|| item.score < 0.1) && item.enabled}")
            .await
            .unwrap());

        // 测试布尔值条件
        assert!(context.resolve_condition("${item.enabled}").await.unwrap());
        assert!(!context
            .resolve_condition("${item.enabled==false}")
            .await
            .unwrap());

        // 测试字符串和数字的混合比较
        context
            .set_output("item".to_string(), json!({"name":"test","score":0.6}))
            .await
            .unwrap();
        assert!(context
            .resolve_condition("${item.name == \"test\" && item.score > 0.5}")
            .await
            .unwrap());

        // 测试没有空格的条件解析
        assert!(context
            .resolve_condition("${item.name==\"test\"}")
            .await
            .unwrap());
        assert!(context
            .resolve_condition("${item.score>0.5}")
            .await
            .unwrap());
        assert!(context
            .resolve_condition("${item.score>=0.6}")
            .await
            .unwrap());
        assert!(!context
            .resolve_condition("${item.score<0.5}")
            .await
            .unwrap());
        assert!(!context
            .resolve_condition("${item.name!=\"test\"}")
            .await
            .unwrap());

        // 测试复合条件中包含没有空格的条件
        assert!(context
            .resolve_condition("${item.name==\"test\" && item.score>=0.6}")
            .await
            .unwrap());
    }
}
