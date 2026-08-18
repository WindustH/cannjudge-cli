use crate::auth::{Auth, parse_set_cookie_headers};
use crate::util::{now_secs, read_json_file, write_json_0600};
use anyhow::{Context, Result};
use reqwest::Method;
use reqwest::blocking::{Client as HttpClient, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug)]
pub struct ApiError {
  pub status: u16,
  pub message: String,
  pub payload: Value,
}

impl std::fmt::Display for ApiError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "HTTP {}: {}", self.status, self.message)
  }
}

impl std::error::Error for ApiError {}

pub struct ApiClient {
  pub base_url: String,
  http: HttpClient,
  pub auth: Option<Auth>,
  auth_cache: Option<PathBuf>,
  cache: Option<CacheConfig>,
}

#[derive(Clone, Debug)]
pub struct CacheConfig {
  pub dir: PathBuf,
  pub refresh: bool,
  pub ttl: Duration,
}

#[derive(Clone, Debug)]
struct CacheCandidate {
  path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedJson {
  url: String,
  user_id: Option<String>,
  created: f64,
  value: Value,
}

impl ApiClient {
  pub fn new(
    base_url: impl Into<String>,
    auth: Option<Auth>,
    auth_cache: Option<PathBuf>,
    cache: Option<CacheConfig>,
  ) -> Result<Self> {
    let http = HttpClient::builder()
      .timeout(Duration::from_secs(120))
      .user_agent("cannjudge-cli/0.1")
      .build()?;
    Ok(Self {
      base_url: base_url.into().trim_end_matches('/').to_string(),
      http,
      auth,
      auth_cache,
      cache,
    })
  }

  pub fn user_id(&self) -> Option<String> {
    self
      .auth
      .as_ref()
      .map(Auth::user_id)
      .filter(|id| !id.is_empty())
  }

  pub fn get_json(&mut self, path: &str, query: &[(&str, String)]) -> Result<Value> {
    self.request_json(Method::GET, path, query, None)
  }

  pub fn get_json_fresh(&mut self, path: &str, query: &[(&str, String)]) -> Result<Value> {
    self.request_json_inner(Method::GET, path, query, None, false)
  }

  pub fn post_json(&mut self, path: &str, data: Value) -> Result<Value> {
    self.request_json(Method::POST, path, &[], Some(data))
  }

  pub fn request_json(
    &mut self,
    method: Method,
    path: &str,
    query: &[(&str, String)],
    data: Option<Value>,
  ) -> Result<Value> {
    self.request_json_inner(method, path, query, data, true)
  }

  fn request_json_inner(
    &mut self,
    method: Method,
    path: &str,
    query: &[(&str, String)],
    data: Option<Value>,
    use_cache: bool,
  ) -> Result<Value> {
    let url = self.url(path, query)?;
    let cache = use_cache
      .then(|| self.cache_candidate(&method, &url, data.is_none()))
      .flatten();
    if let Some(cache) = &cache
      && let Some(value) = self.read_cached_json(cache)?
    {
      return Ok(value);
    }
    let mut req = self
      .http
      .request(method, &url)
      .header(ACCEPT, "application/json");
    if let Some(auth) = &self.auth {
      let cookie_header = auth.cookie_header(&url);
      if !cookie_header.is_empty() {
        req = req.header(COOKIE, cookie_header);
      }
    }
    if let Some(data) = data {
      req = req
        .header(CONTENT_TYPE, "application/json")
        .body(data.to_string());
    }
    let response = req.send().with_context(|| format!("request {url}"))?;
    let value = self.handle_json_response(url.clone(), response)?;
    if let Some(cache) = &cache
      && cacheable_json_response(&url, &value)
    {
      self.write_cached_json(cache, &url, &value)?;
    }
    Ok(value)
  }

