use crate::http::client::HttpClient;
use crate::http::types::HttpConfig;
use crate::search::search::{SearchParams, SearchPeriod, SearchProvider, SearchResult};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SERPER_API_URL: &str = "https://google.serper.dev/search";

/// Represents the structure of the request body for the Serper API.
#[derive(Debug, Serialize)]
struct SerperSearchRequest {
    q: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tbs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<u32>,
}

impl From<SerperSearchRequest> for String {
    fn from(request: SerperSearchRequest) -> Self {
        // Serialization of this struct is not expected to fail.
        serde_json::to_string(&request).unwrap()
    }
}

impl<'a> From<&'a SearchParams> for SerperSearchRequest {
    fn from(params: &'a SearchParams) -> Self {
        let tbs = params.period.as_ref().map(|p| match p {
            SearchPeriod::Hour => "qdr:h".to_string(),
            SearchPeriod::Day => "qdr:d".to_string(),
            SearchPeriod::Week => "qdr:w".to_string(),
            SearchPeriod::Month => "qdr:m".to_string(),
            SearchPeriod::Year => "qdr:y".to_string(),
        });

        // Serper's page parameter is 1-based and defaults to 1.
        // We only send it if it's greater than 1.
        let page = params.count.filter(|&p| p > 1);

        SerperSearchRequest {
            q: params.query.clone(),
            hl: params.language.clone(),
            num: params.count,
            tbs,
            page,
        }
    }
}

/// Represents the overall structure of the Serper API response.
#[derive(Debug, Deserialize)]
struct SerperSearchResponse {
    organic: Vec<SerperOrganicResult>,
}

/// Represents a single organic search result from the Serper API.
#[derive(Debug, Deserialize)]
struct SerperOrganicResult {
    title: String,
    link: String,
    snippet: String,
    date: Option<String>,
}

/// The Serper search provider.
pub struct SerperProvider {
    api_key: String,
    http_client: HttpClient,
}

impl SerperProvider {
    /// Creates a new instance of the Serper provider.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The API key for the Serper service.
    /// * `proxy` - An optional proxy URL to be used for requests.
    pub fn new(api_key: String, proxy: Option<String>) -> Result<Self> {
        let http_client = HttpClient::new(proxy)?;
        Ok(Self {
            api_key,
            http_client,
        })
    }
}

#[async_trait]
impl SearchProvider for SerperProvider {
    async fn search(&self, params: &Value) -> Result<Vec<SearchResult>> {
        // 1. Deserialize the generic Value into our specific SearchParams.
        let search_params = SearchParams::try_from(params)?;

        // 2. Map our internal SearchParams to the Serper-specific request format.
        let request_body = SerperSearchRequest::from(&search_params);

        // 3. Perform the HTTP request using the instance's client.
        // Build the request configuration as per the custom HttpClient's design.
        let config = HttpConfig::post(SERPER_API_URL, request_body)
            .header("X-API-KEY", &self.api_key)
            .header("Content-Type", "application/json");

        // Send the request using the http client.
        let response = self.http_client.send_request(config).await?;

        if !response.is_success() {
            return Err(anyhow!(
                "Serper API request failed with status {}: {}",
                response.status,
                response.error.unwrap_or_else(|| response
                    .body
                    .as_deref()
                    .unwrap_or("No error details")
                    .to_string()),
            ));
        }

        // 4. Deserialize the response and map it to our common SearchResult format.
        let body = response
            .body
            .context("Response body was empty, but a successful status was returned.")?;

        let serper_response: SerperSearchResponse = serde_json::from_str(&body)
            .context("Failed to deserialize Serper response from JSON body")?;

        let results = serper_response
            .organic
            .into_iter()
            .map(|item| SearchResult {
                title: item.title,
                url: item.link,
                snippet: Some(item.snippet),
                // The `date` field from Serper can be used as `content`.
                content: item.date,
                score: None, // Serper doesn't provide a scope field.
            })
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use std::env;

    #[tokio::test]
    async fn test_serper_search() {
        // Read api key from SERPER_API_KEY environment variable.
        let api_key = match env::var("SERPER_API_KEY") {
            Ok(key) => key,
            Err(_) => {
                // if the SERPER_API_KEY environment variable is not set, skip this test.
                println!(
                    "Skipping test_serper_search: SERPER_API_KEY environment variable not set."
                );
                return;
            }
        };

        // create a serper provider instance.
        let client = SerperProvider::new(api_key, None).expect("Failed to create SerperClient");

        // execute a search request.
        let query = "what is rust programming language?";
        let search_result = client.search(&json!({ "query": query })).await;

        // assert that the search result is Ok.
        assert!(
            search_result.is_ok(),
            "Search request failed: {:?}",
            search_result.err()
        );

        let response = search_result.unwrap();

        // assert that the search response contains organic results.
        assert!(!response.is_empty(), "Search returned no organic results.");

        // print the top search result title for verification.
        if let Some(first_result) = response.first() {
            log::debug!("Top search result: {:?}", first_result);
        }
    }
}
