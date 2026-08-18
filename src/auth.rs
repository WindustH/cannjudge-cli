use crate::cdp::{CdpClient, fetch_targets, open_new_tab};
use crate::config;
use crate::util::{now_secs, read_json_file, write_json_0600};
use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{HeaderMap, SET_COOKIE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const CANNJUDGE_HOST_SUFFIX: &str = "cannjudge.cn";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cookie {
  pub name: String,
  pub value: String,
  pub domain: String,
  pub path: String,
  pub expires: Option<f64>,
  #[serde(default)]
  pub secure: bool,
  #[serde(rename = "httpOnly", default)]
  pub http_only: bool,
  #[serde(rename = "sameSite")]
  pub same_site: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Auth {
  pub user: Value,
  #[serde(default)]
  pub cookies: Vec<Cookie>,
  #[serde(default)]
  pub source: String,
  #[serde(default)]
  pub created: f64,
}

#[derive(Clone, Debug)]
pub struct LoginOptions {
  pub base_url: String,
  pub cdp_list_url: String,
  pub chrome_bin: String,
  pub chrome_profile: PathBuf,
  pub auth_cache: PathBuf,
  pub no_launch: bool,
  pub login_timeout: Duration,
  pub probe_interval: Duration,
  pub debug: bool,
}

impl Auth {
  pub fn load(path: &Path) -> Result<Self> {
    let auth: Auth = read_json_file(path)?;
    if !auth.valid() {
      bail!(
        "auth cache is missing a valid cannjudge_user: {}",
        path.display()
      );
    }
    Ok(auth)
  }

  pub fn save(&self, path: &Path) -> Result<()> {
    write_json_0600(path, self)
  }

  pub fn valid(&self) -> bool {
    !self.user_id().is_empty()
  }

  pub fn user_id(&self) -> String {
    normalize_object_id(self.user.get("_id").or_else(|| self.user.get("id")))
  }

  pub fn user_numeric_id(&self) -> Option<i64> {
    self.user.get("ID").and_then(Value::as_i64)
  }

  pub fn user_label(&self) -> String {
    self
      .user
      .get("nickname")
      .and_then(Value::as_str)
      .or_else(|| self.user.get("email").and_then(Value::as_str))
      .map(ToString::to_string)
      .unwrap_or_else(|| self.user_id())
  }

  pub fn cookie_header(&self, url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
      return String::new();
    };
    let host = parsed.host_str().unwrap_or("");
    let path = parsed.path();
    let mut selected: Vec<&Cookie> = self
      .cookies
      .iter()
      .filter(|cookie| {
        !cookie_expired(cookie, 0.0)
          && domain_matches(&cookie.domain, host)
          && path_matches(&cookie.path, path)
      })
      .collect();
    selected.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
    selected
      .into_iter()
      .map(|cookie| format!("{}={}", cookie.name, cookie.value))
      .collect::<Vec<_>>()
      .join("; ")
  }

  pub fn update_cookies(&mut self, cookies: Vec<Cookie>) {
    let mut by_key: HashMap<(String, String, String), Cookie> = HashMap::new();
    for cookie in self.cookies.drain(..) {
      by_key.insert(
        (
          cookie.domain.clone(),
          cookie.path.clone(),
          cookie.name.clone(),
        ),
        cookie,
      );
    }
    for cookie in cookies {
      if !is_cannjudge_domain(&cookie.domain) {
        continue;
      }
      let key = (
        cookie.domain.clone(),
        cookie.path.clone(),
        cookie.name.clone(),
      );
      if cookie_expired(&cookie, 0.0) {
        by_key.remove(&key);
      } else {
        by_key.insert(key, cookie);
      }
    }
    self.cookies = by_key.into_values().collect();
  }
}

pub fn normalize_object_id(value: Option<&Value>) -> String {
  let Some(value) = value else {
    return String::new();
  };
  if let Some(text) = value.as_str() {
    let text = text.trim();
    return if is_object_id(text) {
      text.to_string()
    } else {
      String::new()
    };
  }
  if let Some(nested) = value.get("$oid") {
    return normalize_object_id(Some(nested));
  }
  String::new()
}

pub fn is_object_id(value: &str) -> bool {
  value.len() == 24 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn is_cannjudge_domain(domain: &str) -> bool {
  let clean = domain.trim_start_matches('.').to_ascii_lowercase();
  clean == CANNJUDGE_HOST_SUFFIX || clean.ends_with(&format!(".{CANNJUDGE_HOST_SUFFIX}"))
}

pub fn domain_matches(cookie_domain: &str, host: &str) -> bool {
  let domain = cookie_domain.to_ascii_lowercase();
  let host = host.to_ascii_lowercase();
  if let Some(clean) = domain.strip_prefix('.') {
    host == clean || host.ends_with(&format!(".{clean}"))
  } else {
    host == domain
  }
}

pub fn path_matches(cookie_path: &str, request_path: &str) -> bool {
  let cookie_path = if cookie_path.is_empty() {
    "/"
  } else {
    cookie_path
  };
  if cookie_path == "/" {
    return true;
  }
  let cookie_path = cookie_path.trim_end_matches('/');
  request_path == cookie_path
    || request_path
      .strip_prefix(cookie_path)
      .is_some_and(|rest| rest.starts_with('/'))
}

pub fn cookie_expired(cookie: &Cookie, margin: f64) -> bool {
  match cookie.expires {
    None => false,
    Some(expires) if expires < 0.0 => false,
    Some(expires) => expires <= now_secs() + margin,
  }
}

fn normalize_cdp_cookie(value: &Value) -> Option<Cookie> {
  let name = value.get("name")?.as_str()?.to_string();
  let cookie_value = value.get("value")?.as_str()?.to_string();
  if name.is_empty() || cookie_value.is_empty() {
    return None;
  }
  Some(Cookie {
    name,
    value: cookie_value,
    domain: value
      .get("domain")
      .and_then(Value::as_str)
      .unwrap_or("")
      .to_string(),
    path: value
      .get("path")
      .and_then(Value::as_str)
      .unwrap_or("/")
      .to_string(),
    expires: value.get("expires").and_then(Value::as_f64),
    secure: value.get("secure").and_then(Value::as_bool).unwrap_or(true),
    http_only: value
      .get("httpOnly")
      .and_then(Value::as_bool)
      .unwrap_or(false),
    same_site: value
      .get("sameSite")
      .and_then(Value::as_str)
      .map(ToString::to_string),
  })
}

fn parse_set_cookie(url: &str, raw: &str) -> Option<Cookie> {
  let host = url::Url::parse(url).ok()?.host_str()?.to_string();
  let mut parts = raw.split(';').map(str::trim);
  let first = parts.next()?;
  let (name, value) = first.split_once('=')?;
  if name.trim().is_empty() {
    return None;
  }
  let mut cookie = Cookie {
    name: name.trim().to_string(),
    value: value.to_string(),
    domain: host,
    path: "/".to_string(),
    expires: Some(-1.0),
    secure: false,
    http_only: false,
    same_site: None,
  };
  for attr in parts {
    let (key, value) = attr.split_once('=').unwrap_or((attr, ""));
    match key.to_ascii_lowercase().as_str() {
      "domain" => cookie.domain = value.to_string(),
      "path" => {
        cookie.path = if value.is_empty() {
          "/".to_string()
        } else {
          value.to_string()
        }
      }
      "secure" => cookie.secure = true,
      "httponly" => cookie.http_only = true,
      "samesite" => cookie.same_site = Some(value.to_string()),
      "max-age" => {
        if let Ok(seconds) = value.parse::<f64>() {
          cookie.expires = Some(now_secs() + seconds);
        }
      }
      "expires" => {
        if value.eq_ignore_ascii_case("0") {
          cookie.expires = Some(0.0);
        }
      }
      _ => {}
    }
  }
  Some(cookie)
}

pub fn parse_set_cookie_headers(url: &str, headers: &HeaderMap) -> Vec<Cookie> {
  headers
    .get_all(SET_COOKIE)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .filter_map(|raw| parse_set_cookie(url, raw))
    .collect()
}

pub fn load_or_login(options: &LoginOptions, force_login: bool) -> Result<Auth> {
  if !force_login && let Ok(auth) = Auth::load(&options.auth_cache) {
    return Ok(auth);
  }
  login_with_browser(options)
}

pub fn login_with_browser(options: &LoginOptions) -> Result<Auth> {
  let login_url = format!("{}/auth/login", options.base_url.trim_end_matches('/'));
  let mut child = ensure_chrome(options, &login_url)?;
  let deadline = Instant::now() + options.login_timeout;
  let mut last_error = String::new();

  while Instant::now() < deadline {
    match extract_auth_from_cdp(&options.cdp_list_url, &options.base_url, options.debug) {
      Ok(auth) if auth.valid() => {
        auth.save(&options.auth_cache)?;
        return Ok(auth);
      }
      Ok(_) => last_error = "Chrome profile did not expose a valid CANNJudge user".to_string(),
      Err(err) => last_error = err.to_string(),
    }
    thread::sleep(options.probe_interval);
  }

  if let Some(child) = child.as_mut()
    && let Ok(Some(_)) = child.try_wait()
  {
    last_error = "Chrome exited before CANNJudge login completed".to_string();
  }
  bail!("login timed out: {last_error}");
}

fn ensure_chrome(options: &LoginOptions, login_url: &str) -> Result<Option<Child>> {
  if cdp_alive(&options.cdp_list_url) {
    let _ = open_new_tab(&options.cdp_list_url, login_url);
    return Ok(None);
  }

  if options.no_launch {
    let port = cdp_port(&options.cdp_list_url).unwrap_or(config::CDP_PORT_POOL_START);
    bail!(
      "Chrome DevTools is not available at {}; start Chrome with --remote-debugging-port={port} or omit --no-launch",
      options.cdp_list_url,
    );
  }

  std::fs::create_dir_all(&options.chrome_profile)
    .with_context(|| format!("create {}", options.chrome_profile.display()))?;
  let port = cdp_port(&options.cdp_list_url).unwrap_or(config::CDP_PORT_POOL_START);
  let child = Command::new(&options.chrome_bin)
    .arg(format!("--remote-debugging-port={port}"))
    .arg(format!(
      "--user-data-dir={}",
      options.chrome_profile.to_string_lossy()
    ))
    .arg("--no-first-run")
    .arg("--no-default-browser-check")
    .arg(login_url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .with_context(|| format!("launch {}", options.chrome_bin))?;

  let deadline = Instant::now() + Duration::from_secs(15);
  while Instant::now() < deadline {
    if cdp_alive(&options.cdp_list_url) {
      return Ok(Some(child));
    }
    thread::sleep(Duration::from_millis(250));
  }

  Ok(Some(child))
}

fn cdp_alive(cdp_list_url: &str) -> bool {
  fetch_targets(cdp_list_url).is_ok()
}

fn cdp_port(cdp_list_url: &str) -> Option<u16> {
  let parsed = url::Url::parse(cdp_list_url).ok()?;
  parsed.port()
}

pub fn extract_auth_from_cdp(cdp_list_url: &str, base_url: &str, debug: bool) -> Result<Auth> {
  let base_host = url::Url::parse(base_url)
    .ok()
    .and_then(|url| url.host_str().map(ToString::to_string))
    .unwrap_or_else(|| CANNJUDGE_HOST_SUFFIX.to_string());
  let mut pages: Vec<Value> = fetch_targets(cdp_list_url)?
    .into_iter()
    .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
    .collect();

  if !pages.iter().any(|target| {
    target
      .get("url")
      .and_then(Value::as_str)
      .is_some_and(|url| url.contains(&base_host))
  }) {
    let _ = open_new_tab(cdp_list_url, base_url);
    thread::sleep(Duration::from_secs(1));
    pages = fetch_targets(cdp_list_url)?
      .into_iter()
      .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
      .collect();
  }

  let target = pages
    .iter()
    .find(|target| {
      target
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|url| url.contains(&base_host))
    })
    .or_else(|| pages.first())
    .ok_or_else(|| anyhow!("no Chrome page is available for auth extraction"))?;

  let websocket_url = target
    .get("webSocketDebuggerUrl")
    .and_then(Value::as_str)
    .ok_or_else(|| anyhow!("Chrome target has no webSocketDebuggerUrl"))?;

  let mut cdp = CdpClient::connect(websocket_url, debug)?;
  let result = (|| {
    cdp.call("Page.enable", json!({}), Duration::from_secs(10))?;
    cdp.call("Network.enable", json!({}), Duration::from_secs(10))?;
    cdp.call("Runtime.enable", json!({}), Duration::from_secs(10))?;
    cdp.pump_for(Duration::from_secs(1))?;
    let user_raw = cdp.evaluate(
      r#"(() => {
                try { return localStorage.getItem('cannjudge_user') || ''; }
                catch (e) { return ''; }
            })()"#,
      5000,
    )?;
    let user_text = user_raw.as_str().unwrap_or("").trim();
    if user_text.is_empty() {
      bail!("localStorage.cannjudge_user is empty; finish login in Chrome");
    }
    let user: Value = serde_json::from_str(user_text).context("parse cannjudge_user")?;
    let cookies = cdp.call("Network.getAllCookies", json!({}), Duration::from_secs(10))?;
    let cannjudge_cookies: Vec<Cookie> = cookies
      .get("cookies")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(normalize_cdp_cookie)
      .filter(|cookie| is_cannjudge_domain(&cookie.domain))
      .collect();
    let auth = Auth {
      user,
      cookies: cannjudge_cookies,
      source: "chrome-cdp".to_string(),
      created: now_secs(),
    };
    if !auth.valid() {
      bail!("cannjudge_user does not contain a valid _id");
    }
    Ok(auth)
  })();
  cdp.close();
  result
}

impl Default for LoginOptions {
  fn default() -> Self {
    Self {
      base_url: config::DEFAULT_BASE_URL.to_string(),
      cdp_list_url: config::default_cdp_list_url(),
      chrome_bin: config::default_chrome_bin(),
      chrome_profile: config::expand_tilde(config::default_chrome_profile()),
      auth_cache: config::expand_tilde(config::default_auth_cache()),
      no_launch: false,
      login_timeout: Duration::from_secs(300),
      probe_interval: Duration::from_secs(2),
      debug: false,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{domain_matches, path_matches};

  #[test]
  fn cookie_path_requires_a_segment_boundary() {
    assert!(path_matches("/api", "/api"));
    assert!(path_matches("/api", "/api/items"));
    assert!(!path_matches("/api", "/apix"));
  }

  #[test]
  fn cookie_domain_matching_is_subdomain_aware() {
    assert!(domain_matches(".cannjudge.cn", "api.cannjudge.cn"));
    assert!(!domain_matches(
      ".cannjudge.cn",
      "cannjudge.cn.evil.example"
    ));
  }
}
