//! Prompt cache accounting for Anthropic-compatible usage fields.
//!
//! This cache is local to the proxy process. It does not remove prompt content
//! from requests sent to Kiro; it only makes Anthropic cache usage visible to
//! clients that send `cache_control` breakpoints.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::anthropic::types::{CacheControl, Message, SystemMessage, Tool};
use crate::token;

const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);
const EXTENDED_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_BREAKPOINTS: usize = 4;

static CACHE_STORE: Mutex<BTreeMap<String, CacheEntry>> = Mutex::new(BTreeMap::new());

#[derive(Debug, Clone)]
struct CacheEntry {
    tokens: i32,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct CacheBreakpoint {
    hash: String,
    tokens: i32,
    ttl: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct CacheResult {
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
    pub uncached_input_tokens: i32,
}

impl CacheResult {
    pub fn cached_input_tokens(&self) -> i32 {
        self.cache_creation_input_tokens + self.cache_read_input_tokens
    }

    pub fn input_tokens_after_cache(&self, observed_input_tokens: Option<i32>) -> i32 {
        match observed_input_tokens {
            Some(tokens) => (tokens - self.cached_input_tokens()).max(0),
            None => self.uncached_input_tokens,
        }
    }
}

pub fn account_prompt_cache(
    model: &str,
    api_key: &str,
    system: Option<&Vec<SystemMessage>>,
    messages: &[Message],
    tools: Option<&Vec<Tool>>,
    total_input_tokens: i32,
) -> CacheResult {
    let breakpoints = compute_cache_breakpoints(model, system, messages, tools);
    lookup_or_create(api_key, &breakpoints, total_input_tokens)
}

fn compute_cache_breakpoints(
    model: &str,
    system: Option<&Vec<SystemMessage>>,
    messages: &[Message],
    tools: Option<&Vec<Tool>>,
) -> Vec<CacheBreakpoint> {
    let mut hasher = Sha256::new();
    let mut breakpoints = Vec::new();
    let mut cumulative_tokens = 0;

    if let Some(tools) = tools {
        for tool in tools {
            let normalized = normalize_tool(tool);
            hasher.update(b"tool:");
            hasher.update(normalized.as_bytes());
            cumulative_tokens += token::count_tokens(&normalized) as i32;

            if let Some(cache_control) = &tool.cache_control {
                push_breakpoint(&mut breakpoints, &hasher, cumulative_tokens, cache_control);
            }
        }
    }

    if let Some(system) = system {
        for msg in system {
            hasher.update(b"system:");
            hasher.update(msg.text.as_bytes());
            cumulative_tokens += token::count_tokens(&msg.text) as i32;

            if let Some(cache_control) = &msg.cache_control {
                push_breakpoint(&mut breakpoints, &hasher, cumulative_tokens, cache_control);
            }
        }
    }

    for msg in messages {
        hasher.update(b"role:");
        hasher.update(msg.role.as_bytes());

        match &msg.content {
            Value::String(text) => {
                hasher.update(b"text:");
                hasher.update(text.as_bytes());
                cumulative_tokens += token::count_tokens(text) as i32;
            }
            Value::Array(blocks) => {
                for block in blocks {
                    let normalized = normalize_block_for_hash(block);
                    hasher.update(b"block:");
                    hasher.update(normalized.as_bytes());
                    cumulative_tokens += count_block_tokens(block);

                    if let Some(cache_control) = block.get("cache_control").and_then(|value| {
                        serde_json::from_value::<CacheControl>(value.clone()).ok()
                    }) {
                        push_breakpoint(
                            &mut breakpoints,
                            &hasher,
                            cumulative_tokens,
                            &cache_control,
                        );
                    }
                }
            }
            other => {
                let normalized = normalize_json_value(other);
                hasher.update(b"content:");
                hasher.update(normalized.as_bytes());
                cumulative_tokens += token::count_tokens(&normalized) as i32;
            }
        }
    }

    if breakpoints.len() > MAX_BREAKPOINTS {
        let keep_from = breakpoints.len() - MAX_BREAKPOINTS;
        breakpoints.drain(..keep_from);
    }

    tracing::debug!(
        model = model,
        breakpoint_count = breakpoints.len(),
        "computed prompt cache breakpoints"
    );

    breakpoints
}

fn lookup_or_create(
    api_key: &str,
    breakpoints: &[CacheBreakpoint],
    total_input_tokens: i32,
) -> CacheResult {
    if breakpoints.is_empty() {
        return CacheResult {
            uncached_input_tokens: total_input_tokens,
            ..Default::default()
        };
    }

    let now = Instant::now();
    let mut store = CACHE_STORE.lock();
    store.retain(|_, entry| entry.expires_at > now);

    let mut result = CacheResult::default();
    let namespace = if api_key.is_empty() {
        "anonymous"
    } else {
        api_key
    };

    for (idx, breakpoint) in breakpoints.iter().enumerate().rev() {
        let key = cache_key(namespace, &breakpoint.hash);
        if let Some(entry) = store.get_mut(&key) {
            result.cache_read_input_tokens = entry.tokens;
            entry.expires_at = now + breakpoint.ttl;

            let mut previous_tokens = entry.tokens;
            for later in breakpoints.iter().skip(idx + 1) {
                let later_key = cache_key(namespace, &later.hash);
                store.insert(
                    later_key,
                    CacheEntry {
                        tokens: later.tokens,
                        expires_at: now + later.ttl,
                    },
                );
                result.cache_creation_input_tokens += (later.tokens - previous_tokens).max(0);
                previous_tokens = later.tokens;
            }
            break;
        }
    }

    if result.cache_read_input_tokens == 0 {
        let mut previous_tokens = 0;
        for breakpoint in breakpoints {
            let key = cache_key(namespace, &breakpoint.hash);
            store.insert(
                key,
                CacheEntry {
                    tokens: breakpoint.tokens,
                    expires_at: now + breakpoint.ttl,
                },
            );
            result.cache_creation_input_tokens += (breakpoint.tokens - previous_tokens).max(0);
            previous_tokens = breakpoint.tokens;
        }
    }

    let cached_tokens = result.cache_creation_input_tokens + result.cache_read_input_tokens;
    result.uncached_input_tokens = (total_input_tokens - cached_tokens).max(0);

    tracing::debug!(
        cache_creation_input_tokens = result.cache_creation_input_tokens,
        cache_read_input_tokens = result.cache_read_input_tokens,
        uncached_input_tokens = result.uncached_input_tokens,
        "prompt cache accounting result"
    );

    result
}

fn push_breakpoint(
    breakpoints: &mut Vec<CacheBreakpoint>,
    hasher: &Sha256,
    tokens: i32,
    cache_control: &CacheControl,
) {
    if cache_control.cache_type != "ephemeral" {
        return;
    }

    breakpoints.push(CacheBreakpoint {
        hash: format!("{:x}", hasher.clone().finalize()),
        tokens: tokens.max(0),
        ttl: parse_ttl(cache_control),
    });
}

fn parse_ttl(cache_control: &CacheControl) -> Duration {
    match cache_control.ttl.as_deref() {
        Some("1h") => EXTENDED_TTL,
        _ => DEFAULT_TTL,
    }
}

fn cache_key(namespace: &str, hash: &str) -> String {
    let namespace_hash = Sha256::digest(namespace.as_bytes());
    format!("{:x}:{}", namespace_hash, hash)
}

fn normalize_tool(tool: &Tool) -> String {
    let mut object = serde_json::Map::new();
    object.insert("name".to_string(), Value::String(tool.name.clone()));
    object.insert(
        "description".to_string(),
        Value::String(tool.description.clone()),
    );
    object.insert(
        "input_schema".to_string(),
        serde_json::to_value(&tool.input_schema).unwrap_or(Value::Null),
    );
    if let Some(tool_type) = &tool.tool_type {
        object.insert("type".to_string(), Value::String(tool_type.clone()));
    }
    normalize_json_value(&Value::Object(object))
}

fn count_block_tokens(block: &Value) -> i32 {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .map(|text| token::count_tokens(text) as i32)
            .unwrap_or(0),
        Some("tool_result") => block.get("content").map(count_content_tokens).unwrap_or(0),
        Some("tool_use") => block
            .get("input")
            .map(normalize_json_value)
            .map(|input| token::count_tokens(&input) as i32)
            .unwrap_or(0),
        _ => token::count_tokens(&normalize_json_value(block)) as i32,
    }
}

fn count_content_tokens(content: &Value) -> i32 {
    match content {
        Value::String(text) => token::count_tokens(text) as i32,
        Value::Array(items) => items.iter().map(count_block_tokens).sum(),
        other => token::count_tokens(&normalize_json_value(other)) as i32,
    }
}

fn normalize_block_for_hash(block: &Value) -> String {
    let mut block_without_cache_control = block.clone();
    if let Value::Object(map) = &mut block_without_cache_control {
        map.remove("cache_control");
    }
    normalize_json_value(&block_without_cache_control)
}

fn normalize_json_value(value: &Value) -> String {
    serde_json::to_string(&sort_json_value(value)).unwrap_or_default()
}

fn sort_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            for (key, value) in BTreeMap::from_iter(map.iter()) {
                sorted.insert(key.clone(), sort_json_value(value));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.iter().map(sort_json_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
pub fn clear_for_tests() {
    CACHE_STORE.lock().clear();
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn cache_control() -> CacheControl {
        CacheControl {
            cache_type: "ephemeral".to_string(),
            ttl: None,
        }
    }

    #[test]
    fn creates_then_reads_message_block_cache() {
        clear_for_tests();
        let messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {
                    "type": "text",
                    "text": "stable prefix content for prompt cache",
                    "cache_control": {"type": "ephemeral"}
                },
                {
                    "type": "text",
                    "text": "uncached suffix"
                }
            ]),
        }];
        let total = token::count_all_tokens(
            "claude-sonnet-4-6".to_string(),
            None,
            messages.clone(),
            None,
        ) as i32;

