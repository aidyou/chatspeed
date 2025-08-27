use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::ToolManager;
use crate::workflow::{
    dag::{
        context::Context, executor::WorkflowExecutor, graph::WorkflowGraph, parser::WorkflowParser,
        types::WorkflowResult,
    },
    error::WorkflowError,
};

/// Workflow engine for managing and executing workflows
pub struct WorkflowEngine {
    // main_store: Arc<RwLock<MainStore>>,
    // chat_state: Arc<ChatState>,
    /// Function manager for handling function operations
    tool_manager: Arc<ToolManager>,
    /// Execution context
    pub(crate) context: Arc<Context>,
}

impl WorkflowEngine {
    /// Create a new workflow engine
    pub async fn new<R: Runtime>(app_handle: AppHandle<R>) -> Result<Self, WorkflowError> {
        let chat_state = app_handle.state::<Arc<ChatState>>().inner();

        Ok(Self {
            tool_manager: chat_state.tool_manager.clone(),
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
            self.tool_manager.clone(),
            4, // max_parallel
            Arc::new(graph),
        )?;

        // Execute workflow
        // 使用公共方法设置上下文，而不是直接访问私有字段
        executor.execute().await?;

        Ok(())
    }

    /// Get the calling spec of all registered functions
    ///
    /// # Returns
    /// * `Result<String, WorkflowError>` - The calling spec of all registered functions
    pub async fn get_function_calling_spec(
        &self,
    ) -> Result<Vec<MCPToolDeclaration>, WorkflowError> {
        Ok(self.tool_manager.get_tool_calling_spec(None).await?)
    }
}

#[cfg(test)]
mod tests {
    use crate::test::get_app_handle;

    use super::*;