  pub fn download(&mut self, path: &str, query: &[(&str, String)]) -> Result<Download> {
    let url = self.url(path, query)?;
    let mut req = self.http.get(&url);
    if let Some(auth) = &self.auth {
      let cookie_header = auth.cookie_header(&url);
      if !cookie_header.is_empty() {
        req = req.header(COOKIE, cookie_header);
      }
    }
    let response = req.send().with_context(|| format!("GET {url}"))?;
    if !response.status().is_success() {
      return Err(self.response_error(url, response).into());
    }
    let headers = response.headers().clone();
    self.update_cookies(&url, &headers)?;
    let file_name = parse_download_filename(
      headers
        .get("Content-Disposition")
        .and_then(|v| v.to_str().ok()),
    )
    .unwrap_or_else(|| "download.zip".to_string());
    let bytes = response.bytes()?.to_vec();
    Ok(Download { file_name, bytes })
  }

  fn handle_json_response(&mut self, url: String, response: Response) -> Result<Value> {
    if !response.status().is_success() {
      return Err(self.response_error(url, response).into());
    }
    let headers = response.headers().clone();
    self.update_cookies(&url, &headers)?;
    let text = response.text()?;
    if text.trim().is_empty() {
      return Ok(json!({}));
    }
    serde_json::from_str(&text).with_context(|| {
      format!(
        "parse JSON from {url}: {}",
        text.chars().take(200).collect::<String>()
      )
    })
  }

  fn response_error(&mut self, url: String, response: Response) -> ApiError {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let _ = self.update_cookies(&url, &headers);
    let text = response.text().unwrap_or_default();
    let payload: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({"message": text}));
    let message =
      api_error_message(&payload).unwrap_or_else(|| format!("request failed ({status})"));
    ApiError {
      status,
      message,
      payload,
    }
  }

  fn update_cookies(&mut self, url: &str, headers: &HeaderMap) -> Result<()> {
    let cookies = parse_set_cookie_headers(url, headers);
    if cookies.is_empty() {
      return Ok(());
    }
    if let Some(auth) = &mut self.auth {
      auth.update_cookies(cookies);
      if let Some(path) = &self.auth_cache {
        auth.save(path)?;
      }
    }
    Ok(())
  }

  fn url(&self, path: &str, query: &[(&str, String)]) -> Result<String> {
    let mut url = if path.starts_with("http://") || path.starts_with("https://") {
      url::Url::parse(path)?
    } else {
      let path = if path.starts_with('/') {
        path.to_string()
      } else {
        format!("/{path}")
      };
      url::Url::parse(&format!("{}{}", self.base_url, path))?
    };
    if !query.is_empty() {
      let mut pairs = url.query_pairs_mut();
      for (key, value) in query {
        if !value.is_empty() {
          pairs.append_pair(key, value);
        }
      }
    }
    Ok(url.to_string())
  }

  fn cache_candidate(&self, method: &Method, url: &str, bodyless: bool) -> Option<CacheCandidate> {
    let cache = self.cache.as_ref()?;
    if *method != Method::GET || !bodyless {
      return None;
    }
    let user_id = self.user_id();
    let key = cache_key(url, user_id.as_deref());
    let slug = cache_slug(url);
    Some(CacheCandidate {
      path: cache.dir.join(format!("{slug}-{key}.json")),
    })
  }

  fn read_cached_json(&self, candidate: &CacheCandidate) -> Result<Option<Value>> {
    let Some(cache) = &self.cache else {
      return Ok(None);
    };
    if cache.refresh || !candidate.path.exists() {
      return Ok(None);
    }
    let Ok(entry) = read_json_file::<CachedJson>(&candidate.path) else {
      return Ok(None);
    };
    if cache.ttl.as_secs() > 0 {
      let age = now_secs() - entry.created;
      if age > cache.ttl.as_secs_f64() {
        return Ok(None);
      }
    }
    Ok(Some(entry.value))
  }

  fn write_cached_json(&self, candidate: &CacheCandidate, url: &str, value: &Value) -> Result<()> {
    let entry = CachedJson {
      url: url.to_string(),
      user_id: self.user_id(),
      created: now_secs(),
      value: value.clone(),
    };
    write_json_0600(&candidate.path, &entry)
  }
}

pub struct Download {
  pub file_name: String,
  pub bytes: Vec<u8>,
}

