use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TlsBackend {
    Rustls,
    NativeTls,
}

impl Default for TlsBackend {
    fn default() -> Self {
        Self::Rustls
    }
}

/// KNA 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_region")]
    pub region: String,

    /// Auth Region（用于 Token 刷新），未配置时回退到 region
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_region: Option<String>,

    /// API Region（用于 API 请求），未配置时回退到 region
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_region: Option<String>,

    #[serde(default = "default_kiro_version")]
    pub kiro_version: String,

    #[serde(default)]
    pub machine_id: Option<String>,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(default = "default_system_version")]
    pub system_version: String,

    #[serde(default = "default_node_version")]
    pub node_version: String,

    #[serde(default = "default_tls_backend")]
    pub tls_backend: TlsBackend,

    /// 外部 count_tokens API 地址（可选）
    #[serde(default)]
    pub count_tokens_api_url: Option<String>,

    /// count_tokens API 密钥（可选）
    #[serde(default)]
    pub count_tokens_api_key: Option<String>,

    /// count_tokens API 认证类型（可选，"x-api-key" 或 "bearer"，默认 "x-api-key"）
    #[serde(default = "default_count_tokens_auth_type")]
    pub count_tokens_auth_type: String,

    /// HTTP 代理地址（可选）
    /// 支持格式: http://host:port, https://host:port, socks5://host:port
    #[serde(default)]
    pub proxy_url: Option<String>,

    /// 代理认证用户名（可选）
    #[serde(default)]
    pub proxy_username: Option<String>,

    /// 代理认证密码（可选）
    #[serde(default)]
    pub proxy_password: Option<String>,

    /// Admin API 密钥（可选，启用 Admin API 功能）
    #[serde(default)]
    pub admin_api_key: Option<String>,

    /// 负载均衡模式（"priority" 或 "balanced"）
    #[serde(default = "default_load_balancing_mode")]
    pub load_balancing_mode: String,

    /// 是否开启非流式响应的 thinking 块提取（默认 true）
    ///
    /// 启用后，非流式响应中的 `<thinking>...</thinking>` 标签会被解析为
    /// 独立的 `{"type": "thinking", ...}` 内容块,与流式响应行为一致。
    #[serde(default = "default_extract_thinking")]
    pub extract_thinking: bool,

    /// 默认端点名称（凭据未显式指定 endpoint 时使用，默认 "ide"）
    #[serde(default = "default_endpoint")]
    pub default_endpoint: String,

    /// 端点特定的配置
    ///
    /// 键为端点名（如 "ide" / "cli"），值为该端点自由定义的参数对象。
    /// 未在此表出现的端点沿用实现内置默认值。
    #[serde(default)]
    pub endpoints: HashMap<String, serde_json::Value>,

    /// 配置文件路径（运行时元数据，不写入 JSON）
    #[serde(skip)]
    config_path: Option<PathBuf>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_region() -> String {
    "us-east-1".to_string()
}

fn default_kiro_version() -> String {
    "0.11.107".to_string()
}

fn default_system_version() -> String {
    const SYSTEM_VERSIONS: &[&str] = &["darwin#24.6.0", "win32#10.0.22631"];
    SYSTEM_VERSIONS[fastrand::usize(..SYSTEM_VERSIONS.len())].to_string()
}

fn default_node_version() -> String {
    "22.22.0".to_string()
}

fn default_count_tokens_auth_type() -> String {
    "x-api-key".to_string()
}

fn default_tls_backend() -> TlsBackend {
    TlsBackend::Rustls
}

fn default_load_balancing_mode() -> String {
    "priority".to_string()
}

fn default_extract_thinking() -> bool {
    true
}

fn default_endpoint() -> String {
    crate::kiro::endpoint::ide::IDE_ENDPOINT_NAME.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            region: default_region(),
            auth_region: None,
            api_region: None,
            kiro_version: default_kiro_version(),
            machine_id: None,
            api_key: None,
            system_version: default_system_version(),
            node_version: default_node_version(),
            tls_backend: default_tls_backend(),
            count_tokens_api_url: None,
            count_tokens_api_key: None,
            count_tokens_auth_type: default_count_tokens_auth_type(),
            proxy_url: None,
            proxy_username: None,
            proxy_password: None,
            admin_api_key: None,
            load_balancing_mode: default_load_balancing_mode(),
            extract_thinking: default_extract_thinking(),
            default_endpoint: default_endpoint(),
            endpoints: HashMap::new(),
            config_path: None,
        }
    }
}

