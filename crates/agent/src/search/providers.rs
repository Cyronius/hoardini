use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::{Kind, SearchResult, clip};

pub enum Method {
    Get,
    Post,
}

pub struct HttpReq {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub json: Option<Value>,
}

pub trait SearchProvider: Send + Sync {
    fn kind(&self) -> Kind;
    fn request(&self, key: Option<&str>, query: &str, limit: usize) -> Result<HttpReq>;
    fn parse(&self, body: &str) -> Result<Vec<SearchResult>>;
}

pub struct Tavily;
pub struct Brave;
pub struct Exa;
pub struct Serper;
pub struct SearxNg {
    pub base_url: String,
}

fn need_key(key: Option<&str>, who: &str) -> Result<String> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => Ok(k.to_string()),
        None => bail!("{who} needs an API key"),
    }
}

fn with_params(base: &str, params: &[(&str, &str)]) -> Result<String> {
    Ok(reqwest::Url::parse_with_params(base, params)?.to_string())
}

fn str_field<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).and_then(Value::as_str).unwrap_or("")
}

fn collect(
    items: &[Value],
    url_field: &str,
    snippet: impl Fn(&Value) -> String,
) -> Vec<SearchResult> {
    items
        .iter()
        .filter_map(|it| {
            let url = str_field(it, url_field);
            if url.is_empty() {
                return None;
            }
            Some(SearchResult {
                title: clip(str_field(it, "title"), 200),
                url: url.to_string(),
                snippet: clip(&snippet(it), super::SNIPPET_MAX),
            })
        })
        .collect()
}

fn json_body(body: &str, who: &str) -> Result<Value> {
    serde_json::from_str(body).map_err(|e| anyhow!("{who} returned an unreadable response: {e}"))
}

fn array<'a>(v: &'a Value, path: &[&str], who: &str) -> Result<&'a [Value]> {
    let mut cur = v;
    for p in path {
        cur = cur
            .get(p)
            .ok_or_else(|| anyhow!("{who} response has no `{}` field", path.join(".")))?;
    }
    cur.as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| anyhow!("{who} response `{}` is not a list", path.join(".")))
}

impl SearchProvider for Tavily {
    fn kind(&self) -> Kind {
        Kind::Tavily
    }
    fn request(&self, key: Option<&str>, query: &str, limit: usize) -> Result<HttpReq> {
        let key = need_key(key, "Tavily")?;
        Ok(HttpReq {
            method: Method::Post,
            url: "https://api.tavily.com/search".into(),
            headers: vec![("Authorization", format!("Bearer {key}"))],
            json: Some(json!({ "query": query, "max_results": limit, "search_depth": "basic" })),
        })
    }
    fn parse(&self, body: &str) -> Result<Vec<SearchResult>> {
        let v = json_body(body, "Tavily")?;
        Ok(collect(array(&v, &["results"], "Tavily")?, "url", |it| {
            str_field(it, "content").to_string()
        }))
    }
}

impl SearchProvider for Brave {
    fn kind(&self) -> Kind {
        Kind::Brave
    }
    fn request(&self, key: Option<&str>, query: &str, limit: usize) -> Result<HttpReq> {
        let key = need_key(key, "Brave Search")?;
        Ok(HttpReq {
            method: Method::Get,
            url: with_params(
                "https://api.search.brave.com/res/v1/web/search",
                &[("q", query), ("count", &limit.to_string())],
            )?,
            headers: vec![
                ("X-Subscription-Token", key),
                ("Accept", "application/json".into()),
            ],
            json: None,
        })
    }
    fn parse(&self, body: &str) -> Result<Vec<SearchResult>> {
        let v = json_body(body, "Brave Search")?;
        // Brave omits `web` entirely when nothing matches.
        let Some(web) = v.get("web") else {
            return Ok(Vec::new());
        };
        Ok(collect(
            array(web, &["results"], "Brave Search")?,
            "url",
            |it| str_field(it, "description").to_string(),
        ))
    }
}

impl SearchProvider for Exa {
    fn kind(&self) -> Kind {
        Kind::Exa
    }
    fn request(&self, key: Option<&str>, query: &str, limit: usize) -> Result<HttpReq> {
        let key = need_key(key, "Exa")?;
        Ok(HttpReq {
            method: Method::Post,
            url: "https://api.exa.ai/search".into(),
            headers: vec![("x-api-key", key)],
            json: Some(json!({
                "query": query,
                "numResults": limit,
                "contents": { "text": { "maxCharacters": 500 } }
            })),
        })
    }
    fn parse(&self, body: &str) -> Result<Vec<SearchResult>> {
        let v = json_body(body, "Exa")?;
        Ok(collect(array(&v, &["results"], "Exa")?, "url", |it| {
            let text = str_field(it, "text");
            if !text.is_empty() {
                return text.to_string();
            }
            it.get("highlights")
                .and_then(Value::as_array)
                .map(|h| {
                    h.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default()
        }))
    }
}

impl SearchProvider for Serper {
    fn kind(&self) -> Kind {
        Kind::Serper
    }
    fn request(&self, key: Option<&str>, query: &str, limit: usize) -> Result<HttpReq> {
        let key = need_key(key, "Serper")?;
        Ok(HttpReq {
            method: Method::Post,
            url: "https://google.serper.dev/search".into(),
            headers: vec![("X-API-KEY", key)],
            json: Some(json!({ "q": query, "num": limit })),
        })
    }
    fn parse(&self, body: &str) -> Result<Vec<SearchResult>> {
        let v = json_body(body, "Serper")?;
        // Serper omits `organic` when Google returns only an answer box or nothing.
        let Some(items) = v.get("organic").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        Ok(collect(items, "link", |it| {
            str_field(it, "snippet").to_string()
        }))
    }
}

impl SearchProvider for SearxNg {
    fn kind(&self) -> Kind {
        Kind::Searxng
    }
    fn request(&self, key: Option<&str>, query: &str, _limit: usize) -> Result<HttpReq> {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            bail!("SearXNG needs a base URL");
        }
        let mut headers = vec![("Accept", "application/json".to_string())];
        if let Some(k) = key.map(str::trim).filter(|k| !k.is_empty()) {
            headers.push(("Authorization", format!("Bearer {k}")));
        }
        Ok(HttpReq {
            method: Method::Get,
            url: with_params(
                &format!("{base}/search"),
                &[("q", query), ("format", "json")],
            )?,
            headers,
            json: None,
        })
    }
    fn parse(&self, body: &str) -> Result<Vec<SearchResult>> {
        let v: Value = serde_json::from_str(body).map_err(|_| {
            anyhow!(
                "SearXNG did not return JSON; add `json` to `search.formats` in its settings.yml"
            )
        })?;
        Ok(collect(array(&v, &["results"], "SearXNG")?, "url", |it| {
            str_field(it, "content").to_string()
        }))
    }
}