    #[tokio::test]
    async fn test_get_function_calling_spec() -> Result<(), WorkflowError> {
        let app = get_app_handle();
        let engine = WorkflowEngine::new(app).await?;

        let calling_spec = engine.get_function_calling_spec().await?;
        log::debug!(
            "Function calling spec: {}",
            serde_json::to_string_pretty(&calling_spec).unwrap_or_default()
        );
        assert!(!calling_spec.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_execute() -> Result<(), Box<dyn std::error::Error>> {
        // 创建工作流引擎
        let app = get_app_handle();
        let engine = WorkflowEngine::new(app)
            .await
            .map_err(|e| format!("Failed to create workflow engine: {}", e))?;

        // 测试工作流执行
        let result = engine
            .execute(
                r#"[
                {
                    "id": "parallel_group_1",
                    "parallel": true,
                    "desc": "基础数据查询",
                    "nodes": [
                        {
                            "id": "query_finance",
                            "desc": "查询五粮液的信息",
                            "tool": {
                            "function": "web_search",
                            "param": {
                                    "provider": "baidu_news",
                                    "kw": ["五粮液财报", "五粮液负面"],
                                    "number": 10
                                },
                                "output": "news"
                            }
                        },
                        {
                            "id": "search_news",
                            "desc": "聚合近期新闻与舆情",
                            "tool": {
                                "function": "web_search",
                                "param": {
                                    "provider": "google_news",
                                    "kw": "五粮液板块",
                                    "number": 10
                                },
                                "output": "news"
                            }
                        }
                    ]
                },
                {
                    "id": "search_result_dedup",
                    "desc": "dedup news and analyse",
                    "dependencies": ["query_finance", "search_news"],
                    "tool": {
                        "function": "search_dedup",
                        "param": {
                            "results": "${news}",
                            "query": "五粮液财报"
                        }
                    }
                },
                {
                    "id": "news_fetch",
                    "desc": "fetch news details",
                    "dependencies": ["search_result_dedup"],
                    "loop": {
                        "input": "${search_result_dedup}",
                        "functions": [{
                            "function": "web_crawler",
                            "param": {
                                "url": "${item.url}"
                            }
                        }]
                    }
                }
            ]"#,
            )
            .await;

        println!("{:#?}", result);
        assert!(result.is_ok());
        assert!(engine
            .context
            .get_output("search_result_dedup")
            .await
            .is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_run_dag() -> Result<(), Box<dyn std::error::Error>> {
        // 2. 加载工作流配置
        let workflow_config = r#"[
        {
            "id": "news_group",
            "parallel": false,
            "desc": "获取最新新闻",
            "nodes": [
            {
                "id": "search_industry",
                "tool": {
                    "function": "web_search",
                    "param": {
                        "provider": "google",
                        "kw": "白酒行业发展趋势 政策影响 市场份额",
                        "time_period": "month",
                        "number": 15
                    },
                    "output": "industry_results"
                }
            },
            {
                "id": "industry_dedup",
                "dependencies": ["search_industry"],
                "tool": {
                    "function": "search_dedup",
                    "param": {
                        "query": "白酒行业发展趋势 政策影响 市场份额",
                        "results": "${industry_results}"
                    },
                    "output": "industry_dedup_results"
                }
            },
            {
                "id": "industry_relevant_search_result",
                "dependencies": ["industry_dedup"],
                "tool": {
                    "function": "chat_completion",
                    "param": {
                        "model_name": "general",
                        "messages": [
                            {
                                "role": "user",
                                "content": "搜索结果：\n```json\n${industry_dedup_results}\n```\n 请从上面的搜索结果中提取与“白酒行业发展趋势 政策影响 市场份额”最相关的前 5 个搜索结果。\n\n 注意：\n - 返回的数据必须保留原数据结构和对象：title,url,summary 等\n - 以json 格式返回，请不要输出无关的数据、说明和解释等避免破坏 json 格式"
                            }
                        ]
                    },
                    "output": "industry_relevant_results"
                }
            },
            {
                "id": "industry_content_fetch_loop",
                "dependencies": ["industry_relevant_search_result"],
                "loop": {
                    "input": "${industry_relevant_results.content}",
                    "functions": [
                        {
                            "function": "web_crawler",
                            "param": {
                                "url": "${item.url}",
                                "format": "markdown"
                            },
                            "output": "industry_content"
                        }
                    ]
                }
            }]
        },
        {
            "id": "finance_group",
            "parallel": false,
            "desc": "获取财务数据",
            "nodes": [{
                "id": "search_finance",
                "tool": {
                "function": "web_search",
                "param": {
                    "provider": "baidu",
                    "kw": "五粮液 2025年报 财务指标 资产负债表",
                    "number": 20
                },
                "output": "finance_results"
                }
            },
            {
                "id": "finance_dedup",
                "dependencies": ["search_finance"],
                "tool": {
                    "function": "search_dedup",
                    "param": {
                        "query": "五粮液 2025年报 财务指标 资产负债表",
                        "results": "${finance_results}"
                    },
                    "output": "finance_dedup_results"
                }
            },
            {
                "id": "finance_relevant_search_result",
                "dependencies": ["finance_dedup"],
                "tool": {
                    "function": "chat_completion",
                    "param": {
                        "model_name": "general",
                        "messages": [
                            {
                                "role": "user",
                                "content": "搜索结果：\n```json\n${finance_dedup_results}\n```\n 请从上面的搜索结果中提取与“五粮液 2025年报 财务指标 资产负债表”最相关的前 3 个搜索结果。\n\n 注意：\n - 返回的数据必须保留原数据结构和对象：title,url,summary 等\n - 以json 格式返回，请不要输出无关的数据、说明和解释等避免破坏 json 格式"
                            }
                        ]
                    },
                    "output": "finance_relevant_results"
                }
            },
            {
                "id": "finance_content_fetch_loop",
                "dependencies": ["finance_relevant_search_result"],
                "loop": {
                    "input": "${finance_relevant_results.content}",
                    "functions": [
                        {
                            "function": "web_crawler",
                            "param": {
                                "url": "${item.url}",
                                "format": "markdown"
                            },
                            "output": "finance_content"
                        }
                    ]
                }
            }]
        },
        {
            "id": "analysis_group",
            "parallel": true,
            "nodes": [
            {
                "id": "financial_analysis",
                "dependencies": ["finance_content_fetch_loop"],
                "tool": {
                    "function": "chat_completion",
                    "param": {
                        "model_name": "general",
                        "messages": [{
                            "role": "user",
                            "content": "分析以下财务数据，识别关键指标趋势：\n```json\n${finance_content}\n```"
                        }]
                    },
                    "output": "financial_report"
                }
            },
            {
                "id": "risk_evaluation",
                "dependencies": ["industry_content_fetch_loop"],
                "tool": {
                    "function": "chat_completion",
                    "param": {
                        "model_name": "general",
                        "messages": [
                        {
                            "role": "user",
                            "content": "评估行业政策风险：\n```json\n${industry_content}\n```"
                        }
                        ]
                    },
                    "output": "risk_report"
                }
            }
            ]
        },
        {
            "id": "final_report",
            "dependencies": ["analysis_group"],
            "tool": {
                "function": "chat_completion",
                "param": {
                    "model_name": "reasoning",
                    "messages": [{
                            "role": "user",
                            "content": "综合财务分析（${financial_report.content}）和风险报告（${risk_report.content}），给出投资建议"
                        }]
                },
                "output": "final_report"
            }
        }]
"#;

        let app = get_app_handle();
        let engine = WorkflowEngine::new(app)
            .await
            .map_err(|e| format!("Failed to create workflow engine: {}", e))?;

        let _ = engine
            .execute(workflow_config)
            .await
            .map_err(|e| format!("Workflow execution failed: {}", e))?;

        println!(
            "Workflow execution result: {:#?}",
            engine.context.get_output("final_report").await
        );
        Ok(())
    }
}
