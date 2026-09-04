use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use wildmatch::WildMatch;

pub mod pricing;

const EMBEDDED_CATALOG: &str = include_str!("../../assets/model_catalog/model_catalog.json");
const EMBEDDED_TRANSPORT_CATALOG: &str =
    include_str!("../../assets/model_catalog/transport_catalog.json");
const EMBEDDED_MODELS_DEV_CATALOG: &str = include_str!("../../assets/models_dev/catalog.json");
const EMBEDDED_MODELS_DEV_PROVIDERS: &str = include_str!("../../assets/models_dev/providers.json");
pub const MODELS_DEV_CACHE_IDLE_TTL: Duration = Duration::from_secs(15 * 60);
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ModelsDevCatalog {
    pub models: HashMap<String, ModelsDevModel>,
    pub providers: HashMap<String, ModelsDevProvider>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDevProviderList {
    pub providers: Vec<ModelsDevPresetProvider>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDevPresetProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub documentation_url: Option<String>,
    #[serde(default)]
    pub model_list_url: Option<String>,
    #[serde(default)]
    pub key_apply_url: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub responses: bool,
    #[serde(default)]
    pub model_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelsDevProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, ModelsDevModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelsDevModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub attachment: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub structured_output: Option<bool>,
    #[serde(default)]
    pub temperature: Option<bool>,
    #[serde(default)]
    pub modalities: Option<Modalities>,
    #[serde(default)]
    pub limit: Option<ModelLimits>,
    #[serde(default)]
    pub cost: Option<ModelCost>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Modalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelLimits {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub input: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub reasoning: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub input_audio: Option<f64>,
    #[serde(default)]
    pub output_audio: Option<f64>,
    #[serde(default)]
    pub tiers: Vec<serde_json::Value>,
}

use crate::db::{PricingConfig, PricingTier};

#[derive(Default)]
struct CatalogCache<T> {
    value: Option<T>,
    last_used: Option<Instant>,
}

impl<T> CatalogCache<T> {
    fn get(&mut self) -> Option<&T> {
        let expired = self
            .last_used
            .is_some_and(|last_used| last_used.elapsed() >= MODELS_DEV_CACHE_IDLE_TTL);
        if expired {
            self.value = None;
            self.last_used = None;
            return None;
        }
        self.last_used = Some(Instant::now());
        self.value.as_ref()
    }

    fn set(&mut self, value: T) {
        self.value = Some(value);
        self.last_used = Some(Instant::now());
    }
}

static MODELS_DEV: Mutex<CatalogCache<Arc<ModelsDevCatalog>>> = Mutex::new(CatalogCache {
    value: None,
    last_used: None,
});

pub fn set_models_dev_catalog(catalog: ModelsDevCatalog) -> Result<(), CatalogError> {
    MODELS_DEV
        .lock()
        .map_err(|_| CatalogError::CachePoisoned)?
        .set(Arc::new(catalog));
    Ok(())
}

pub fn models_dev_catalog() -> Result<Arc<ModelsDevCatalog>, CatalogError> {
    let mut cache = MODELS_DEV.lock().map_err(|_| CatalogError::CachePoisoned)?;
    if let Some(catalog) = cache.get() {
        return Ok(catalog.clone());
    }
    let catalog: ModelsDevCatalog =
        serde_json::from_str(EMBEDDED_MODELS_DEV_CATALOG).map_err(CatalogError::Parse)?;
    let catalog = Arc::new(catalog);
    cache.set(catalog.clone());
    Ok(catalog)
}

pub fn models_dev_preset_providers() -> Result<ModelsDevProviderList, CatalogError> {
    serde_json::from_str(EMBEDDED_MODELS_DEV_PROVIDERS).map_err(CatalogError::Parse)
}

pub fn pricing_config_from_model(model: &ModelsDevModel) -> Option<PricingConfig> {
    let cost = model.cost.as_ref()?;
    let tiers = cost
        .tiers
        .iter()
        .filter_map(|tier| {
            let context_size = tier.get("tier")?.get("size")?.as_u64()?;
            Some(PricingTier {
                context_size,
                input_per_million: tier.get("input").and_then(|v| v.as_f64()).unwrap_or(0.0),
                output_per_million: tier.get("output").and_then(|v| v.as_f64()).unwrap_or(0.0),
                cache_per_million: tier
                    .get("cache_read")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                reasoning_per_million: tier.get("reasoning").and_then(|v| v.as_f64()),
                cache_write_per_million: tier
                    .get("cache_write")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                audio_input_per_million: tier
                    .get("input_audio")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                audio_output_per_million: tier
                    .get("output_audio")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
            })
        })
        .collect();
    Some(PricingConfig {
        input_per_million: cost.input.unwrap_or(0.0),
        output_per_million: cost.output.unwrap_or(0.0),
        cache_per_million: cost.cache_read.unwrap_or(0.0),
        reasoning_per_million: cost.reasoning,
        reasoning_pricing_mode: if cost.reasoning.is_some_and(|value| value > 0.0) {
            "separate"
        } else {
            "output"
        }
        .into(),
        cache_write_per_million: cost.cache_write.unwrap_or(0.0),
        audio_input_per_million: cost.input_audio.unwrap_or(0.0),
        audio_output_per_million: cost.output_audio.unwrap_or(0.0),
        tiers,
        pricing_source: Some("models.dev".into()),
        multiplier: 1.0,
    })
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingAdapter {
    #[serde(alias = "openai")]
    OpenAi,
    Claude,
    Gemini,
    #[serde(alias = "deepseek")]
    DeepSeek,
    Qwen,
    Glm,
    Kimi,
    #[serde(alias = "stepfun")]
    StepFun,
    HunyuanHy4Preview,
    Doubao,
    #[serde(alias = "sensenova")]
    SenseNova,
    Mistral,
    Mimo,
    Minimax,
    NvidiaNim,
    Amd,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub function_call: Option<bool>,
    #[serde(default)]
    pub image_input: Option<bool>,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            reasoning: None,
            function_call: None,
            image_input: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReasoningPolicy {
    pub supported_efforts: Vec<String>,
    pub default_effort: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MatchPatterns {
    #[serde(default)]
    model: Vec<String>,
    #[serde(default)]
    profile: Vec<String>,
    #[serde(default)]
    endpoint_host: Vec<String>,
    #[serde(default)]
    backend_protocol: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    url: String,
    verified_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileRule {
    id: String,
    priority: i32,
    #[serde(rename = "match")]
    matcher: MatchPatterns,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    capabilities: Capabilities,
    #[serde(default)]
    context_size: Option<u32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    recommended_temperature: Option<f32>,
    #[serde(default)]
    reasoning: Option<ReasoningPolicy>,
    #[serde(default)]
    sources: Vec<Source>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransportRule {
    id: String,
    priority: i32,
    #[serde(rename = "match")]
    matcher: MatchPatterns,
    thinking_adapter: ThinkingAdapter,
    #[serde(default)]
    sources: Vec<Source>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Defaults {
    capabilities: Capabilities,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogDocument {
    version: u32,
    defaults: Defaults,
    profiles: Vec<ProfileRule>,
    transports: Vec<TransportRule>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("failed to parse model catalog: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unsupported model catalog version {0}")]
    UnsupportedVersion(u32),
    #[error("duplicate catalog {kind} id '{id}'")]
    DuplicateId { kind: &'static str, id: String },
    #[error("catalog field conflict at priority {priority}: {field}")]
    Conflict { priority: i32, field: &'static str },
    #[error("catalog profile '{0}' is missing")]
    MissingProfile(String),
    #[error("models.dev catalog is empty or has no usable providers")]
    EmptyModelsDevCatalog,
    #[error("model.dev catalog cache is unavailable")]
    CachePoisoned,
    #[error("catalog transport '{0}' is missing")]
    MissingTransport(String),
    #[error("catalog transport '{transport}' has an invalid reasoning default effort")]
    InvalidReasoningDefault { transport: String },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedModelProfile {
    pub catalog_version: u32,
    pub matched_profile_ids: Vec<String>,
    pub family: Option<String>,
    pub capabilities: Capabilities,
    pub attachment: Option<bool>,
    pub structured_output: Option<bool>,
    pub audio_input: Option<bool>,
    pub audio_output: Option<bool>,
    pub video_input: Option<bool>,
    pub pdf_input: Option<bool>,
    pub context_size: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub recommended_temperature: Option<f32>,
    pub reasoning: Option<ReasoningPolicy>,
    pub thinking_adapter: Option<ThinkingAdapter>,
    pub matched_transport_id: Option<String>,
    pub pricing: Option<PricingConfig>,
}

static CATALOG: LazyLock<Result<CatalogDocument, CatalogError>> =
    LazyLock::new(|| parse_catalog(EMBEDDED_CATALOG));

static TRANSPORT_CATALOG: LazyLock<Result<CatalogDocument, CatalogError>> = LazyLock::new(|| {
    let transport: TransportDocument = serde_json::from_str(EMBEDDED_TRANSPORT_CATALOG)?;
    if transport.version != 1 {
        return Err(CatalogError::UnsupportedVersion(transport.version));
    }
    let mut catalog = parse_catalog(EMBEDDED_CATALOG)?;
    catalog.transports = transport.transports;
    parse_catalog(&serde_json::to_string(&catalog).map_err(CatalogError::Parse)?)
});

#[derive(Debug, Clone, Deserialize)]
struct TransportDocument {
    version: u32,
    transports: Vec<TransportRule>,
}
fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn catalog_model_ids(model_id: &str) -> impl Iterator<Item = String> + '_ {
    let normalized = normalize(model_id);
    let base = normalized
        .split_once(':')
        .map_or(normalized.clone(), |(base, _)| base.to_string());
    [normalized, base].into_iter()
}

fn catalog_model_ids_with_short_id(model_id: &str) -> Vec<String> {
    let mut candidates: Vec<String> = catalog_model_ids(model_id).collect();
    if let Some((_, short_id)) = model_id.trim().split_once('/') {
        let short_candidates: Vec<String> = catalog_model_ids(short_id).collect();
        for candidate in short_candidates {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn matches(patterns: &[String], value: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| WildMatch::new(&normalize(pattern)).matches(value))
}

fn parse_catalog(source: &str) -> Result<CatalogDocument, CatalogError> {
    let catalog: CatalogDocument = serde_json::from_str(source)?;
    if catalog.version != 1 {
        return Err(CatalogError::UnsupportedVersion(catalog.version));
    }
    let mut profile_ids = HashSet::new();
    for profile in &catalog.profiles {
        if !profile_ids.insert(profile.id.clone()) {
            return Err(CatalogError::DuplicateId {
                kind: "profile",
                id: profile.id.clone(),
            });
        }
        if let Some(policy) = &profile.reasoning {
            if !policy
                .supported_efforts
                .iter()
                .any(|effort| effort == &policy.default_effort)
            {
                return Err(CatalogError::InvalidReasoningDefault {
                    transport: profile.id.clone(),
                });
            }
        }
    }
    let mut transport_ids = HashSet::new();
    for transport in &catalog.transports {
        if !transport_ids.insert(transport.id.clone()) {
            return Err(CatalogError::DuplicateId {
                kind: "transport",
                id: transport.id.clone(),
            });
        }
        for profile_id in &transport.matcher.profile {
            if !profile_ids.contains(profile_id) {
                return Err(CatalogError::MissingProfile(profile_id.clone()));
            }
        }
    }
    Ok(catalog)
}

fn merge_option<T: Clone + PartialEq>(
    current: &mut Option<T>,
    current_priority: &mut Option<i32>,
    value: &Option<T>,
    priority: i32,
    field: &'static str,
) -> Result<(), CatalogError> {
    let Some(value) = value else { return Ok(()) };
    match *current_priority {
        Some(existing_priority) if existing_priority > priority => return Ok(()),
        Some(existing_priority)
            if existing_priority == priority && current.as_ref() != Some(value) =>
        {
            return Err(CatalogError::Conflict { priority, field });
        }
        _ => {
            *current = Some(value.clone());
            *current_priority = Some(priority);
        }
    }
    Ok(())
}

fn profile_matches(rule: &ProfileRule, model: &str) -> bool {
    rule.matcher.model.is_empty() || matches(&rule.matcher.model, model)
}

fn transport_matches(
    rule: &TransportRule,
    model: &str,
    host: Option<&str>,
    protocol: Option<&str>,
    profile_ids: &[String],
    ignore_host: bool,
) -> bool {
    let profile_ok = rule.matcher.profile.is_empty()
        || rule
            .matcher
            .profile
            .iter()
            .any(|id| profile_ids.iter().any(|matched| matched == id));
    let model_ok = rule.matcher.model.is_empty() || matches(&rule.matcher.model, model);
    let host_ok = ignore_host
        || rule.matcher.endpoint_host.is_empty()
        || host.is_some_and(|host| matches(&rule.matcher.endpoint_host, host));
    let protocol_ok = rule.matcher.backend_protocol.is_empty()
        || protocol.is_some_and(|protocol| matches(&rule.matcher.backend_protocol, protocol));
    profile_ok && model_ok && host_ok && protocol_ok
}

fn catalog_document() -> Result<CatalogDocument, CatalogError> {
    match CATALOG.as_ref() {
        Ok(catalog) => Ok(catalog.clone()),
        Err(error) => Err(CatalogError::Parse(serde_json::Error::io(
            std::io::Error::other(error.to_string()),
        ))),
    }
}

fn transport_document() -> Result<CatalogDocument, CatalogError> {
    match TRANSPORT_CATALOG.as_ref() {
        Ok(catalog) => Ok(catalog.clone()),
        Err(error) => Err(CatalogError::Parse(serde_json::Error::io(
            std::io::Error::other(error.to_string()),
        ))),
    }
}
pub fn resolve_model_profile(model_id: &str) -> Result<ResolvedModelProfile, CatalogError> {
    let catalog = models_dev_catalog()?;
    resolve_model_profile_from_catalog(catalog.as_ref(), model_id)
}

pub fn resolve_model_profile_from_catalog(
    models_dev: &ModelsDevCatalog,
    model_id: &str,
) -> Result<ResolvedModelProfile, CatalogError> {
    resolve_model_profile_from_catalog_with_context(models_dev, model_id, None, None)
}

pub fn resolve_model_profile_from_catalog_with_context(
    models_dev: &ModelsDevCatalog,
    model_id: &str,
    provider_id: Option<&str>,
    base_url: Option<&str>,
) -> Result<ResolvedModelProfile, CatalogError> {
    let mut profile =
        resolve_model_profile_with_catalog(&catalog_document()?, model_id, None, None, None)?;
    let model = find_catalog_model(models_dev, model_id, provider_id, base_url);
    if let Some(model) = model {
        profile.family = profile.family.or_else(|| model.family.clone());
        profile.capabilities.reasoning = profile.capabilities.reasoning.or(model.reasoning);
        profile.capabilities.function_call = profile.capabilities.function_call.or(model.tool_call);
        profile.attachment = model.attachment.or(profile.attachment);
        profile.structured_output = model.structured_output.or(profile.structured_output);
        profile.capabilities.image_input = model
            .modalities
            .as_ref()
            .map(|modalities| modalities.input.iter().any(|value| value == "image"))
            .or(profile.capabilities.image_input);
        profile.audio_input = model
            .modalities
            .as_ref()
            .map(|modalities| modalities.input.iter().any(|value| value == "audio"))
            .or(profile.audio_input);
        profile.audio_output = model
            .modalities
            .as_ref()
            .map(|modalities| modalities.output.iter().any(|value| value == "audio"))
            .or(profile.audio_output);
        profile.video_input = model
            .modalities
            .as_ref()
            .map(|modalities| modalities.input.iter().any(|value| value == "video"))
            .or(profile.video_input);
        profile.pdf_input = model
            .modalities
            .as_ref()
            .map(|modalities| modalities.input.iter().any(|value| value == "pdf"))
            .or(profile.pdf_input);
        profile.recommended_temperature = model
            .temperature
            .map(|supported| if supported { 0.7 } else { -0.1 })
            .or(profile.recommended_temperature);
        profile.pricing = pricing_config_from_model(model).or(profile.pricing);
        profile.context_size = model
            .limit
            .as_ref()
            .and_then(|l| l.context.map(|v| v.min(u32::MAX as u64) as u32))
            .or(profile.context_size);
        profile.max_output_tokens = model
            .limit
            .as_ref()
            .and_then(|l| l.output.map(|v| v.min(u32::MAX as u64) as u32))
            .or(profile.max_output_tokens);
    }
    Ok(profile)
}

fn find_catalog_model<'a>(
    models_dev: &'a ModelsDevCatalog,
    model_id: &str,
    provider_id: Option<&str>,
    base_url: Option<&str>,
) -> Option<&'a ModelsDevModel> {
    let exact_candidates: Vec<String> = catalog_model_ids(model_id).collect();

    if let Some(provider_id) = provider_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(model) = models_dev
            .providers
            .iter()
            .find(|(id, _)| normalize(id) == normalize(provider_id))
            .and_then(|(_, provider)| find_provider_model(provider, &exact_candidates))
        {
            return Some(model);
        }
    }

    if let Some(host) = base_url.and_then(url_host) {
        let matches: Vec<&ModelsDevModel> = models_dev
            .providers
            .values()
            .filter(|provider| {
                provider.api.as_deref().and_then(url_host).as_deref() == Some(host.as_str())
            })
            .filter_map(|provider| find_provider_model(provider, &exact_candidates))
            .collect();
        if let Some(model) = unique_model(matches) {
            return Some(model);
        }
    }

    let fuzzy_candidates = catalog_model_ids_with_short_id(model_id);
    if let Some(provider_prefix) = model_id
        .trim()
        .split_once('/')
        .map(|(prefix, _)| normalize(prefix))
        .filter(|prefix| !prefix.is_empty())
    {
        if let Some(model) = models_dev
            .providers
            .iter()
            .filter(|(id, _)| normalize(id).starts_with(&provider_prefix))
            .find_map(|(_, provider)| find_provider_model(provider, &fuzzy_candidates))
        {
            return Some(model);
        }
    }

    if let Some(model) = fuzzy_candidates
        .iter()
        .find_map(|candidate| models_dev.models.get(candidate))
    {
        return Some(model);
    }

    models_dev
        .providers
        .values()
        .find_map(|provider| find_provider_model(provider, &fuzzy_candidates))
}

fn find_provider_model<'a>(
    provider: &'a ModelsDevProvider,
    candidates: &[String],
) -> Option<&'a ModelsDevModel> {
    candidates
        .iter()
        .find_map(|candidate| provider.models.get(candidate))
}

fn unique_model<'a>(matches: Vec<&'a ModelsDevModel>) -> Option<&'a ModelsDevModel> {
    let first = matches.first().copied()?;
    matches
        .iter()
        .all(|model| std::ptr::eq(*model, first))
        .then_some(first)
}

fn url_host(value: &str) -> Option<String> {
    Url::parse(value.trim())
        .ok()
        .and_then(|url| url.host_str().map(normalize))
}

pub fn resolve_transport(
    model_id: &str,
    base_url: Option<&str>,
    backend_protocol: Option<&str>,
    metadata: Option<&HashMap<String, String>>,
) -> Result<Option<(ThinkingAdapter, String)>, CatalogError> {
    let catalog = transport_document()?;
    resolve_transport_with_catalog(&catalog, model_id, base_url, backend_protocol, metadata)
}

fn resolve_transport_with_catalog(
    catalog: &CatalogDocument,
    model_id: &str,
    base_url: Option<&str>,
    backend_protocol: Option<&str>,
    metadata: Option<&HashMap<String, String>>,
) -> Result<Option<(ThinkingAdapter, String)>, CatalogError> {
    let profile = resolve_model_profile_with_catalog(
        catalog,
        model_id,
        base_url,
        backend_protocol,
        metadata,
    )?;
    let model = normalize(model_id);
    let host = base_url
        .and_then(|url| Url::parse(url.trim()).ok())
        .and_then(|url| url.host_str().map(normalize));
    if let Some(override_id) = metadata.and_then(|values| values.get("modelCatalogTransport")) {
        let transport = catalog
            .transports
            .iter()
            .find(|rule| rule.id == override_id.trim())
            .ok_or_else(|| CatalogError::MissingTransport(override_id.clone()))?;
        if transport_matches(
            transport,
            &model,
            None,
            backend_protocol.map(normalize).as_deref(),
            &profile.matched_profile_ids,
            true,
        ) {
            return Ok(Some((transport.thinking_adapter, transport.id.clone())));
        }
        return Ok(None);
    }
    let mut candidates: Vec<&TransportRule> = catalog
        .transports
        .iter()
        .filter(|rule| {
            !rule.matcher.endpoint_host.is_empty()
                && transport_matches(
                    rule,
                    &model,
                    host.as_deref(),
                    backend_protocol.map(normalize).as_deref(),
                    &profile.matched_profile_ids,
                    false,
                )
        })
        .collect();
    let Some(max_priority) = candidates.iter().map(|rule| rule.priority).max() else {
        return Ok(None);
    };
    candidates.retain(|rule| rule.priority == max_priority);
    let transport_ids: HashSet<&str> = candidates.iter().map(|rule| rule.id.as_str()).collect();
    if transport_ids.len() != 1 {
        log::warn!(
            "ambiguous model catalog transport for model '{}' at priority {}: {} candidates",
            model,
            max_priority,
            transport_ids.len()
        );
        return Ok(None);
    }
    Ok(candidates
        .first()
        .map(|rule| (rule.thinking_adapter, rule.id.clone())))
}

fn resolve_model_profile_with_catalog(
    catalog: &CatalogDocument,
    model_id: &str,
    _base_url: Option<&str>,
    _protocol: Option<&str>,
    _metadata: Option<&HashMap<String, String>>,
) -> Result<ResolvedModelProfile, CatalogError> {
    let model = normalize(model_id);
    let mut rules: Vec<&ProfileRule> = catalog
        .profiles
        .iter()
        .filter(|rule| profile_matches(rule, &model))
        .collect();
    rules.sort_by_key(|rule| (rule.priority, &rule.id));
    let mut result = ResolvedModelProfile {
        catalog_version: catalog.version,
        matched_profile_ids: rules.iter().map(|rule| rule.id.clone()).collect(),
        family: None,
        capabilities: catalog.defaults.capabilities.clone(),
        attachment: None,
        structured_output: None,
        audio_input: None,
        audio_output: None,
        video_input: None,
        pdf_input: None,
        context_size: None,
        max_output_tokens: None,
        recommended_temperature: None,
        reasoning: None,
        thinking_adapter: None,
        matched_transport_id: None,
        pricing: None,
    };
    let mut family_priority = None;
    let mut reasoning_priority = None;
    let mut reasoning_capability_priority = None;
    let mut function_call_priority = None;
    let mut image_input_priority = None;
    let mut context_priority = None;
    let mut output_priority = None;
    let mut temperature_priority = None;
    for rule in rules {
        merge_option(
            &mut result.family,
            &mut family_priority,
            &rule.family,
            rule.priority,
            "family",
        )?;
        merge_option(
            &mut result.capabilities.reasoning,
            &mut reasoning_capability_priority,
            &rule.capabilities.reasoning,
            rule.priority,
            "capabilities.reasoning",
        )?;
        merge_option(
            &mut result.capabilities.function_call,
            &mut function_call_priority,
            &rule.capabilities.function_call,
            rule.priority,
            "capabilities.functionCall",
        )?;
        merge_option(
            &mut result.capabilities.image_input,
            &mut image_input_priority,
            &rule.capabilities.image_input,
            rule.priority,
            "capabilities.imageInput",
        )?;
        merge_option(
            &mut result.context_size,
            &mut context_priority,
            &rule.context_size,
            rule.priority,
            "contextSize",
        )?;
        merge_option(
            &mut result.max_output_tokens,
            &mut output_priority,
            &rule.max_output_tokens,
            rule.priority,
            "maxOutputTokens",
        )?;
        merge_option(
            &mut result.recommended_temperature,
            &mut temperature_priority,
            &rule.recommended_temperature,
            rule.priority,
            "recommendedTemperature",
        )?;
        merge_option(
            &mut result.reasoning,
            &mut reasoning_priority,
            &rule.reasoning,
            rule.priority,
            "reasoning",
        )?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_models_dev_catalog_is_valid() {
        let catalog = models_dev_catalog().expect("models.dev catalog");
        assert!(!catalog.providers.is_empty());
        assert!(catalog
            .providers
            .values()
            .any(|provider| { provider.models.values().any(|model| model.cost.is_some()) }));
    }

    #[test]
    fn provider_prefix_fallback_uses_short_id_after_exact_matches_fail() {
        let catalog = models_dev_catalog().expect("models.dev catalog");
        let profile = resolve_model_profile_from_catalog_with_context(
            &catalog,
            "ZHIPU/GLM-5.3-Flash",
            Some("aliyun"),
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        )
        .expect("model profile");

        assert_eq!(profile.context_size, Some(1_000_000));
        assert_eq!(profile.max_output_tokens, Some(131_072));
        assert_eq!(profile.capabilities.reasoning, Some(true));
        assert_eq!(profile.capabilities.function_call, Some(true));
    }

    #[test]
    fn provider_prefix_fallback_is_case_insensitive() {
        let catalog = models_dev_catalog().expect("models.dev catalog");
        let profile = resolve_model_profile_from_catalog_with_context(
            &catalog,
            "zHiPu/GlM-5.3-FlAsH",
            Some("ZHIPUAI"),
            Some("https://custom.example/v1"),
        )
        .expect("model profile");

        assert_eq!(profile.context_size, Some(1_000_000));
        assert_eq!(profile.max_output_tokens, Some(131_072));
    }

    #[test]
    fn exact_provider_match_remains_before_fuzzy_provider_fallback() {
        let catalog = models_dev_catalog().expect("models.dev catalog");
        let profile = resolve_model_profile_from_catalog_with_context(
            &catalog,
            "glm-5.3-flash",
            Some("zhipuai"),
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        )
        .expect("model profile");

        assert_eq!(profile.context_size, Some(1_000_000));
    }

    #[test]
    fn provider_context_selects_provider_specific_pricing() {
        let catalog = models_dev_catalog().expect("models.dev catalog");

        let openai = resolve_model_profile_from_catalog_with_context(
            &catalog,
            "gpt-5.6-sol",
            Some("openai"),
            Some("https://api.openai.com/v1"),
        )
        .expect("OpenAI profile");
        let xpersona = resolve_model_profile_from_catalog_with_context(
            &catalog,
            "gpt-5.6-sol",
            Some("xpersona"),
            Some("https://api.xpersona.ai/v1"),
        )
        .expect("xpersona profile");

        assert_eq!(
            openai
                .pricing
                .as_ref()
                .map(|pricing| (pricing.input_per_million, pricing.output_per_million)),
            Some((4.0, 20.0))
        );
        assert_eq!(
            xpersona
                .pricing
                .as_ref()
                .map(|pricing| (pricing.input_per_million, pricing.output_per_million)),
            Some((1.5, 12.0))
        );
    }

    #[test]
    fn model_without_provider_context_falls_back_to_first_matching_provider() {
        let catalog = models_dev_catalog().expect("models.dev catalog");
        let profile =
            resolve_model_profile_from_catalog(&catalog, "gpt-5.6-sol").expect("model profile");

        assert!(profile.pricing.is_some());
    }

    #[test]
    fn profile_matching_normalizes_and_supports_wildcards() {
        let profile = resolve_model_profile("  QWQ-32B  ").expect("catalog");
        assert_eq!(profile.family.as_deref(), Some("qwen"));
        assert_eq!(profile.capabilities.reasoning, Some(true));
    }

    #[test]
    fn aggregator_does_not_select_official_transport() {
        assert!(resolve_transport(
            "deepseek-r1",
            Some("https://openrouter.ai/api/v1"),
            Some("openai"),
            None
        )
        .expect("catalog")
        .is_none());
    }

    #[test]
    fn invalid_catalog_shapes_are_rejected() {
        let duplicate = r#"{"version":1,"defaults":{"capabilities":{}},"profiles":[{"id":"x","priority":1,"match":{"model":["x"]}},{"id":"x","priority":2,"match":{"model":["y"]}}],"transports":[]}"#;
        assert!(matches!(
            parse_catalog(duplicate),
            Err(CatalogError::DuplicateId {
                kind: "profile",
                ..
            })
        ));

        let unknown = r#"{"version":1,"defaults":{"capabilities":{}},"profiles":[],"transports":[],"unexpected":true}"#;
        assert!(parse_catalog(unknown).is_err());

        let invalid_effort = r#"{"version":1,"defaults":{"capabilities":{}},"profiles":[{"id":"x","priority":1,"match":{"model":["x"]},"reasoning":{"supportedEfforts":["low"],"defaultEffort":"high"}}],"transports":[]}"#;
        assert!(matches!(
            parse_catalog(invalid_effort),
            Err(CatalogError::InvalidReasoningDefault { .. })
        ));
    }

    #[test]
    fn higher_priority_rule_overrides_lower_priority_field() {
        let source = r#"{"version":1,"defaults":{"capabilities":{}},"profiles":[{"id":"family","priority":1,"match":{"model":["demo*"]},"family":"base","capabilities":{"reasoning":false}},{"id":"exact","priority":2,"match":{"model":["demo-1"]},"family":"exact","capabilities":{"reasoning":true}}],"transports":[]}"#;
        let catalog = parse_catalog(source).expect("catalog");
        let result = resolve_model_profile_with_catalog(&catalog, "demo-1", None, None, None)
            .expect("profile");
        assert_eq!(result.family.as_deref(), Some("exact"));
        assert_eq!(result.capabilities.reasoning, Some(true));
    }

    #[test]
    fn same_priority_conflicting_fields_are_rejected() {
        let source = r#"{"version":1,"defaults":{"capabilities":{}},"profiles":[{"id":"a","priority":1,"match":{"model":["demo"]},"family":"one"},{"id":"b","priority":1,"match":{"model":["demo"]},"family":"two"}],"transports":[]}"#;
        let catalog = parse_catalog(source).expect("catalog");
        assert!(matches!(
            resolve_model_profile_with_catalog(&catalog, "demo", None, None, None),
            Err(CatalogError::Conflict {
                priority: 1,
                field: "family"
            })
        ));
    }

    #[test]
    fn legacy_official_transports_are_endpoint_bound() {
        let cases = [
            (
                "glm-5.3-flash",
                "https://open.bigmodel.cn/api/paas/v4",
                ThinkingAdapter::Glm,
            ),
            (
                "kimi-k3",
                "https://api.moonshot.cn/v1",
                ThinkingAdapter::Kimi,
            ),
            (
                "step-1-flash",
                "https://api.stepfun.ai/v1",
                ThinkingAdapter::StepFun,
            ),
            (
                "o3-mini",
                "https://api.openai.com/v1",
                ThinkingAdapter::OpenAi,
            ),
            (
                "Qwen3.8-Flash-Next",
                "https://developer.amd.com.cn/radeon/api/v1",
                ThinkingAdapter::Amd,
            ),
            (
                "DeepSeek-V4-Flash",
                "https://developer.amd.com.cn/radeon/api/v1",
                ThinkingAdapter::Amd,
            ),
        ];
        for (model, endpoint, adapter) in cases {
            assert_eq!(
                resolve_transport(model, Some(endpoint), Some("openai"), None)
                    .expect("catalog")
                    .map(|(resolved, _)| resolved),
                Some(adapter)
            );
            assert!(resolve_transport(
                model,
                Some("https://openrouter.ai/api/v1"),
                Some("openai"),
                None
            )
            .expect("catalog")
            .is_none());
        }
    }
    #[test]
    fn same_adapter_at_same_priority_is_still_ambiguous() {
        let source = r#"{"version":1,"defaults":{"capabilities":{}},"profiles":[{"id":"demo","priority":1,"match":{"model":["demo"]}}],"transports":[{"id":"demo-a","priority":10,"match":{"profile":["demo"],"endpointHost":["example.com"]},"thinkingAdapter":"deepseek"},{"id":"demo-b","priority":10,"match":{"profile":["demo"],"endpointHost":["example.com"]},"thinkingAdapter":"deepseek"}]}"#;
        let catalog = parse_catalog(source).expect("catalog");
        assert!(resolve_transport_with_catalog(
            &catalog,
            "demo",
            Some("https://example.com/v1"),
            Some("openai"),
            None,
        )
        .expect("catalog")
        .is_none());
    }

    #[test]
    fn explicit_transport_override_is_validated() {
        let mut metadata = HashMap::from([(
            String::from("modelCatalogTransport"),
            String::from("deepseek-official"),
        )]);
        assert!(resolve_transport(
            "deepseek-r1",
            Some("https://custom.example/v1"),
            Some("openai"),
            Some(&metadata)
        )
        .expect("catalog")
        .is_some());
        metadata.insert("modelCatalogTransport".into(), "missing".into());
        assert!(matches!(
            resolve_transport("deepseek-r1", None, None, Some(&metadata)),
            Err(CatalogError::MissingTransport(_))
        ));
    }
}
