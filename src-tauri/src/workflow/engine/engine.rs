use serde_json::Value;
use std::sync::Arc;

use crate::ai::interaction::chat_completion::ChatState;
use crate::commands::chat::setup_chat_proxy;
use crate::db::AiModel;
use crate::db::MainStore;
use crate::workflow::error::WorkflowError;
use crate::workflow::function_manager::FunctionManager;
use crate::workflow::tools::chat_completion::ChatCompletion;
use crate::workflow::tools::fetch::Fetch;
use crate::workflow::tools::search::Search;
use crate::workflow::tools::search_dedup::SearchDedupTool;
use crate::workflow::WorkflowExecutor;
use crate::workflow::WorkflowGraph;
use crate::workflow::{context::Context, parser::WorkflowParser, types::WorkflowResult};

/// Workflow engine for managing and executing workflows
pub struct WorkflowEngine {
    main_store: Arc<std::sync::Mutex<MainStore>>,
    chat_state: Arc<ChatState>,
    /// Function manager for handling function operations
    function_manager: Arc<FunctionManager>,
    /// Execution context
    pub(crate) context: Arc<Context>,
}

impl WorkflowEngine {
    /// Create a new workflow engine
    pub async fn new(
        main_store: Arc<std::sync::Mutex<MainStore>>,
        chat_state: Arc<ChatState>,
        chatspeedbot_server: Option<String>,
    ) -> Result<Self, WorkflowError> {
        let function_manager = FunctionManager::new();

        // register Request tool
        // function_manager
        //     .register_function(Arc::new(Request::new()))
        //     .await?;

        // register chat completion tool
        let chat_completion = ChatCompletion::new(chat_state.clone());
        for model_type in ["reasoning", "general"] {
            let model = Self::get_model(&main_store, model_type)?;
            chat_completion.add_model(model_type, model).await;
        }
        function_manager
            .register_function(Arc::new(chat_completion))
            .await?;

        function_manager
            .register_function(Arc::new(SearchDedupTool))
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
            chat_state,
            main_store,
            function_manager: Arc::new(function_manager),
            context: Arc::new(Context::new()),
        })
    }

    /// Get an AI model by its type
    ///
    /// Retrieves an AI model by its type from the configuration store.
    ///
    /// # Arguments
    /// * `main_store` - The main store containing the configuration
    /// * `model_type` - The type of the AI model to retrieve
    ///
    /// # Returns
    /// * `Result<AiModel, WorkflowError>` - The AI model or an error
    fn get_model(
        main_store: &Arc<std::sync::Mutex<MainStore>>,
        model_type: &str,
    ) -> Result<AiModel, WorkflowError> {
        let mut change_proxy_type = false;
        let model_name = format!("workflow_{}_model", model_type);
        // get reasoning model
        let reasoning_model = main_store
            .lock()
            .map_err(|e| WorkflowError::Store(format!("Failed to lock main store: {}", e)))?
            .get_config(&model_name, Value::Null);
        if reasoning_model.is_null() {
            return Err(WorkflowError::Config(
                format!("Failed to get {} model", model_type).to_string(),
            ));
        }

        let model_id = reasoning_model["id"].as_i64().unwrap_or_default();
        if model_id < 1 {
            return Err(WorkflowError::Config(
                format!("model {} not found.", model_type).to_string(),
            ));
        }

        let mut ai_model = main_store
            .lock()
            .map_err(|e| WorkflowError::Store(format!("Failed to lock main store: {}", e)))?
            .config
            .get_ai_model_by_id(model_id)
            .map_err(|e| format!("model {} not found: {}", model_type, e))?;

        // 从配置中获取模型详细信息
        // 只在模型的代理类型与传入 metadata 的代理类型不同时才进行修改
        let model_proxy_type = if let Some(md) = ai_model.metadata.clone() {
            md.get("proxyType").map(|v| v.clone())
        } else {
            None
        };

        if let Some(md) = ai_model.metadata.as_mut() {
            if let Some(obj) = md.as_object_mut() {
                // 检查当前 metadata 中是否已有 proxyType
                let current_proxy_type = obj.get("proxyType").map(|v| v.clone());

                // 如果当前没有 proxyType 或者与模型的 proxyType 不同，才进行更新
                if current_proxy_type.is_none() || current_proxy_type != model_proxy_type {
                    obj.insert(
                        "proxyType".to_string(),
                        model_proxy_type.unwrap_or(Value::Null).clone(),
                    );
                    change_proxy_type = true;
                }
            }
        }

        if change_proxy_type {
            setup_chat_proxy(&main_store, &mut ai_model.metadata)?;
        }

        Ok(ai_model)
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

    /// Get the calling spec of all registered functions
    ///
    /// # Returns
    /// * `Result<String, WorkflowError>` - The calling spec of all registered functions
    pub async fn get_function_calling_spec(&self) -> Result<String, WorkflowError> {
        let mut specs = Vec::new();
        for function in self.function_manager.get_registered_functions().await {
            if let Ok(function) = self.function_manager.get_function(&function).await {
                specs.push(function.function_calling_spec());
            }
        }
        serde_json::to_string(&specs).map_err(|e| WorkflowError::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::libs::window_channels::WindowChannels;

    fn get_db_path() -> std::path::PathBuf {
        let db_path = {
            let dev_dir = &*crate::STORE_DIR.read();
            dev_dir.join("chatspeed.db")
        };
        db_path
    }

    #[tokio::test]
    async fn test_get_function_calling_spec() -> Result<(), WorkflowError> {
        let main_store =
            MainStore::new(get_db_path()).map_err(|e| WorkflowError::Store(e.to_string()))?;
        let engine = WorkflowEngine::new(
            Arc::new(std::sync::Mutex::new(main_store)),
            Arc::new(ChatState::new(Arc::new(WindowChannels::new()))),
            Some("http://localhost:12321".to_string()),
        )
        .await?;

        let calling_spec = engine.get_function_calling_spec().await?;
        log::debug!("Function calling spec: {}", calling_spec);
        assert!(!calling_spec.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_execute() -> Result<(), Box<dyn std::error::Error>> {
        let main_store =
            MainStore::new(get_db_path()).map_err(|e| WorkflowError::Store(e.to_string()))?;
        // 创建工作流引擎
        let engine = WorkflowEngine::new(
            Arc::new(std::sync::Mutex::new(main_store)),
            Arc::new(ChatState::new(Arc::new(WindowChannels::new()))),
            Some("http://localhost:12321".to_string()),
        )
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
                            "function": "search",
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
                                "function": "search",
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
                            "function": "fetch",
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

    /// 测试五粮液股票分析工作流的完整执行流程
    #[tokio::test]
    async fn test_wuliangye_analysis_workflow() -> Result<(), Box<dyn std::error::Error>> {
        // 1. 准备测试环境
        let chat_state = Arc::new(ChatState::new(Arc::new(WindowChannels::new())));

        // 2. 加载工作流配置
        let workflow_config = r#"[
            {
                "id": "data_collection",
                "parallel": true,
                "desc": "并行数据采集阶段",
                "nodes": [
                    {
                        "id": "fetch_financial_reports",
                        "desc": "获取最新财务报表",
                        "tool": {
                            "function": "fetch",
                            "param": {
                                "url": "https://api.wuliangye.com/financial-reports/latest"
                            }
                        }
                    },
                    {
                        "id": "search_news",
                        "desc": "聚合近期新闻舆情",
                        "tool": {
                            "function": "search",
                            "param": {
                                "provider": "baidu_news",
                                "kw": ["五粮液 财报", "五粮液 管理层变动"],
                                "number": 20,
                                "resolve_baidu_links": true
                            }
                        }
                    },
                    {
                        "id": "industry_analysis",
                        "desc": "白酒行业趋势分析",
                        "tool": {
                            "function": "search",
                            "param": {
                                "provider": "google",
                                "kw": "中国白酒行业 2024年趋势",
                                "number": 15
                            }
                        }
                    }
                ]
            },
            {
                "id": "data_processing",
                "parallel": false,
                "dependencies": ["data_collection"],
                "desc": "数据加工与分析阶段",
                "nodes": [
                    {
                        "id": "analyze_financials",
                        "desc": "财务指标分析",
                        "tool": {
                            "function": "chat_completion",
                            "param": {
                                "model": "gpt-4-turbo",
                                "messages": [
                                    {
                                        "role": "system",
                                        "content": "请分析以下财务报表数据：${fetch_financial_reports.content}"
                                    }
                                ],
                                "metadata": {
                                    "temperature": 0.2,
                                    "maxTokens": 2000
                                }
                            }
                        }
                    },
                    {
                        "id": "sentiment_analysis",
                        "desc": "新闻舆情分析",
                        "tool": {
                            "function": "chat_completion",
                            "param": {
                                "model": "gpt-4",
                                "messages": [
                                    {
                                        "role": "system",
                                        "content": "请对以下新闻摘要进行情感分析：${search_news.results}"
                                    }
                                ]
                            }
                        }
                    }
                ]
            },
            {
                "id": "generate_report",
                "dependencies": ["data_processing"],
                "desc": "生成最终投资报告",
                "tool": {
                    "function": "chat_completion",
                    "param": {
                        "model": "gpt-4-turbo",
                        "messages": [
                            {
                                "role": "system",
                                "content": "综合以下分析结果生成投资报告：\n财务分析：${analyze_financials.content}\n舆情分析：${sentiment_analysis.content}\n行业趋势：${industry_analysis.results}"
                            }
                        ],
                        "metadata": {
                            "temperature": 0.7,
                            "maxTokens": 4000
                        }
                    }
                }
            }
        ]"#;

        // 3. 创建并执行工作流引擎
        let main_store =
            MainStore::new(get_db_path()).map_err(|e| WorkflowError::Store(e.to_string()))?;
        let engine = WorkflowEngine::new(
            Arc::new(std::sync::Mutex::new(main_store)),
            chat_state,
            Some("http://localhost:12321".to_string()),
        )
        .await
        .map_err(|e| format!("Failed to create workflow engine: {}", e))?;

        let result = engine
            .execute(workflow_config)
            .await
            .map_err(|e| format!("Workflow execution failed: {}", e));

        // 4. 验证执行结果
        assert!(result.is_ok(), "Workflow execution should succeed");
        println!("Workflow execution result: {:#?}", result);

        Ok(())
    }
}