        let first = account_prompt_cache("claude-sonnet-4-6", "key", None, &messages, None, total);
        assert!(first.cache_creation_input_tokens > 0);
        assert_eq!(first.cache_read_input_tokens, 0);

        let second = account_prompt_cache("claude-sonnet-4-6", "key", None, &messages, None, total);
        assert_eq!(second.cache_creation_input_tokens, 0);
        assert_eq!(
            second.cache_read_input_tokens,
            first.cache_creation_input_tokens
        );
    }

    #[test]
    fn supports_system_cache_control() {
        clear_for_tests();
        let system = vec![SystemMessage {
            text: "stable system prompt".to_string(),
            cache_control: Some(cache_control()),
        }];
        let messages = vec![Message {
            role: "user".to_string(),
            content: Value::String("hello".to_string()),
        }];
        let total = token::count_all_tokens(
            "claude-sonnet-4-6".to_string(),
            Some(system.clone()),
            messages.clone(),
            None,
        ) as i32;

        let first = account_prompt_cache(
            "claude-sonnet-4-6",
            "key-system",
            Some(&system),
            &messages,
            None,
            total,
        );
        let second = account_prompt_cache(
            "claude-sonnet-4-6",
            "key-system",
            Some(&system),
            &messages,
            None,
            total,
        );

        assert!(first.cache_creation_input_tokens > 0);
        assert_eq!(
            second.cache_read_input_tokens,
            first.cache_creation_input_tokens
        );
    }
}
