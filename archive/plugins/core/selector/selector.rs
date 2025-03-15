use super::types::{ElementData, ExtractMode, SelectorConfig, SelectorResult};
use crate::plugins::{
    traits::{PluginFactory, PluginInfo, PluginType},
    Plugin, PluginError,
};

use async_trait::async_trait;
use rust_i18n::t;
use scraper::{Html, Selector};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// HTML content selector using CSS selectors
pub struct SelectorPlugin {
    plugin_info: PluginInfo,
}

impl SelectorPlugin {
    /// Creates a new selector plugin instance
    pub fn new() -> Self {
        Self {
            plugin_info: PluginInfo {
                id: "selector".to_string(),
                name: "selector".to_string(),
                version: "0.1.0".to_string(),
            },
        }
    }

    /// Extracts content from HTML using CSS selector rules
    fn extract_content(
        &self,
        html: &str,
        selectors: &HashMap<String, super::types::SelectorRule>,
    ) -> Result<HashMap<String, Vec<ElementData>>, Box<dyn std::error::Error + Send + Sync>> {
        let results: Arc<Mutex<HashMap<String, Vec<ElementData>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let document = Html::parse_document(html);

        for (field_name, rule) in selectors {
            let field_results: Arc<Mutex<Vec<ElementData>>> = Arc::new(Mutex::new(Vec::new()));
            let field_results_clone = Arc::clone(&field_results);
            let attrs_to_extract = rule.attributes.clone().unwrap_or_default();
            let extract_mode = rule.extract.clone();

            // Parse CSS selector
            let selector = Selector::parse(&rule.selector).map_err(|e| {
                Box::new(PluginError::RuntimeError(format!(
                    "Invalid selector: {}",
                    e
                ))) as Box<dyn std::error::Error + Send + Sync>
            })?;

            for element in document.select(&selector) {
                let mut element_data = ElementData {
                    content: None,
                    attributes: HashMap::new(),
                };

                // Extract attributes if specified
                for attr in &attrs_to_extract {
                    if let Some(value) = element.value().attr(attr) {
                        element_data
                            .attributes
                            .insert(attr.clone(), value.to_string());
                    }
                }

                // Extract content based on mode
                let content = match extract_mode {
                    ExtractMode::Text => element
                        .text()
                        .collect::<Vec<_>>()
                        .join("")
                        .trim()
                        .to_string(),
                    ExtractMode::Html => element.inner_html(),
                    ExtractMode::OuterHtml => element.html(),
                };

                if !content.is_empty() {
                    element_data.content = Some(content);
                    if let Ok(mut results) = field_results_clone.lock() {
                        results.push(element_data);
                    }
                }
            }

            // Get the field results safely
            let field_results = field_results.lock().map_err(|e| {
                Box::new(PluginError::RuntimeError(format!(
                    "Failed to lock field results: {}",
                    e
                ))) as Box<dyn std::error::Error + Send + Sync>
            })?;

            if !field_results.is_empty() {
                let mut results = results.lock().map_err(|e| {
                    Box::new(PluginError::RuntimeError(format!(
                        "Failed to lock results: {}",
                        e
                    ))) as Box<dyn std::error::Error + Send + Sync>
                })?;
                results.insert(field_name.clone(), field_results.to_vec());
            }
        }

        // Get the final results safely and clone the data
        let guard = results.lock().map_err(|e| {
            Box::new(PluginError::RuntimeError(format!(
                "Failed to lock final results: {}",
                e
            ))) as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(guard.clone())
    }
}

#[async_trait]
impl crate::plugins::traits::Plugin for SelectorPlugin {
    async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn destroy(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn plugin_info(&self) -> &PluginInfo {
        &self.plugin_info
    }

    fn plugin_type(&self) -> &PluginType {
        &PluginType::Native
    }

    fn input_schema(&self) -> Value {
        json!({
            "properties": {
                "html": {
                    "type": "string",
                    "required": true,
                    "description": "HTML content to extract from"
                },
                "selectors": {
                    "type": "object",
                    "required": true,
                    "description": "CSS selector rules to apply",
                    "properties": {}
                }
            }
        })
    }

    fn output_schema(&self) -> Value {
        json!({
            "fields": {
                "type": "object",
                "required": true,
                "description": t!("selector.description.fields")
            }
        })
    }

    fn validate_input(
        &self,
        input: &Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(obj) = input.as_object() {
            // validate html field
            match obj.get("html") {
                Some(html) if html.is_string() => {
                    let html_str = html.as_str().ok_or_else(|| {
                        PluginError::InvalidInput(t!("selector.errors.invalid_html").to_string())
                    })?;
                    if html_str.is_empty() {
                        return Err(Box::new(PluginError::InvalidInput(
                            t!("selector.errors.empty_html").to_string(),
                        )));
                    }
                }
                _ => {
                    return Err(Box::new(PluginError::InvalidInput(
                        t!("selector.errors.invalid_html").to_string(),
                    )));
                }
            }

            // validate selectors field
            match obj.get("selectors") {
                Some(selectors) if selectors.is_object() => {
                    let selectors_obj = selectors.as_object().ok_or_else(|| {
                        PluginError::InvalidInput(
                            t!("selector.errors.invalid_selectors").to_string(),
                        )
                    })?;

                    // only check properties is empty
                    if selectors_obj.is_empty() {
                        return Err(Box::new(PluginError::InvalidInput(
                            t!("selector.errors.empty_selectors").to_string(),
                        )));
                    }
                }
                _ => {
                    return Err(Box::new(PluginError::InvalidInput(
                        t!("selector.errors.invalid_selectors").to_string(),
                    )));
                }
            }
        } else {
            return Err(Box::new(PluginError::InvalidInput(
                t!("selector.errors.invalid_input").to_string(),
            )));
        }

        Ok(())
    }

    fn validate_output(&self, output: &Value) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(obj) = output.as_object() {
            match obj.get("fields") {
                Some(fields) if fields.is_object() => {
                    let fields_obj = fields.as_object().ok_or_else(|| {
                        PluginError::InvalidOutput(t!("selector.errors.invalid_fields").to_string())
                    })?;

                    if fields_obj.is_empty() {
                        return Err(Box::new(PluginError::InvalidOutput(
                            t!("selector.errors.empty_fields").to_string(),
                        )));
                    }
                }
                _ => {
                    return Err(Box::new(PluginError::InvalidOutput(
                        t!("selector.errors.invalid_fields").to_string(),
                    )));
                }
            }
        } else {
            return Err(Box::new(PluginError::InvalidOutput(
                t!("selector.errors.invalid_output").to_string(),
            )));
        }

        Ok(())
    }

    /// Extracts content from HTML using CSS selector rules
    ///
    /// # Arguments
    /// * `input` - Selector config as JSON object
    /// * `plugin_info` - Plugin info, it's None for the selector plugin
    ///
    /// # Returns
    ///  * `Value` - JSON object with extracted content
    ///
    /// # Errors
    /// * `PluginError::InvalidInput` - If input is invalid
    /// * `PluginError::RuntimeError` - If an error occurs during execution
    ///
    async fn execute(
        &mut self,
        input: Option<Value>,
        _plugin_info: Option<PluginInfo>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let input = input.ok_or_else(|| {
            Box::new(PluginError::InvalidInput(
                "Selector config is required".to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;

        self.validate_input(&input)?;

        let config: SelectorConfig = serde_json::from_value(input).map_err(|e| {
            Box::new(PluginError::InvalidInput(format!(
                "Invalid selector config: {}",
                e
            ))) as Box<dyn std::error::Error + Send + Sync>
        })?;

        let fields = self.extract_content(&config.html, &config.selectors)?;
        let result = SelectorResult { fields };

        let output = serde_json::to_value(result).map_err(|e| {
            Box::new(PluginError::RuntimeError(e.to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        self.validate_output(&output)?;

        Ok(output)
    }
}

/// Factory for creating Selector plugin instances
pub struct SelectorPluginFactory;

impl SelectorPluginFactory {
    /// Creates a new selector plugin factory
    pub fn new() -> Self {
        Self {}
    }
}

impl PluginFactory for SelectorPluginFactory {
    fn create_instance(
        &self,
        _init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(SelectorPlugin::new()))
    }
}

#[cfg(test)]
mod tests {
    use crate::plugins::{core::selector::types::SelectorRule, Plugin};

    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_text_extraction() {
        let html = r#"
            <div class="container">
                <h1 id="title">Hello World</h1>
                <p class="content">This is a paragraph</p>
                <p class="content">Another paragraph</p>
            </div>
        "#;

        let mut selectors = HashMap::new();
        selectors.insert(
            "title".to_string(),
            SelectorRule {
                selector: "#title".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );
        selectors.insert(
            "paragraphs".to_string(),
            SelectorRule {
                selector: "p.content".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        let config = SelectorConfig {
            html: html.to_string(),
            selectors,
        };

        let mut plugin = SelectorPlugin::new();
        let result = plugin
            .execute(Some(serde_json::to_value(config).unwrap()), None)
            .await
            .unwrap();
        let result: SelectorResult = serde_json::from_value(result).unwrap();

        // Check title
        let title = &result.fields["title"];
        assert_eq!(title.len(), 1);
        assert_eq!(title[0].content.as_ref().unwrap(), "Hello World");

        // Check paragraphs
        let paragraphs = &result.fields["paragraphs"];
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(
            paragraphs[0].content.as_ref().unwrap(),
            "This is a paragraph"
        );
        assert_eq!(paragraphs[1].content.as_ref().unwrap(), "Another paragraph");
    }

    #[tokio::test]
    async fn test_html_extraction() {
        let html = r#"
            <div class="container">
                <article class="post" data-id="1">
                    <h2>First Post</h2>
                    <div class="content">
                        <p>Hello</p>
                        <p>World</p>
                    </div>
                </article>
            </div>
        "#;

        let mut selectors = HashMap::new();
        selectors.insert(
            "article".to_string(),
            SelectorRule {
                selector: "article.post".to_string(),
                attributes: Some(vec!["data-id".to_string()]),
                extract: ExtractMode::Html,
            },
        );

        let config = SelectorConfig {
            html: html.to_string(),
            selectors,
        };

        let mut plugin = SelectorPlugin::new();
        let result = plugin
            .execute(Some(serde_json::to_value(config).unwrap()), None)
            .await
            .unwrap();
        let result: SelectorResult = serde_json::from_value(result).unwrap();

        // Check article
        let articles = &result.fields["article"];
        assert_eq!(articles.len(), 1);

        let article = &articles[0];
        // Check attribute
        assert_eq!(article.attributes.get("data-id").unwrap(), "1");
        // Check inner HTML (should contain h2 and div but not article tag)
        let content = article.content.as_ref().unwrap();
        assert!(content.contains("<h2>First Post</h2>"));
        assert!(content.contains("<div class=\"content\">"));
        assert!(!content.contains("<article"));
    }

    #[tokio::test]
    async fn test_outer_html_extraction() {
        let html = r#"
            <div class="container">
                <div class="item" data-type="first">
                    <span>Item 1</span>
                </div>
                <div class="item" data-type="second">
                    <span>Item 2</span>
                </div>
            </div>
        "#;

        let mut selectors = HashMap::new();
        selectors.insert(
            "items".to_string(),
            SelectorRule {
                selector: "div.item".to_string(),
                attributes: Some(vec!["data-type".to_string()]),
                extract: ExtractMode::OuterHtml,
            },
        );

        let config = SelectorConfig {
            html: html.to_string(),
            selectors,
        };

        let mut plugin = SelectorPlugin::new();
        let result = plugin
            .execute(Some(serde_json::to_value(config).unwrap()), None)
            .await
            .unwrap();
        let result: SelectorResult = serde_json::from_value(result).unwrap();

        // Check items
        let items = &result.fields["items"];
        assert_eq!(items.len(), 2);

        // Check first item
        let first_item = &items[0];
        assert_eq!(first_item.attributes.get("data-type").unwrap(), "first");
        let content = first_item.content.as_ref().unwrap();
        assert!(content.starts_with("<div class=\"item\""));
        assert!(content.contains("<span>Item 1</span>"));
        assert!(content.ends_with("</div>"));

        // Check second item
        let second_item = &items[1];
        assert_eq!(second_item.attributes.get("data-type").unwrap(), "second");
        let content = second_item.content.as_ref().unwrap();
        assert!(content.starts_with("<div class=\"item\""));
        assert!(content.contains("<span>Item 2</span>"));
        assert!(content.ends_with("</div>"));
    }

    #[tokio::test]
    async fn test_invalid_selector() {
        let html = "<div>Hello</div>";
        let mut selectors = HashMap::new();
        selectors.insert(
            "invalid".to_string(),
            SelectorRule {
                selector: "div:not(".to_string(), // 使用另一个明确无效的选择器
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        let config = SelectorConfig {
            html: html.to_string(),
            selectors,
        };

        let mut plugin = SelectorPlugin::new();
        let result = plugin
            .execute(Some(serde_json::to_value(config).unwrap()), None)
            .await;
        assert!(result.is_err());

        // 检查错误信息是否包含 "Invalid selector"
        if let Err(e) = result {
            assert!(e.to_string().contains("Invalid selector"));
        }
    }

    #[tokio::test]
    async fn test_complex_selectors() {
        let html = r#"
            <div class="container">
                <article class="post">
                    <h1 class="title">Hello World</h1>
                    <div class="content">
                        <p>First paragraph</p>
                        <p>Second <strong>paragraph</strong></p>
                        <ul class="list">
                            <li>Item 1</li>
                            <li>Item 2</li>
                        </ul>
                    </div>
                    <div class="metadata">
                        <span class="date">2024-01-01</span>
                        <span class="author">John Doe</span>
                    </div>
                </article>
            </div>
        "#;

        let mut selectors = HashMap::new();

        // Test complex nested selector with text extraction
        selectors.insert(
            "title".to_string(),
            SelectorRule {
                selector: "article.post h1.title".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        // Test multiple elements selection with text extraction
        selectors.insert(
            "list_items".to_string(),
            SelectorRule {
                selector: "ul.list li".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        // Test HTML extraction with nested elements
        selectors.insert(
            "content".to_string(),
            SelectorRule {
                selector: "div.content".to_string(),
                attributes: None,
                extract: ExtractMode::Html,
            },
        );

        // Test attribute extraction
        selectors.insert(
            "metadata".to_string(),
            SelectorRule {
                selector: "div.metadata span".to_string(),
                attributes: Some(vec!["class".to_string()]),
                extract: ExtractMode::Text,
            },
        );

        // Test outer HTML with specific element
        selectors.insert(
            "full_article".to_string(),
            SelectorRule {
                selector: "article.post".to_string(),
                attributes: None,
                extract: ExtractMode::OuterHtml,
            },
        );

        let config = SelectorConfig {
            html: html.to_string(),
            selectors,
        };

        let mut plugin = SelectorPlugin::new();
        let result = plugin
            .execute(Some(serde_json::to_value(config).unwrap()), None)
            .await;
        assert!(result.is_ok());

        let result = result.unwrap();
        let result: SelectorResult = serde_json::from_value(result).unwrap();

        // Verify title
        let title_data = &result.fields["title"];
        assert_eq!(title_data.len(), 1);
        assert_eq!(title_data[0].content.as_ref().unwrap(), "Hello World");

        // Verify list items
        let list_items = &result.fields["list_items"];
        assert_eq!(list_items.len(), 2);
        assert_eq!(list_items[0].content.as_ref().unwrap(), "Item 1");
        assert_eq!(list_items[1].content.as_ref().unwrap(), "Item 2");

        // Verify content HTML
        let content = &result.fields["content"];
        assert_eq!(content.len(), 1);
        let content_html = content[0].content.as_ref().unwrap();
        assert!(content_html.contains("<p>First paragraph</p>"));
        assert!(content_html.contains("<p>Second <strong>paragraph</strong></p>"));
        assert!(content_html.contains("<ul class=\"list\">"));

        // Verify metadata spans with attributes
        let metadata = &result.fields["metadata"];
        assert_eq!(metadata.len(), 2);
        assert!(metadata
            .iter()
            .any(|d| d.attributes.get("class").unwrap() == "date"));
        assert!(metadata
            .iter()
            .any(|d| d.attributes.get("class").unwrap() == "author"));

        // Verify full article outer HTML
        let full_article = &result.fields["full_article"];
        assert_eq!(full_article.len(), 1);
        let article_html = full_article[0].content.as_ref().unwrap();
        assert!(article_html.starts_with("<article class=\"post\">"));
        assert!(article_html.ends_with("</article>"));
    }

    #[tokio::test]
    async fn test_wikipedia_extraction() {
        // 获取维基百科的 Rust 编程语言页面
        let url = "https://en.wikipedia.org/wiki/Rust_(programming_language)";
        let response = reqwest::get(url).await.unwrap();
        let html = response.text().await.unwrap();

        let mut selectors = HashMap::new();

        // 提取页面标题
        selectors.insert(
            "title".to_string(),
            SelectorRule {
                selector: "h1#firstHeading".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        // 提取文章第一段内容
        selectors.insert(
            "first_paragraph".to_string(),
            SelectorRule {
                selector: "#mw-content-text .mw-parser-output > p:not(.mw-empty-elt):first-of-type"
                    .to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        // 提取目录标题
        selectors.insert(
            "toc_items".to_string(),
            SelectorRule {
                selector: "div#toc span.toctext".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        // 提取所有段落（用于调试）
        selectors.insert(
            "all_paragraphs".to_string(),
            SelectorRule {
                selector: "#mw-content-text .mw-parser-output > p".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        // 提取所有标题（用于调试）
        selectors.insert(
            "all_headings".to_string(),
            SelectorRule {
                selector: "h1, h2, h3".to_string(),
                attributes: None,
                extract: ExtractMode::Text,
            },
        );

        let config = SelectorConfig { html, selectors };

        let mut plugin = SelectorPlugin::new();
        let result = plugin
            .execute(Some(serde_json::to_value(config).unwrap()), None)
            .await;
        assert!(result.is_ok());

        let result = result.unwrap();
        let result: SelectorResult = serde_json::from_value(result).unwrap();

        // 验证标题
        if let Some(title) = result.fields.get("title") {
            assert!(!title.is_empty());
            let title_text = title[0].content.as_ref().unwrap();
            assert!(title_text.contains("Rust"));
            println!("Title: {}", title_text);
        } else {
            println!("Title not found");
        }

        // 验证第一段内容
        if let Some(first_para) = result.fields.get("first_paragraph") {
            assert!(!first_para.is_empty());
            let first_para_text = first_para[0].content.as_ref().unwrap();
            assert!(first_para_text.contains("Rust"));
            println!("\nFirst paragraph: {}", first_para_text);
        } else {
            println!("First paragraph not found");
        }

        // 验证目录项
        if let Some(toc_items) = result.fields.get("toc_items") {
            if !toc_items.is_empty() {
                println!("\nTable of Contents:");
                for item in toc_items {
                    println!("- {}", item.content.as_ref().unwrap());
                }
            } else {
                println!("No TOC items found");
            }
        } else {
            println!("TOC not found");
        }

        // 打印所有段落（用于调试）
        if let Some(paragraphs) = result.fields.get("all_paragraphs") {
            println!("\nAll paragraphs found: {}", paragraphs.len());
            for (i, p) in paragraphs.iter().take(3).enumerate() {
                println!("Paragraph {}: {}", i + 1, p.content.as_ref().unwrap());
            }
        }

        // 打印所有标题（用于调试）
        if let Some(headings) = result.fields.get("all_headings") {
            println!("\nAll headings found: {}", headings.len());
            for (i, h) in headings.iter().take(5).enumerate() {
                println!("Heading {}: {}", i + 1, h.content.as_ref().unwrap());
            }
        }
    }
}
