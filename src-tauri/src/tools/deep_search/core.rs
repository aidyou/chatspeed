use async_trait::async_trait;
use chrono::Datelike;
use futures::future::join_all;
use json_value_merge::Merge;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

use super::prompt::{
    GENERATE_QUERIES_PROMPT, GENERATE_REPORT_PROMPT, GET_RELATED_RESULT_PROMPT, SUMMARIZE_PROMPT,
};
use crate::{
    ai::{
        interaction::chat_completion::{complete_chat_async, complete_chat_blocking, ChatState},
        traits::chat::{ChatCompletionResult, ChatResponse, MCPToolDeclaration, MessageType},
    },
    db::AiModel,
    http::chp::Chp,
    libs::{dedup::dedup_and_rank_results, util::format_json_str},
    search::{SearchProviderName, SearchResult},
    tools::{ModelName, NativeToolResult, ToolDefinition},
};

pub struct DeepSearch {
    chat_state: Arc<ChatState>,
    models: Arc<tokio::sync::RwLock<HashMap<ModelName, AiModel>>>,
    crawler_url: String,
    search_providers: Vec<SearchProviderName>,
    max_crawler_threads: usize,
    progress_callback: Arc<dyn Fn(Arc<ChatResponse>) + Send + Sync>,
    stop_flag: Arc<AtomicBool>,
}

