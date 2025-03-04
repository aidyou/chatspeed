use std::sync::Arc;

use crate::workflow::error::WorkflowError;
use crate::workflow::function_manager::FunctionManager;
use crate::workflow::tools::fetch::Fetch;
use crate::workflow::tools::request::Request;
use crate::workflow::tools::search::Search;
use crate::workflow::WorkflowExecutor;
use crate::workflow::WorkflowGraph;
use crate::workflow::{context::Context, parser::WorkflowParser, types::WorkflowResult};

/// Workflow engine for managing and executing workflows
pub struct WorkflowEngine {
    /// Function manager for handling function operations
    function_manager: Arc<FunctionManager>,
    /// Execution context
    pub(crate) context: Arc<Context>,
}

impl WorkflowEngine {
    /// Create a new workflow engine
    pub async fn new(chatspeedbot_server: Option<String>) -> Result<Self, WorkflowError> {
        let function_manager = FunctionManager::new();

        // register Request tool
        function_manager
            .register_function(Arc::new(Request::new()))
            .await?;

        if let Some(server) = chatspeedbot_server {
            // register Search tool
            function_manager
                .register_function(Arc::new(Search::new(server.clone())))
                .await?;

            // register Fetch tool
            function_manager
                .register_function(Arc::new(Fetch::new(server.clone())))
                .await?;
        }

        Ok(Self {
            function_manager: Arc::new(function_manager),
            context: Arc::new(Context::new()),
        })
    }

    /// Execute the workflow
    pub async fn execute(&self, workflow_config: &str) -> WorkflowResult<()> {
        // Get workflow graph
        let (nodes, edges) = WorkflowParser::parse(workflow_config)?;
        let graph = WorkflowGraph::new(nodes, edges)?;

        // Create executor
        let mut executor = WorkflowExecutor::create(
            self.context.clone(),
            self.function_manager.clone(),
            4, // max_parallel
            Arc::new(graph),
        )?;

        // Execute workflow
        // 使用公共方法设置上下文，而不是直接访问私有字段
        executor.execute().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute() {
        // Create the log file
        let console_config = simplelog::ConfigBuilder::new()
            .set_target_level(log::LevelFilter::Off) // 关闭目标/模块路径显示
            .set_location_level(log::LevelFilter::Off) // 关闭文件位置显示
            .set_time_level(log::LevelFilter::Info)
            .build();
        simplelog::CombinedLogger::init(vec![simplelog::TermLogger::new(
            simplelog::LevelFilter::Debug,
            console_config,
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        )])
        .expect(&rust_i18n::t!("main.failed_to_initialize_logger"));

        let engine = WorkflowEngine::new(Some("http://localhost:12321".to_string()))
            .await
            .unwrap();

        // Test workflow execution
        let result = engine
            .execute(
                r#"[
                {
                    "group": "parallel_group_1",
                    "parallel": true,
                    "desc": "基础数据查询",
                    "nodes": [
                        {
                            "node": "query_finance",
                            "desc": "查询五粮液的信息",
                            "tool": {
                            "function": "Search",
                            "param": {
                                    "provider": "baidu_news",
                                    "kw": ["五粮液财报", "五粮液负面"],
                                    "number": 10
                                }
                            }
                        },
                        {
                            "node": "search_news",
                            "desc": "聚合近期新闻与舆情",
                            "tool": {
                            "function": "Search",
                            "param": {
                                    "provider": "google_news",
                                    "kw": "五粮液板块"
                                }
                            }
                        }
                    ]
                }
            ]"#,
            )
            .await
            .unwrap();
        println!("{:#?}", result);
    }
}
