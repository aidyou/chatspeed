use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::workflow::context::Context;
use crate::workflow::error::WorkflowError;

/// 函数调用的结果类型
pub type FunctionResult = Result<Value, WorkflowError>;

/// 函数定义特性
#[async_trait]
pub trait FunctionDefinition: Send + Sync {
    /// 获取函数名称
    fn name(&self) -> &str;

    /// 获取函数类型
    fn function_type(&self) -> FunctionType;

    /// 获取函数描述
    fn description(&self) -> &str;

    /// 执行函数
    async fn execute(&self, params: Value, context: &Context) -> FunctionResult;
}

/// 函数类型枚举
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionType {
    Native, // Native function
    Http,   // HTTP protocol
    CHP,    //Chatspeed http protocol
}

impl fmt::Display for FunctionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FunctionType::Native => write!(f, "Native"),
            FunctionType::Http => write!(f, "Http"),
            FunctionType::CHP => write!(f, "CHP"),
        }
    }
}

/// 函数管理器
pub struct FunctionManager {
    /// 注册的函数映射
    functions: RwLock<HashMap<String, Arc<dyn FunctionDefinition>>>,
}

impl FunctionManager {
    /// 创建新的函数管理器
    pub fn new() -> Self {
        Self {
            functions: RwLock::new(HashMap::new()),
        }
    }

    /// 注册函数
    pub async fn register_function(
        &self,
        function: Arc<dyn FunctionDefinition>,
    ) -> Result<(), WorkflowError> {
        let name = function.name().to_string();
        let mut functions = self.functions.write().await;

        if functions.contains_key(&name) {
            return Err(WorkflowError::FunctionAlreadyExists(name));
        }

        functions.insert(name, function);
        Ok(())
    }

    /// 注销函数
    pub async fn unregister_function(&self, name: &str) -> Result<(), WorkflowError> {
        let mut functions = self.functions.write().await;

        if !functions.contains_key(name) {
            return Err(WorkflowError::FunctionNotFound(name.to_string()));
        }

        functions.remove(name);
        Ok(())
    }

    /// 获取函数
    pub async fn get_function(
        &self,
        name: &str,
    ) -> Result<Arc<dyn FunctionDefinition>, WorkflowError> {
        let functions = self.functions.read().await;

        functions
            .get(name)
            .cloned()
            .ok_or_else(|| WorkflowError::FunctionNotFound(name.to_string()))
    }

    /// 执行函数
    pub async fn execute_function(
        &self,
        name: &str,
        params: Value,
        context: &Context,
    ) -> FunctionResult {
        let function = self.get_function(name).await?;
        function.execute(params, context).await
    }

    /// 获取所有注册的函数名称
    pub async fn get_registered_functions(&self) -> Vec<String> {
        let functions = self.functions.read().await;
        functions.keys().cloned().collect()
    }

    /// 检查函数是否已注册
    pub async fn has_function(&self, name: &str) -> bool {
        let functions = self.functions.read().await;
        functions.contains_key(name)
    }
}

/// 默认实现的空函数管理器
impl Default for FunctionManager {
    fn default() -> Self {
        Self::new()
    }
}