impl DeepSearch {
    pub fn new(
        chat_state: Arc<ChatState>,
        max_crawler_threads: Option<usize>,
        crawler_url: String,
        search_providers: Vec<SearchProviderName>,
        progress_callback: Arc<dyn Fn(Arc<ChatResponse>) + Send + Sync>,
    ) -> Self {
        Self {
            chat_state,
            models: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            crawler_url,
            search_providers,
            max_crawler_threads: max_crawler_threads.unwrap_or(5),
            progress_callback,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Adds a new model to the models map.
    ///
    /// # Arguments
    /// * `name` - The name of the model
    /// * `model` - The model to add
    pub async fn add_model(&self, name: ModelName, model: AiModel) {
        self.models.write().await.insert(name, model);
    }

    /// Retrieves a model by name.
    ///
    /// # Arguments
    /// * `name` - The name of the model
    ///
    /// # Returns
    /// * `Option<AiModel>` - The model if found, otherwise `None`
    pub async fn get_model(&self, name: ModelName) -> Option<AiModel> {
        self.models.read().await.get(&name).cloned()
    }

    /// Gets the chat state for the DeepSearch instance.
    ///
    /// # Returns
    /// * `bool` - The stop flag for the DeepSearch instance.
    fn should_stop(&self) -> bool {
        self.stop_flag.load(Ordering::Acquire)
    }

    /// Checks the stop flag.
    ///
    /// # Returns
    /// * `Result<()>` - Ok if the flag is not set, otherwise an error message
    fn check_stop(&self) -> Result<(), String> {
        if self.stop_flag.load(Ordering::Acquire) {
            Err(t!("tools.deep_search.search_stopped_by_user").into())
        } else {
            Ok(())
        }
    }

    /// Gets the stop flag for the DeepSearch instance.
    ///
    /// # Returns
    /// * `Arc<AtomicBool>` - The stop flag for the DeepSearch instance.
    pub fn get_stop_flag(&self) -> Arc<AtomicBool> {
        self.stop_flag.clone()
    }

    /// Executes a deep search operation by generating multiple search queries from the original question,
    /// then executing search chains for each generated query.
    ///
    /// # Arguments
    /// * `chat_id` - The chat ID for tracking the search process
    /// * `question` - The original search question/topic
    /// * `metadata` - Optional metadata for the search
    /// * `search_engines` - Vector of search engine names to use
    /// * `max_results` - Maximum number of results to return per search query
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Combined vector of search results from all generated queries
    /// * `Err(String)` - Error message if the search fails
    pub async fn execute_deep_search(
        &self,
        chat_id: &str,
        question: &str,
        metadata: Option<Value>,
        max_results: i64,
    ) -> Result<(), String> {
        // 1. Generate search queries based on the question and conversation context
        let plans = self
            .generate_deeper_search_plans(chat_id, question, metadata.clone())
            .await?;

        self.check_stop()?;
        // 2. Execute search chain for each keyword
        let mut all_results = Vec::new();
        let mut current_plan_id: i32 = 0;
        for plan in plans.iter() {
            self.check_stop()?;

            self.send_plan_progress_message(
                chat_id,
                current_plan_id,
                plans.clone(),
                metadata.clone(),
            );
            match self
                .execute_search_chain(chat_id, &plan, metadata.clone(), max_results)
                .await
            {
                Ok(results) => {
                    all_results.extend(results);
                }
                Err(_) => {
                    if self.should_stop() {
                        return Err(t!("tools.deep_search.search_stopped_by_user").into());
                    }
                }
            }
            current_plan_id += 1;
        }
        self.send_plan_progress_message(chat_id, current_plan_id, plans.clone(), metadata.clone());

        self.check_stop()?;

        self.generate_report(chat_id, question, metadata, Arc::new(all_results))
            .await?;

        Ok(())
    }

    /// Send a search plan progress message to the chat.
    ///
    /// # Arguments
    /// * `chat_id` - The chat ID for tracking the search process
    /// * `current_plan_id` - The index of the current plan in the plan list
    /// * `plans` - The list of search plans to display
    /// * `metadata` - Optional metadata for the search
    fn send_plan_progress_message(
        &self,
        chat_id: &str,
        current_plan_id: i32,
        plans: Vec<String>,
        metadata: Option<Value>,
    ) {
        let mut i = 0;
        let mut content = vec![];
        for plan in plans {
            let status = if i < current_plan_id {
                "✔"
            } else if i == current_plan_id {
                "▷"
            } else {
                "□"
            };
            content.push(format!("{} {}", status, plan));
            i += 1;
        }
        self.send_process_message(
            chat_id,
            serde_json::to_string(&content).unwrap_or_default(),
            MessageType::Plan,
            metadata.clone(),
        );
    }

    /// Generates deeper search queries based on the original question and conversation context.
    ///
    /// This method:
    /// 1. Takes the original question and conversation messages
    /// 2. Formats a prompt with current/last year context
    /// 3. Sends to AI to generate refined search queries
    /// 4. Parses and returns the top 3 most relevant queries
    ///
    /// # Arguments
    /// * `question` - The original search question/topic
    /// * `metadata` - The chat metadata from fornt-end
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - Vector of generated search queries (max 3)
    /// * `Err(String)` - Error message if generation fails
    pub(crate) async fn generate_deeper_search_plans(
        &self,
        chat_id: &str,
        question: &str,
        metadata: Option<Value>,
    ) -> Result<Vec<String>, String> {
        // Send
        self.send_process_message(
            chat_id,
            t!("tools.deep_search.generating_query_plan").to_string(),
            MessageType::Step,
            metadata.clone(),
        );

        let current_year = chrono::Local::now().year();
        let last_year = current_year - 1;
        let prompt = GENERATE_QUERIES_PROMPT
            .replace("{{current_year}}", current_year.to_string().as_str())
            .replace("{{last_year}}", last_year.to_string().as_str())
            .replace("{{user_query}}", question);
        let messages: Vec<Value> = vec![json!({"role": "user", "content": prompt})];

        let chat_reslut = self
            .chat_with_retry(ModelName::Reasoning, messages, metadata.clone(), false)
            .await?;

        // Parse result into query list
        let json_str = format_json_str(&chat_reslut.content);
        let query_value: Value = serde_json::from_str(&json_str).unwrap_or_default();
        if let Some(error) = query_value["error"].as_str() {
            let err = t!(
                "tools.deep_search.failed_to_generate_queries",
                error = error
            )
            .to_string();
            self.send_process_message(chat_id, err.clone(), MessageType::Error, metadata.clone());
            return Err(err);
        }

        let queries: Vec<String> = query_value["plan"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        if !queries.is_empty() {
            self.send_process_message(
                chat_id,
                t!(
                    "tools.deep_search.query_plan_generated",
                    plan = queries.join("\n")
                )
                .to_string(),
                MessageType::Log,
                metadata.clone(),
            );
        } else {
            let err = t!("tools.deep_search.failed_to_generate_queries").to_string();
            self.send_process_message(chat_id, err.clone(), MessageType::Error, metadata.clone());
            return Err(err);
        }

        Ok(queries)
    }

    /// Generates a comprehensive report based on search results and sends it as a chat message.
    ///
    /// This method:
    /// 1. Filters and formats search results into a structured content
    /// 2. Sends the raw search results as a reference message
    /// 3. Generates a detailed report using AI reasoning capabilities
    /// 4. Handles retries in case of failures (max 3 attempts)
    ///
    /// # Arguments
    /// * `chat_id` - Unique identifier for the chat session
    /// * `question` - Original search query/topic
    /// * `metadata` - Optional JSON metadata for the chat (can include additional context)
    /// * `search_results` - Collection of search results to generate report from
    ///   - Must implement `IntoIterator` with `SearchResult` items
    ///   - Must be `Serialize` for JSON conversion
    ///   - Must be `Copy` to allow multiple iterations
    ///
    /// # Returns
    /// * `Ok(())` - If report generation and sending succeeded
    /// * `Err(String)` - Error message if failed after retries
    ///
    /// # Notes
    /// - Uses `ModelName::Reasoning` for report generation
    /// - Automatically retries up to 3 times with exponential backoff
    /// - Merges chat metadata with model metadata before sending
    async fn generate_report(
        &self,
        chat_id: &str,
        question: &str,
        metadata: Option<Value>,
        search_results: Arc<Vec<SearchResult>>,
    ) -> Result<(), String> {
        let mut content = vec![];
        let mut i = 1;
        let mut search_results = match Arc::try_unwrap(search_results) {
            Ok(results) => results,
            Err(arc) => (*arc).clone(),
        };

        for s in search_results.iter_mut() {
            if s.snippet.clone().unwrap_or_default().is_empty() {
                continue;
            }
            s.id = i;
            i += 1;
            content.push(format!(
                "[webpage {} begin]\n{}\n[webpage {} end]",
                s.id,
                s.snippet.clone().unwrap_or_default(),
                s.id
            ))
        }
        if content.is_empty() {
            return Ok(());
        }

        // send refence message
        self.send_process_message(
            chat_id,
            serde_json::to_string(&search_results).unwrap_or_default(),
            MessageType::Reference,
            metadata.clone(),
        );

        let prompt = GENERATE_REPORT_PROMPT
            .replace("{{question}}", question)
            .replace(
                "{{current_date}}",
                chrono::Local::now().format("%Y-%m-%d").to_string().as_str(),
            )
            .replace("{{search_results}}", &content.join("\n\n"));
        let model = self.get_model(ModelName::Reasoning).await.ok_or_else(|| {
            let err = t!(
                "tools.model_not_found",
                model_name = ModelName::Reasoning.to_string()
            )
            .to_string();
            log::error!("{}", err);
            err
        })?;

        let chat_metadata = self.merge_metadata(metadata, model.metadata.clone());

        for i in 0..3 {
            let progress_callback = Arc::clone(&self.progress_callback);
            let callback = move |response: Arc<ChatResponse>| {
                progress_callback(response);
            };
            match complete_chat_async(
                Some(self.chat_state.clone()),
                model.api_protocol.clone().try_into()?,
                Some(&model.base_url),
                model.default_model.clone(),
                Some(&model.api_key),
                chat_id.to_string(),
                vec![json!({"role": "user", "content": prompt})],
                None,
                chat_metadata.clone(),
                callback,
            )
            .await
            .map_err(|e| e.to_string())
            {
                Ok(_) => {
                    return Ok(());
                }
                Err(e) => {
                    if i == 2 {
                        return Err(e);
                    }
                    log::error!("Retrying chat with reasoning model, error: {}", e);
                    let delay = 2u64.pow(i as u32);
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                }
            }
        }
        Ok(())
    }

    /// Merges metadata from the chat and model.
    ///
    /// # Arguments
    /// * `metadata` - The chat metadata from fornt-end
    /// * `model_metadata` - The metadata from the model
    ///
    /// # Returns
    /// * `Option<Value>` - The merged metadata if found, otherwise `None`
    fn merge_metadata(
        &self,
        metadata: Option<Value>,
        model_metadata: Option<Value>,
    ) -> Option<Value> {
        if let Some(meta) = metadata {
            if let Some(cmd) = model_metadata {
                let mut merged = meta.clone();
                merged.merge(&cmd);
                Some(merged)
            } else {
                Some(meta)
            }
        } else {
            model_metadata.clone()
        }
    }

    /// Executes a deep search operation based on the provided question and search engines.
    ///
    /// This method:
    /// 1. Executes the search operation using the provided search engines
    /// 2. Dedups and ranks the results
    /// 3. Gets related results using AI
    /// 4. Crawls the data
    /// 5. Summarizes the results
    ///
    /// # Arguments
    /// * `chat_id` - The chat ID for tracking the search process
    /// * `question` - The search question/topic
    /// * `metadata` - The chat metadata from fornt-end
    /// * `max_results` - Maximum number of results to return
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - Vector of search results
    /// * `Err(String)` - Error message if the search fails
    pub async fn execute_search_chain(
        &self,
        chat_id: &str,
        question: &str,
        metadata: Option<Value>,
        max_results: i64,
    ) -> Result<Vec<SearchResult>, String> {
        // Step 1: Perform initial search
        self.check_stop()?;
        self.send_process_message(
            chat_id,
            t!("tools.deep_search.searching", keyword = question).to_string(),
            MessageType::Step,
            metadata.clone(),
        );
        let mut search_results = self
            .execute_multi_search(vec![question.to_string()], Some(max_results), None)
            .await?;
        if search_results.is_empty() {
            self.send_process_message(
                chat_id,
                t!("tools.deep_search.no_search_result", keyword = question).to_string(),
                MessageType::Log,
                metadata.clone(),
            );
            return Ok(vec![]);
        } else {
            self.send_process_message(
                chat_id,
                t!(
                    "tools.deep_search.search_success",
                    count = search_results.len(),
                    keyword = question,
                )
                .to_string(),
                MessageType::Log,
                metadata.clone(),
            )
        }

        // Step 2: dedup search
        self.check_stop()?;
        self.send_process_message(
            chat_id,
            t!("tools.deep_search.search_analysing", keyword = question).to_string(),
            MessageType::Step,
            metadata.clone(),
        );
        search_results = self.dedup_results(search_results, question);

        // Step 3: Rank results by ai
        self.check_stop()?;
        search_results = self
            .get_related_results(question, search_results, metadata.clone())
            .await?;
        if search_results.is_empty() {
            self.send_process_message(
                chat_id,
                t!("tools.deep_search.no_related_result", keyword = question).to_string(),
                MessageType::Log,
                metadata.clone(),
            );
            return Ok(vec![]);
        } else {
            self.send_process_message(
                chat_id,
                t!(
                    "tools.deep_search.related_result_found",
                    count = search_results.len(),
                    keyword = question,
                )
                .to_string(),
                MessageType::Log,
                metadata.clone(),
            );
        }

        // Step 4, crawl results
        self.send_process_message(
            chat_id,
            t!("tools.deep_search.crawling").to_string(),
            MessageType::Step,
            metadata.clone(),
        );
        match self
            .crawl_data(chat_id, search_results.clone(), metadata.clone())
            .await
        {
            Some(results) => {
                if results.is_empty() {
                    self.send_process_message(
                        chat_id,
                        t!(
                            "tools.deep_search.search_results_crawler_failed",
                            keyword = question
                        )
                        .to_string(),
                        MessageType::Log,
                        metadata.clone(),
                    );
                } else {
                    search_results = results;
                }
            }
            None => {
                self.send_process_message(
                    chat_id,
                    t!(
                        "tools.deep_search.search_results_crawler_failed",
                        keyword = question
                    )
                    .to_string(),
                    MessageType::Log,
                    metadata.clone(),
                );
                return Ok(search_results);
            }
        }

        // Step 5: Summarize results
        self.send_process_message(
            chat_id,
            t!("tools.deep_search.summarizing").to_string(),
            MessageType::Step,
            metadata.clone(),
        );
        search_results = self
            .summary(chat_id, question, search_results.clone(), metadata)
            .await?;

        Ok(search_results)
    }

    /// Execute concurrent searches across multiple search engines
    ///
    /// # Arguments
    /// * `kws` - the keywords
    /// * `number` - the number of results
    /// * `time_period` - the time period
    ///
    /// # Returns
    /// * `Ok(Vec<SearchResult>)` - The search results
    /// * `Err(String)` - The error message
    async fn execute_multi_search(
        &self,
        kws: Vec<String>,
        number: Option<i64>,
        time_period: Option<String>,
    ) -> Result<Vec<SearchResult>, String> {
        if self.crawler_url.is_empty() {
            return Err(t!("tools.deep_search.crawler_url_not_found").to_string());
        }

        // Create crawler instance
        let crawler = Chp::new(self.crawler_url.clone(), None);

        // Concurrently execute different search engines
        let mut handles = Vec::new();
        let number = number.unwrap_or(20);

        for provider in self.search_providers.clone() {
            self.check_stop()?;

            // Try to convert to SearchProviderName
            #[cfg(debug_assertions)]
            {
                log::debug!(
                    "Execute searching, provider: {}, keyword: {}",
                    &provider.to_string(),
                    kws.join(", ")
                );
            }

            let crawler_clone = crawler.clone();
            let keywords_clone = kws.clone(); // Use the cloned Vec<String>
            let stop_flag = self.stop_flag.clone();
            // Spawn async task
            let period = time_period.clone().map(|s| s.trim().to_string());

            let handle = tokio::spawn(async move {
                if stop_flag.load(Ordering::Acquire) {
                    return Ok::<Vec<SearchResult>, String>(vec![]);
                }

                // Get string slices from owned strings
                let keywords_refs: Vec<&str> = keywords_clone.iter().map(|s| s.as_str()).collect();
                let result = crawler_clone
                    .web_search(
                        provider.clone(),
                        &keywords_refs,
                        None,
                        Some(number),
                        period.as_deref(),
                        true,
                    )
                    .await
                    .map_err(|e| e.to_string());
                #[cfg(debug_assertions)]
                {
                    log::debug!(
                        "Searching finished, provider: {}, keyword: {}, result count: {}",
                        provider.to_string(),
                        keywords_clone.join(", "),
                        result.as_ref().map(|v| v.len()).unwrap_or(0)
                    )
                }

                result
            });

            handles.push(handle);
        }

        // If no valid search engines, return error
        if handles.is_empty() {
            return Err(t!("tools.deep_search.search_provider_not_found").to_string());
        }

        // Wait for all searches to complete and merge results
        let mut all_results = Vec::new();
        let results = futures::future::join_all(handles).await;

        for result in results {
            match result {
                Ok(Ok(items)) => all_results.extend(items),
                Ok(Err(e)) => eprintln!(
                    "{}",
                    t!("tools.deep_search.search_failed", error = e).to_string()
                ),
                Err(e) => eprintln!(
                    "{}",
                    t!("tools.deep_search.search_failed", error = e).to_string()
                ),
            }
        }

        Ok(all_results)
    }

    /// Dedup and rank results
    ///
    /// # Arguments
    /// * `results` - The search results
    /// * `query` - The search query
    ///
    /// # Returns
    /// * `Vec<SearchResult>` - The deduped and ranked results
    fn dedup_results(&self, results: Vec<SearchResult>, query: &str) -> Vec<SearchResult> {
        dedup_and_rank_results(results, query)
    }

    /// Filters and returns the most relevant search results for a given question
    ///
    /// This method:
    /// 1. Formats a prompt with requirements for filtering results
    /// 2. Sends the prompt to AI for processing
    /// 3. Parses and returns the filtered results
    ///
    /// # Arguments
    /// * `question` - The search topic/question
    /// * `search_results` - Raw search results to filter
    /// * `metadata` - The chat metadata from fornt-end
    ///
    /// # Returns
    /// Filtered results preserving original structure but:
    /// - Sorted by relevance
    /// - Excluding video/image sites
    /// - Containing at least title/url/summary
    ///
    /// # Errors
    /// Returns String error if:
    /// - AI processing fails
    /// - JSON parsing fails
    async fn get_related_results(
        &self,
        question: &str,
        search_results: Vec<SearchResult>,
        metadata: Option<Value>,
    ) -> Result<Vec<SearchResult>, String> {
        let prompt = GET_RELATED_RESULT_PROMPT
            .replace("{{max_search_result}}", "5")
            .replace("{{topic}}", question)
            .replace(
                "{{search_results}}",
                &serde_json::to_string(&search_results).unwrap_or("".to_string()),
            );
        let chat_result = self
            .chat_with_retry(
                ModelName::General,
                vec![json!({"role": "user", "content": prompt})],
                metadata,
                true,
            )
            .await?;

        // Parse result into query list
        let queries = format_json_str(&chat_result.content);

        let result: Vec<SearchResult> = serde_json::from_str(&queries).map_err(|e| {
            t!(
                "tools.deep_search.failed_to_parse_related_results",
                error = e.to_string()
            )
            .to_string()
        })?; // Added t!

        #[cfg(debug_assertions)]
        {
            log::debug!("Get related results count: {}", result.len());
        }

        Ok(result)
    }

    /// Crawls and extracts content from search result URLs
    ///
    /// This function:
    /// 1. Filters valid HTTP/HTTPS URLs from search results
    /// 2. Processes URLs in concurrent batches (max 5 per batch)
    /// 3. Extracts content in markdown format
    /// 4. Returns JSON string of crawled results
    ///
    /// # Arguments
    /// * `chat_id` - The chat ID for tracking the search process
    /// * `results` - An iterator of SearchResult items containing URLs to crawl
    /// * `metadata` - The chat metadata from fornt-end
    ///
    /// # Returns
    /// * `Option<Vec<SearchResult>>` - A vector of SearchResult items with content extracted
    ///   - Each item contains the original fields from `results` plus an additional `summary` field
    ///   - Returns `None` if no valid URLs are found
    async fn crawl_data<T>(
        &self,
        chat_id: &str,
        results: T,
        metadata: Option<Value>,
    ) -> Option<Vec<SearchResult>>
    where
        T: IntoIterator<Item = SearchResult>,
    {
        if self.should_stop() {
            return None;
        }

        // At most crawl 2 URLs
        let valid_results: Vec<SearchResult> = results
            .into_iter()
            .filter(|r| r.url.starts_with("http://") || r.url.starts_with("https://"))
            .collect();

        if valid_results.is_empty() {
            return None;
        }

        let crawler = Arc::new(Chp::new(self.crawler_url.clone(), None));
        let crawl_results = Arc::new(Mutex::new(Vec::new()));

        // Process in batches, with a maximum of 5 concurrent requests per batch
        for chunk in valid_results.chunks(self.max_crawler_threads) {
            if self.should_stop() {
                break;
            }

            // create tasks for each URL in the batch
            let tasks = chunk.iter().map(|result| {
                let url = result.url.clone();
                let crawler_clone = Arc::clone(&crawler);
                let results_clone = Arc::clone(&crawl_results);

                #[cfg(debug_assertions)]
                log::debug!("Crawling URL: {}", &url);

                let metadata_clone = metadata.clone();
                let title = result.title.clone();
                let stop_flag = self.stop_flag.clone();
                async move {
                    if stop_flag.load(Ordering::Acquire) {
                        return;
                    }

                    self.send_process_message(
                        chat_id,
                        t!("tools.deep_search.crawling_web", url = url.clone()).to_string(),
                        MessageType::Log,
                        metadata_clone.clone(),
                    );

                    let crawl_result = match crawler_clone
                        .web_crawler(&url, Some(json!({"format":"markdown"})))
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            log::warn!("Crawling failed for {}: {}", url, e);
                            return;
                        }
                    };

                    if stop_flag.load(Ordering::Acquire) {
                        return;
                    }
                    if !crawl_result.content.is_empty() {
                        self.send_process_message(
                            chat_id,
                            t!(
                                "tools.deep_search.crawler_success",
                                title = title,
                                url = url.clone()
                            )
                            .to_string(),
                            MessageType::Log,
                            metadata_clone,
                        );

                        let mut result_clone = result.clone();
                        result_clone.snippet = Some(crawl_result.content);
                        let mut results = results_clone.lock().await;
                        results.push(result_clone);

                        #[cfg(debug_assertions)]
                        log::debug!("Crawling finished, URL: {}", &url,);
                    } else {
                        self.send_process_message(
                            chat_id,
                            t!(
                                "tools.deep_search.crawler_failed",
                                title = title,
                                url = url.clone()
                            )
                            .to_string(),
                            MessageType::Log,
                            metadata_clone,
                        );

                        log::warn!("Crawling failed, URL: {}", &url);
                    }
                }
            });

            // Wait for the current batch of tasks to complete
            join_all(tasks).await;
        }

        // Get results
        let mut final_results = crawl_results.lock().await;

        // Reset id, starting from 1
        for (index, result) in final_results.iter_mut().enumerate() {
            result.id = index + 1
        }

        Some(final_results.clone())
    }

    /// Summarizes search results by extracting relevant content for a given question
    ///
    /// This method:
    /// 1. Filters search results that have non-empty summaries
    /// 2. For each result, sends the summary to AI for processing
    /// 3. Returns a new vector of SearchResult with AI-generated summaries
    ///
    /// # Arguments
    /// * `query` - The topic/question to summarize for
    /// * `search_results` - An iterator of SearchResult items to process
    /// * `metadata` - The chat metadata from fornt-end
    ///
    /// # Returns
    /// A vector of SearchResult items where each item contains:
    /// - Original fields from input
    /// - New AI-generated summary focused on the question
    async fn summary<T>(
        &self,
        chat_id: &str,
        query: &str,
        search_results: T,
        metadata: Option<Value>,
    ) -> Result<Vec<SearchResult>, String>
    where
        T: IntoIterator<Item = SearchResult>,
    {
        let mut results = Vec::<SearchResult>::new();
        for result in search_results
            .into_iter()
            .filter(|r| r.snippet.as_deref().map(|s| !s.is_empty()).unwrap_or(false))
        {
            self.check_stop()?;

            let snippet = result
                .snippet
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or("".to_string());
            #[cfg(debug_assertions)]
            {
                log::debug!("Summarizing URL: {}", &result.url);
            }

            self.send_process_message(
                chat_id,
                t!(
                    "tools.deep_search.summarizing_webpage",
                    title = result.title.clone()
                )
                .to_string(),
                MessageType::Log,
                metadata.clone(),
            );

            let prompt = SUMMARIZE_PROMPT
                .replace("{{topic}}", query)
                .replace("{{content}}", snippet.as_str());
            let chat_result = self
                .chat_with_retry(
                    ModelName::General,
                    vec![json!({"role": "user", "content": prompt})],
                    metadata.clone(),
                    false,
                )
                .await;
            let _ = chat_result
                .map(|r| {
                    self.send_process_message(
                        chat_id,
                        t!(
                            "tools.deep_search.summarization_success",
                            title = result.title.clone()
                        )
                        .to_string(),
                        MessageType::Log,
                        metadata.clone(),
                    );

                    #[cfg(debug_assertions)]
                    log::debug!("Summarizing finished, URL: {}", &result.url);

                    let mut result = result.clone();
                    result.snippet = Some(r.content.clone());
                    results.push(result);
                })
                .map_err(|e| {
                    self.send_process_message(
                        chat_id,
                        t!(
                            "tools.deep_search.summarization_failed",
                            title = result.title.clone(),
                            error = e
                        )
                        .to_string(),
                        MessageType::Log,
                        metadata.clone(),
                    );
                    log::error!("Summarizing failed, URL: {}, error: {}", &result.url, e);
                    e
                });
        }
        Ok(results)
    }

    /// Chat with retry 3 times
    ///
    /// # Arguments
    /// * `messages` - The messages to send
    /// * `is_json` - Whether the response is json
    ///
    /// # Returns
    /// * `Ok(ChatCompletionResult)` - The chat completion result
    /// * `Err(String)` - The error message
    async fn chat_with_retry(
        &self,
        model_name: ModelName,
        messages: Vec<Value>,
        metadata: Option<Value>,
        is_json: bool,
    ) -> Result<ChatCompletionResult, String> {
        let model = self.get_model(model_name.clone()).await.ok_or_else(|| {
            let err = t!("tools.model_not_found", model_name = model_name.to_string()).to_string();
            log::error!("{}", err);
            err
        })?;
        let chat_metadata = self.merge_metadata(metadata, model.metadata.clone());

        for i in 0..3 {
            let chat_id = uuid::Uuid::new_v4().to_string();
            match complete_chat_blocking(
                model.api_protocol.clone().try_into()?,
                Some(model.base_url.as_str()),
                model.default_model.clone(),
                Some(model.api_key.as_str()),
                chat_id.clone(),
                messages.clone(),
                None,
                chat_metadata.clone(),
            )
            .await
            {
                Ok(chat_result) => {
                    if chat_result.content.trim().is_empty() {
                        if i >= 2 {
                            log::error!("Failed to complete chat after 3 retries");
                            return Err(t!("tools.max_retries_exceeded", node = "chat_completion")
                                .to_string());
                        }
                        match self.check_stop() {
                            Ok(_) => {}
                            Err(e) => {
                                return Err(e);
                            }
                        }
                        continue;
                    }
                    if is_json {
                        let content = format_json_str(&chat_result.content);
                        match serde_json::from_str::<Value>(&content) {
                            Ok(parsed) => {
                                // ensure the formatted JSON is valid and well-formatted
                                let formatted_content = serde_json::to_string_pretty(&parsed)
                                    .unwrap_or_else(|_| content.clone());
                                let mut new_result = chat_result.clone();
                                new_result.content = formatted_content;
                                return Ok(new_result);
                            }
                            Err(e) => {
                                match self.check_stop() {
                                    Ok(_) => {}
                                    Err(e) => {
                                        return Err(e);
                                    }
                                }
                                log::error!("Failed to parse json: {}", e.to_string());
                                continue;
                            }
                        }
                    }
                    return Ok(chat_result);
                }
                Err(e) => {
                    if i == 2 {
                        log::error!("Failed to complete chat after 3 retries: {}", e.to_string());
                        return Err(
                            t!("tools.max_retries_exceeded", node = "chat_completion").to_string()
                        );
                    }
                    match self.check_stop() {
                        Ok(_) => {}
                        Err(e) => {
                            return Err(e);
                        }
                    }
                    // Wait for 1 second before retrying
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                }
            }
        }
        Err("Failed to complete chat after 3 retries".to_string())
    }

    /// Sends a message to the progress callback if it's set.
    ///
    /// # Arguments
    /// * `chat_id` - The chat ID for tracking the search process
    /// * `chunk` - The message chunk to send
    /// * `message_type` - The type of the message (e.g., Step, Log)
    /// * `metadata` - The chat metadata from fornt-end
    ///
    /// # Returns
    /// * `()` - No return value
    fn send_process_message(
        &self,
        chat_id: &str,
        chunk: String,
        message_type: MessageType,
        metadata: Option<Value>,
    ) {
        if self.should_stop() {
            return;
        }

        (&self.progress_callback)(ChatResponse::new_with_arc(
            chat_id.to_string(),
            chunk.to_string(),
            message_type,
            metadata,
            None,
        ));
    }
}

#[async_trait]
impl ToolDefinition for DeepSearch {
    fn name(&self) -> &str {
        "deep_search"
    }