fn api_error_message(payload: &Value) -> Option<String> {
  for pointer in [
    "/message",
    "/msg",
    "/error",
    "/detail",
    "/data/message",
    "/data/msg",
    "/data/error",
    "/data/detail",
  ] {
    if let Some(text) = payload.pointer(pointer).and_then(Value::as_str)
      && !text.trim().is_empty()
    {
      return Some(text.trim().to_string());
    }
  }
  None
}

fn parse_download_filename(disposition: Option<&str>) -> Option<String> {
  let raw = disposition?.trim();
  if raw.is_empty() {
    return None;
  }
  let re_utf8 = regex::Regex::new(r#"(?i)filename\*\s*=\s*UTF-8''([^;]+)"#).ok()?;
  if let Some(captures) = re_utf8.captures(raw)
    && let Some(value) = captures.get(1)
  {
    return urlencoding::decode(value.as_str())
      .ok()
      .and_then(|value| safe_download_filename(&value));
  }
  let re_quoted = regex::Regex::new(r#"(?i)filename\s*=\s*"([^"]+)""#).ok()?;
  if let Some(captures) = re_quoted.captures(raw)
    && let Some(value) = captures.get(1)
  {
    return safe_download_filename(value.as_str());
  }
  let re_plain = regex::Regex::new(r#"(?i)filename\s*=\s*([^;]+)"#).ok()?;
  re_plain
    .captures(raw)
    .and_then(|captures| captures.get(1))
    .and_then(|value| safe_download_filename(value.as_str().trim()))
}

fn safe_download_filename(value: &str) -> Option<String> {
  let normalized = value.replace('\\', "/");
  let name = std::path::Path::new(&normalized)
    .file_name()
    .and_then(|name| name.to_str())?
    .trim();
  if name.is_empty() || name == "." || name == ".." || name.contains('\0') {
    None
  } else {
    Some(name.to_string())
  }
}

pub fn downcast_api_error(error: &anyhow::Error) -> Option<&ApiError> {
  error.downcast_ref::<ApiError>()
}

pub fn ensure_auth(client: &ApiClient) -> Result<String> {
  client
    .user_id()
    .filter(|id| !id.is_empty())
    .ok_or_else(|| anyhow::anyhow!("login required; run `cannjudge auth login` first"))
}

fn cache_key(url: &str, user_id: Option<&str>) -> String {
  let mut hasher = DefaultHasher::new();
  url.hash(&mut hasher);
  user_id.hash(&mut hasher);
  format!("{:016x}", hasher.finish())
}

fn cache_slug(url: &str) -> String {
  let path = url::Url::parse(url)
    .ok()
    .map(|url| url.path().trim_matches('/').to_string())
    .unwrap_or_else(|| "api".to_string());
  let mut slug = path
    .chars()
    .map(|ch| {
      if ch.is_ascii_alphanumeric() {
        ch.to_ascii_lowercase()
      } else {
        '_'
      }
    })
    .collect::<String>();
  while slug.contains("__") {
    slug = slug.replace("__", "_");
  }
  let slug = slug.trim_matches('_');
  if slug.is_empty() {
    "api".to_string()
  } else {
    slug.chars().take(80).collect()
  }
}

fn cacheable_json_response(url: &str, value: &Value) -> bool {
  let path = url::Url::parse(url)
    .ok()
    .map(|url| url.path().to_string())
    .unwrap_or_default();
  if path.starts_with("/api/submissions/") && !path.ends_with("/package") {
    let status = value
      .get("status")
      .and_then(Value::as_str)
      .unwrap_or("")
      .trim()
      .to_ascii_lowercase();
    return is_terminal_status(&status);
  }
  true
}

fn is_terminal_status(status: &str) -> bool {
  !(status.is_empty()
    || status == "running"
    || status == "pending"
    || status == "queued"
    || status == "judging"
    || status == "compiling")
}

#[cfg(test)]
mod tests {
  use super::parse_download_filename;

  #[test]
  fn download_filename_cannot_escape_destination() {
    assert_eq!(
      parse_download_filename(Some(r#"attachment; filename="../../result.zip""#)),
      Some("result.zip".to_string())
    );
    assert_eq!(
      parse_download_filename(Some(
        r#"attachment; filename*=UTF-8''%E6%B5%8B%E8%AF%95.zip"#
      )),
      Some("测试.zip".to_string())
    );
  }
}
