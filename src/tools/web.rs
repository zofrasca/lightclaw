use crate::tools::ToolError;
use html2text::from_read;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use rig::completion::request::ToolDefinition;
use rig::tool::Tool;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use url::Url;

const DEFAULT_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36";
const MAX_REDIRECTS: usize = 5;

#[derive(Clone)]
pub struct WebSearchTool {
    api_key: Option<String>,
}

impl WebSearchTool {
    pub fn new(api_key: Option<String>) -> Self {
        Self { api_key }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WebSearchArgs {
    /// Search query
    pub query: String,
    /// Number of results (1-10)
    #[serde(default, deserialize_with = "de_optional_u8")]
    pub count: Option<u8>,
}

fn de_optional_u8<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| D::Error::custom("count must be an integer between 0 and 255")),
        Some(serde_json::Value::String(s)) => s
            .trim()
            .parse::<u8>()
            .map(Some)
            .map_err(|_| D::Error::custom("count string must be an integer between 0 and 255")),
        Some(_) => Err(D::Error::custom(
            "count must be an integer or integer string",
        )),
    }
}

impl Tool for WebSearchTool {
    const NAME: &'static str = "web_search";
    type Args = WebSearchArgs;
    type Output = String;
    type Error = ToolError;

    fn definition(
        &self,
        _prompt: String,
    ) -> impl std::future::Future<Output = ToolDefinition> + Send {
        async {
            ToolDefinition {
                name: Self::NAME.to_string(),
                description: "Search the web. Returns titles, URLs, and snippets.".to_string(),
                parameters: serde_json::to_value(schemars::schema_for!(WebSearchArgs)).unwrap(),
            }
        }
    }

    fn call(
        &self,
        args: Self::Args,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        async move {
            let Some(api_key) = &self.api_key else {
                return Ok("Error: BRAVE_API_KEY not configured".to_string());
            };
            let n = args.count.unwrap_or(5).min(10).max(1);
            let client = reqwest::Client::new();
            let res = client
                .get("https://api.search.brave.com/res/v1/web/search")
                .query(&[("q", &args.query), ("count", &n.to_string())])
                .header(ACCEPT, "application/json")
                .header("X-Subscription-Token", api_key)
                .send()
                .await
                .map_err(|e| ToolError::msg(e.to_string()))?;
            let status = res.status();
            if !status.is_success() {
                return Ok(format!("Error: Brave search failed with status {status}"));
            }
            let body: serde_json::Value = res
                .json()
                .await
                .map_err(|e| ToolError::msg(e.to_string()))?;
            let results = body
                .get("web")
                .and_then(|w| w.get("results"))
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            if results.is_empty() {
                return Ok(format!("No results for: {}", args.query));
            }
            let mut lines = vec![format!("Results for: {}\n", args.query)];
            for (i, item) in results.iter().take(n as usize).enumerate() {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                lines.push(format!("{}. {}\n   {}", i + 1, title, url));
                if let Some(desc) = item.get("description").and_then(|v| v.as_str()) {
                    lines.push(format!("   {}", desc));
                }
            }
            Ok(lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WebSearchArgs;

    #[test]
    fn web_search_args_accept_numeric_count() {
        let args: WebSearchArgs =
            serde_json::from_value(serde_json::json!({"query": "hn", "count": 10})).unwrap();
        assert_eq!(args.count, Some(10));
    }

    #[test]
    fn web_search_args_accept_string_count() {
        let args: WebSearchArgs =
            serde_json::from_value(serde_json::json!({"query": "hn", "count": "10"})).unwrap();
        assert_eq!(args.count, Some(10));
    }
}

#[derive(Clone)]
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct WebFetchArgs {
    /// URL to fetch
    pub url: String,
    /// Extract mode: "markdown" or "text"
    #[serde(default, alias = "extractMode")]
    pub extract_mode: Option<String>,
    /// Maximum characters to return (minimum 100)
    #[serde(default, alias = "maxChars", deserialize_with = "de_optional_usize")]
    pub max_chars: Option<usize>,
}

fn de_optional_usize<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .map(|v| Some(v as usize))
            .ok_or_else(|| D::Error::custom("max_chars must be a non-negative integer")),
        Some(serde_json::Value::String(s)) => s
            .trim()
            .parse::<usize>()
            .map(Some)
            .map_err(|_| D::Error::custom("max_chars string must be an integer")),
        Some(_) => Err(D::Error::custom(
            "max_chars must be an integer or integer string",
        )),
    }
}

impl Tool for WebFetchTool {
    const NAME: &'static str = "web_fetch";
    type Args = WebFetchArgs;
    type Output = String;
    type Error = ToolError;

    fn definition(
        &self,
        _prompt: String,
    ) -> impl std::future::Future<Output = ToolDefinition> + Send {
        async {
            ToolDefinition {
                name: Self::NAME.to_string(),
                description: "Fetch URL and extract readable content (HTML → markdown/text)."
                    .to_string(),
                parameters: serde_json::to_value(schemars::schema_for!(WebFetchArgs)).unwrap(),
            }
        }
    }

    fn call(
        &self,
        args: Self::Args,
    ) -> impl std::future::Future<Output = Result<Self::Output, Self::Error>> + Send {
        async move {
            if let Err(err) = validate_url(&args.url) {
                return Ok(
                    json!({ "error": format!("URL validation failed: {err}"), "url": args.url })
                        .to_string(),
                );
            }
            let extract_mode = args
                .extract_mode
                .as_deref()
                .map(|m| m.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "text".to_string());
            let max_chars = args.max_chars.unwrap_or(50_000);
            let mut headers = HeaderMap::new();
            headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_UA));
            let client = reqwest::Client::builder()
                .default_headers(headers)
                .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
                .build()
                .map_err(|e| ToolError::msg(e.to_string()))?;
            let res = client
                .get(&args.url)
                .send()
                .await
                .map_err(|e| ToolError::msg(e.to_string()))?;
            let status = res.status();
            let final_url = res.url().to_string();
            let ctype = res
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let text = res
                .text()
                .await
                .map_err(|e| ToolError::msg(e.to_string()))?;
            let mut extractor = "raw";
            let mut out_text = text.clone();
            if extract_mode == "raw" {
                extractor = "raw";
            } else if ctype.contains("application/json") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    out_text = serde_json::to_string_pretty(&val).unwrap_or(text);
                    extractor = "json";
                }
            } else if ctype.contains("text/html")
                || text.to_ascii_lowercase().starts_with("<!doctype")
                || text.to_ascii_lowercase().starts_with("<html")
            {
                let rendered = from_read(text.as_bytes(), 100);
                out_text = rendered;
                extractor = "html2text";
            }
            let truncated = out_text.len() > max_chars;
            if truncated {
                out_text.truncate(max_chars);
            }
            Ok(json!({
                "url": args.url,
                "finalUrl": final_url,
                "status": status.as_u16(),
                "extractor": extractor,
                "extractMode": extract_mode,
                "truncated": truncated,
                "length": out_text.len(),
                "text": out_text
            })
            .to_string())
        }
    }
}

fn validate_url(raw: &str) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| e.to_string())?;
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!("only http/https allowed, got '{other}'")),
    }
}