impl Config {
    /// 获取默认配置文件路径
    pub fn default_config_path() -> &'static str {
        "config.json"
    }

    /// 获取有效的 Auth Region（用于 Token 刷新）
    /// 优先使用 auth_region，未配置时回退到 region
    pub fn effective_auth_region(&self) -> &str {
        self.auth_region.as_deref().unwrap_or(&self.region)
    }

    /// 获取有效的 API Region（用于 API 请求）
    /// 优先使用 api_region，未配置时回退到 region
    pub fn effective_api_region(&self) -> &str {
        self.api_region.as_deref().unwrap_or(&self.region)
    }

    /// 从文件加载配置
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            // 配置文件不存在，返回默认配置
            let mut config = Self::default();
            config.config_path = Some(path.to_path_buf());
            return Ok(config);
        }

        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        config.config_path = Some(path.to_path_buf());
        Ok(config)
    }

    /// 用环境变量覆盖配置文件中的值。
    ///
    /// 这主要用于 Zeabur 等 PaaS 平台：配置文件仍然可用，但关键运行时配置
    /// 可以直接从环境变量注入，避免必须挂载 `/app/config/config.json`。
    pub fn apply_env_overrides(&mut self) -> anyhow::Result<()> {
        if let Some((_, value)) = env_value(&["KIRO_RS_HOST", "BIND_HOST", "HOST"]) {
            self.host = value;
        }

        if let Some((name, value)) = env_value(&["KIRO_RS_PORT", "PORT"]) {
            self.port = value
                .parse::<u16>()
                .with_context(|| format!("环境变量 {name} 不是有效端口: {value}"))?;
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_REGION", "REGION"]) {
            self.region = value;
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_AUTH_REGION", "AUTH_REGION"]) {
            self.auth_region = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_API_REGION", "API_REGION"]) {
            self.api_region = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_KIRO_VERSION", "KIRO_VERSION"]) {
            self.kiro_version = value;
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_MACHINE_ID", "MACHINE_ID"]) {
            self.machine_id = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_API_KEY", "API_KEY"]) {
            self.api_key = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_SYSTEM_VERSION", "SYSTEM_VERSION"]) {
            self.system_version = value;
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_NODE_VERSION", "NODE_VERSION"]) {
            self.node_version = value;
        }

        if let Some((name, value)) = env_value(&["KIRO_RS_TLS_BACKEND", "TLS_BACKEND"]) {
            self.tls_backend = parse_tls_backend(&value)
                .with_context(|| format!("环境变量 {name} 不是有效 TLS 后端: {value}"))?;
        }

        if let Some((_, value)) = env_value(&["COUNT_TOKENS_API_URL"]) {
            self.count_tokens_api_url = Some(value);
        }

        if let Some((_, value)) = env_value(&["COUNT_TOKENS_API_KEY"]) {
            self.count_tokens_api_key = Some(value);
        }

        if let Some((_, value)) = env_value(&["COUNT_TOKENS_AUTH_TYPE"]) {
            self.count_tokens_auth_type = value;
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_PROXY_URL", "PROXY_URL"]) {
            self.proxy_url = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_PROXY_USERNAME", "PROXY_USERNAME"]) {
            self.proxy_username = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_PROXY_PASSWORD", "PROXY_PASSWORD"]) {
            self.proxy_password = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_ADMIN_API_KEY", "ADMIN_API_KEY"]) {
            self.admin_api_key = Some(value);
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_LOAD_BALANCING_MODE", "LOAD_BALANCING_MODE"])
        {
            self.load_balancing_mode = value;
        }

        if let Some((name, value)) = env_value(&["KIRO_RS_EXTRACT_THINKING", "EXTRACT_THINKING"]) {
            self.extract_thinking = parse_bool(&value)
                .with_context(|| format!("环境变量 {name} 不是有效布尔值: {value}"))?;
        }

        if let Some((_, value)) = env_value(&["KIRO_RS_DEFAULT_ENDPOINT", "DEFAULT_ENDPOINT"]) {
            self.default_endpoint = value;
        }

        Ok(())
    }

    /// 获取配置文件路径（如果有）
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// 将当前配置写回原始配置文件
    pub fn save(&self) -> anyhow::Result<()> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("配置文件路径未知，无法保存配置"))?;

        let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        fs::write(path, content)
            .with_context(|| format!("写入配置文件失败: {}", path.display()))?;
        Ok(())
    }
}

fn env_value<'a>(names: &'a [&'a str]) -> Option<(&'a str, String)> {
    for name in names {
        if let Ok(value) = std::env::var(name) {
            let value = value.trim();
            if !value.is_empty() {
                return Some((*name, value.to_string()));
            }
        }
    }
    None
}

fn parse_tls_backend(value: &str) -> anyhow::Result<TlsBackend> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "rustls" => Ok(TlsBackend::Rustls),
        "native-tls" => Ok(TlsBackend::NativeTls),
        _ => anyhow::bail!("支持的值为 rustls 或 native-tls"),
    }
}

fn parse_bool(value: &str) -> anyhow::Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("支持的值为 true/false, 1/0, yes/no 或 on/off"),
    }
}
