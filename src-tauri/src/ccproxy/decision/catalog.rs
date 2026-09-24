//! Decision-only catalog with exact host and optional path matching.
use super::types::DecisionError;
use serde::Deserialize;
use std::{collections::HashSet, sync::LazyLock};
use url::Url;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog { version: u32, rules: Vec<Rule> }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule { id: String, priority: i32, host: String, #[serde(default)] path: Option<String>, adapter: Adapter }

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Adapter { SystemOne }

static CATALOG: LazyLock<Result<Catalog, String>> = LazyLock::new(|| {
    let catalog: Catalog = serde_json::from_str(include_str!("../../../assets/model_catalog/decision_catalog.json")).map_err(|e| e.to_string())?;
    validate_catalog(&catalog).map_err(str::to_string)?;
    Ok(catalog)
});

fn validate_catalog(catalog: &Catalog) -> Result<(), &'static str> {
    if catalog.version != 1 { return Err("unsupported version"); }
    let mut ids = HashSet::new();
    for rule in &catalog.rules {
        if rule.id.trim().is_empty() || !ids.insert(&rule.id) || rule.host.is_empty() || rule.host.contains(['/', ':', '@', '*']) || rule.host != rule.host.to_ascii_lowercase() || rule.path.as_deref().is_some_and(|path| !path.starts_with('/') || path.contains(['?', '#'])) {
            return Err("invalid rule");
        }
    }
    Ok(())
}

pub(super) fn endpoint(value: &str) -> Result<Url, DecisionError> {
    let url = Url::parse(value.trim()).map_err(|_| DecisionError::InvalidUrl("parse"))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || url.path() == "/" {
        return Err(DecisionError::InvalidUrl("scheme, host, path or credentials"));
    }
    if url.scheme() == "http" && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
        return Err(DecisionError::InvalidUrl("non-local HTTP"));
    }
    Ok(url)
}

pub(super) fn resolve(url: &Url) -> Result<(Adapter, String), DecisionError> {
    let catalog = CATALOG.as_ref().map_err(|error| DecisionError::Catalog(error.clone()))?;
    resolve_with_catalog(catalog, url)
}

fn resolve_with_catalog(catalog: &Catalog, url: &Url) -> Result<(Adapter, String), DecisionError> {
    let matches: Vec<&Rule> = catalog.rules.iter().filter(|rule| url.host_str() == Some(rule.host.as_str()) && rule.path.as_deref().is_none_or(|path| url.path() == path)).collect();
    let Some(priority) = matches.iter().map(|rule| rule.priority).max() else { return Ok((Adapter::SystemOne, "default_system_one".into())); };
    let mut highest = matches.into_iter().filter(|rule| rule.priority == priority);
    let rule = highest.next().ok_or(DecisionError::Catalog("missing rule".into()))?;
    if highest.next().is_some() { return Err(DecisionError::Catalog("ambiguous rules".into())); }
    Ok((rule.adapter, rule.id.clone()))
}

pub(super) fn models_endpoint(value: &str) -> Result<Url, DecisionError> {
    let mut url = endpoint(value)?;
    let mut segments = url.path_segments().ok_or(DecisionError::UnsupportedListEndpoint)?.collect::<Vec<_>>();
    if segments.pop() != Some("systemone") { return Err(DecisionError::UnsupportedListEndpoint); }
    segments.push("models");
    url.set_path(&segments.join("/"));
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_and_catalog_guard() {
        assert!(endpoint("https://api.typesafe.ai.evil.test/v1/systemone").is_ok());
        assert!(endpoint("https://user:key@api.typesafe.ai/v1/systemone").is_err());
        assert!(endpoint("https://api.typesafe.ai/v1/systemone?token=x").is_err());
        assert!(endpoint("http://api.typesafe.ai/v1/systemone").is_err());
        assert_eq!(models_endpoint("https://api.typesafe.ai/v1/systemone").unwrap().as_str(), "https://api.typesafe.ai/v1/models");
        assert!(models_endpoint("https://api.typesafe.ai/custom").is_err());
        let catalog = Catalog { version: 1, rules: vec![Rule { id: "first".into(), priority: 1, host: "api.typesafe.ai".into(), path: None, adapter: Adapter::SystemOne }, Rule { id: "second".into(), priority: 1, host: "api.typesafe.ai".into(), path: None, adapter: Adapter::SystemOne }] };
        assert!(resolve_with_catalog(&catalog, &endpoint("https://api.typesafe.ai/v1/systemone").unwrap()).is_err());
        assert_eq!(resolve(&endpoint("https://api.typesafe.ai.evil.test/v1/systemone").unwrap()).unwrap().1, "default_system_one");
        let siliconflow = endpoint("https://api.siliconflow.cn/v1/systemone").unwrap();
        assert_eq!(resolve(&siliconflow).unwrap().1, "default_system_one");
        assert_eq!(resolve(&endpoint("https://api.siliconflow.cn.evil.test/v1/systemone").unwrap()).unwrap().1, "default_system_one");
        assert_eq!(siliconflow.as_str(), "https://api.siliconflow.cn/v1/systemone");
        assert_eq!(models_endpoint(siliconflow.as_str()).unwrap().as_str(), "https://api.siliconflow.cn/v1/models");
    }
}