    fn description(&self) -> &str {
        "Execute a deep search for a given topic: \n1. Generate search queries \n2. Search 3. \nRank results \n4. Crawl results \n5. Summarize results"
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                    "type": "object",
                    "properties": {
                        "model_name": {
                            "type": "string",
                            "enum": ["reasoning", "general"],
                            "description": "Model type: 'reasoning' (planning/analysis) or 'general' (text processing)"
                        },
                        "chat_id": {
                            "type": "string",
                            "description": "Optional chat ID"
                        },
                        "messages": {
                            "type": "array",
                            "description": "Message list with role and content",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "role": {
                                        "type": "string",
                                        "enum": ["system", "user", "assistant", "function"],
                                        "description": "Message sender role"
                                    },
                                    "content": {
                                        "type": "string",
                                        "description": "Message text"
                                    }
                                },
                                "required": ["role", "content"]
                            }
                        },
                        "max_tokens": {
                            "type": "integer",
                            "description": "Maximum tokens to generate"
                        },
                        "temperature": {
                            "type": "number",
                            "description": "Sampling temperature"
                        },
                        "top_p": {
                            "type": "number",
                            "description": "Top-p sampling value"
                        },
                        "top_k": {
                            "type": "integer",
                            "description": "Top-k sampling value"
                        }
                    },
                    "required": ["model_name", "messages"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    async fn call(&self, _param: serde_json::Value) -> NativeToolResult {
        todo!()
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::{
        ai::traits::chat::ChatResponse,
        search::SearchProviderName,
        tools::{DeepSearch, ModelName, ToolManager},
    };

    async fn setup_search_tool() -> DeepSearch {
        let db_path = {
            let dev_dir = &*crate::STORE_DIR.read();
            dev_dir.join("chatspeed.db")
        };
        let main_store = Arc::new(std::sync::RwLock::new(
            crate::db::MainStore::new(db_path).unwrap(),
        ));

        let channel = crate::libs::window_channels::WindowChannels::new();
        let chat_state =
            crate::ai::interaction::chat_completion::ChatState::new(std::sync::Arc::new(channel));
        let process_callback: Arc<dyn Fn(Arc<ChatResponse>) + Send + Sync> = Arc::new(|s| {
            println!("{}", serde_json::to_string(&s).unwrap());
        });

        let ds = DeepSearch::new(
            chat_state,
            None,
            "http://127.0.0.1:12321".to_string(),
            vec![SearchProviderName::Google],
            process_callback,
        );
        ds.add_model(
            ModelName::Reasoning,
            ToolManager::get_model(main_store.clone(), ModelName::General.as_ref()).unwrap(),
        )
        .await;
        ds.add_model(
            ModelName::Reasoning,
            ToolManager::get_model(main_store, ModelName::Reasoning.as_ref()).unwrap(),
        )
        .await;

        ds
    }

    #[tokio::test]
    async fn test_generate_search_queries() {
        let tool = setup_search_tool().await;
        let queries = tool
            .generate_deeper_search_plans("abc", "chatgpt 5 什么时候发布？", None)
            .await;
        dbg!(&queries);
        assert!(queries.is_ok());
    }

    #[tokio::test]
    async fn test_mutil_search_queries() {
        let tools = setup_search_tool().await;
        let result = tools
            .execute_multi_search(vec!["chatgpt 5 什么时候发布？".to_string()], None, None)
            .await;
        dbg!(&result);
        assert!(result.is_ok());
    }
}
