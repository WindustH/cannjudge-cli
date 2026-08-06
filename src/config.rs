use std::env;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "cannjudge-cli";
pub const DEFAULT_BASE_URL: &str = "https://cannjudge.cn";
pub const DEFAULT_CDP_LIST_URL: &str = "http://127.0.0.1:9222/json";

pub fn default_config_dir() -> String {
  for key in ["CANNJUDGE_CONFIG_DIR"] {
    if let Ok(value) = env::var(key)
      && !value.is_empty()
    {
      return value;
    }
  }

  if let Ok(value) = env::var("XDG_CONFIG_HOME")
    && !value.is_empty()
  {
    return PathBuf::from(value)
      .join(APP_NAME)
      .to_string_lossy()
      .into_owned();
  }

  home_dir()
    .join(".config")
    .join(APP_NAME)
    .to_string_lossy()
    .into_owned()
}

pub fn default_auth_cache() -> String {
  PathBuf::from(default_config_dir())
    .join("auth.json")
    .to_string_lossy()
    .into_owned()
}

pub fn default_state_file() -> String {
  PathBuf::from(default_config_dir())
    .join("state.json")
    .to_string_lossy()
    .into_owned()
}

pub fn default_cache_dir() -> String {
  env::var("CANNJUDGE_CACHE_DIR").unwrap_or_else(|_| ".cache/cannjudge-cli".to_string())
}

pub fn default_chrome_profile() -> String {
  PathBuf::from(default_config_dir())
    .join("chrome-profile")
    .to_string_lossy()
    .into_owned()
}

pub fn default_chrome_bin() -> String {
  if let Ok(value) = env::var("CHROME")
    && !value.is_empty()
  {
    return value;
  }
  if Path::new("/opt/google/chrome/google-chrome").exists() {
    return "/opt/google/chrome/google-chrome".to_string();
  }
  "google-chrome-stable".to_string()
}

pub fn default_cdp_list_url() -> String {
  env::var("CANNJUDGE_CDP_LIST_URL").unwrap_or_else(|_| DEFAULT_CDP_LIST_URL.to_string())
}

pub fn expand_tilde(path: impl AsRef<str>) -> PathBuf {
  let path = path.as_ref();
  if path == "~" {
    return home_dir();
  }
  if let Some(rest) = path.strip_prefix("~/") {
    return home_dir().join(rest);
  }
  PathBuf::from(path)
}

pub fn home_dir() -> PathBuf {
  env::var_os("HOME")
    .filter(|value| !value.is_empty())
    .map(PathBuf::from)
    .or_else(|| env::current_dir().ok())
    .unwrap_or_else(|| PathBuf::from("."))
}
