use anyhow::{Result, bail};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{ErrorKind, Write};
use std::net::TcpListener;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "cannjudge-cli";
pub const DEFAULT_BASE_URL: &str = "https://cannjudge.cn";
pub const DEFAULT_ACCOUNT: &str = "default";
pub const CDP_PORT_POOL_START: u16 = 61600;
pub const CDP_PORT_POOL_END: u16 = 61799;

pub fn default_account() -> String {
  env::var("CANNJUDGE_ACCOUNT").unwrap_or_else(|_| DEFAULT_ACCOUNT.to_string())
}

pub fn validate_account_name(value: &str) -> Result<String> {
  let value = value.trim();
  if value.is_empty() || value == "." || value == ".." || value.len() > 64 {
    bail!("invalid account name: {value:?}");
  }
  if !value
    .bytes()
    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
  {
    bail!("invalid account name {value:?}; use only ASCII letters, digits, '-', '_' or '.'");
  }
  Ok(value.to_string())
}

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
  account_auth_cache(&default_account())
    .unwrap_or_else(|_| PathBuf::from(default_config_dir()).join("auth.json"))
    .to_string_lossy()
    .into_owned()
}

pub fn default_state_file() -> String {
  account_state_file(&default_account())
    .unwrap_or_else(|_| PathBuf::from(default_config_dir()).join("state.json"))
    .to_string_lossy()
    .into_owned()
}

pub fn default_cache_dir() -> String {
  if let Ok(value) = env::var("CANNJUDGE_CACHE_DIR")
    && !value.is_empty()
  {
    return value;
  }
  if let Ok(value) = env::var("XDG_CACHE_HOME")
    && !value.is_empty()
  {
    return PathBuf::from(value)
      .join(APP_NAME)
      .to_string_lossy()
      .into_owned();
  }
  home_dir()
    .join(".cache")
    .join(APP_NAME)
    .to_string_lossy()
    .into_owned()
}

pub fn default_chrome_profile() -> String {
  account_chrome_profile(&default_account())
    .unwrap_or_else(|_| PathBuf::from(default_cache_dir()).join("chrome-profile"))
    .to_string_lossy()
    .into_owned()
}

fn accounts_dir() -> PathBuf {
  PathBuf::from(default_config_dir()).join("accounts")
}

pub fn account_auth_cache(account: &str) -> Result<PathBuf> {
  let account = validate_account_name(account)?;
  if account == DEFAULT_ACCOUNT {
    Ok(PathBuf::from(default_config_dir()).join("auth.json"))
  } else {
    Ok(accounts_dir().join(format!("{account}.json")))
  }
}

pub fn account_state_file(account: &str) -> Result<PathBuf> {
  let account = validate_account_name(account)?;
  if account == DEFAULT_ACCOUNT {
    Ok(PathBuf::from(default_config_dir()).join("state.json"))
  } else {
    Ok(accounts_dir().join(account).join("state.json"))
  }
}

pub fn account_chrome_profile(account: &str) -> Result<PathBuf> {
  let account = validate_account_name(account)?;
  let cache_dir = PathBuf::from(default_cache_dir());
  if account == DEFAULT_ACCOUNT {
    Ok(cache_dir.join("chrome-profile"))
  } else {
    Ok(
      cache_dir
        .join("accounts")
        .join(account)
        .join("chrome-profile"),
    )
  }
}

pub fn account_cdp_list_url(account: &str) -> Result<String> {
  let account = validate_account_name(account)?;
  if let Ok(value) = env::var("CANNJUDGE_CDP_LIST_URL")
    && !value.is_empty()
  {
    return Ok(value);
  }
  let port = account_cdp_port(&account)?;
  Ok(format!("http://127.0.0.1:{port}/json"))
}

fn account_cdp_port(account: &str) -> Result<u16> {
  let path = accounts_dir().join(account).join("cdp-port");
  if let Some(port) = read_port_file(&path, CDP_PORT_POOL_START..=CDP_PORT_POOL_END) {
    return Ok(port);
  }
  let port = find_available_port(CDP_PORT_POOL_START..=CDP_PORT_POOL_END, account)?;
  write_port_file(&path, port)?;
  Ok(read_port_file(&path, CDP_PORT_POOL_START..=CDP_PORT_POOL_END).unwrap_or(port))
}

fn read_port_file(path: &Path, range: RangeInclusive<u16>) -> Option<u16> {
  let port = fs::read_to_string(path).ok()?.trim().parse().ok()?;
  range.contains(&port).then_some(port)
}

fn write_port_file(path: &Path, port: u16) -> Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  match fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(path)
  {
    Ok(mut file) => {
      writeln!(file, "{port}")?;
      file.sync_all()?;
    }
    Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
    Err(err) => return Err(err.into()),
  }
  Ok(())
}

fn find_available_port(range: RangeInclusive<u16>, account: &str) -> Result<u16> {
  let start = *range.start();
  let len = range.clone().count();
  let mut hasher = std::collections::hash_map::DefaultHasher::new();
  account.hash(&mut hasher);
  let offset = (hasher.finish() as usize) % len;
  for index in 0..len {
    let port = start + ((offset + index) % len) as u16;
    if TcpListener::bind(("127.0.0.1", port)).is_ok() {
      return Ok(port);
    }
  }
  anyhow::bail!("no free CANNJudge Chrome DevTools port in the configured pool")
}

pub fn list_accounts() -> Vec<String> {
  let mut accounts = vec![DEFAULT_ACCOUNT.to_string()];
  if let Ok(entries) = fs::read_dir(accounts_dir()) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        continue;
      }
      if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        && validate_account_name(stem).is_ok()
        && stem != DEFAULT_ACCOUNT
      {
        accounts.push(stem.to_string());
      }
    }
  }
  accounts.sort();
  accounts.dedup();
  accounts
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
  account_cdp_list_url(&default_account())
    .unwrap_or_else(|_| format!("http://127.0.0.1:{}/json", CDP_PORT_POOL_START))
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
  if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
    return PathBuf::from(home);
  }
  if let Some(home) = passwd_home_dir() {
    return home;
  }
  PathBuf::from("/")
}

/// Resolve the home directory from passwd when HOME is unset or empty
/// (e.g. under systemd or stripped environments), so config/cache paths
/// never resolve relative to the current working directory.
fn passwd_home_dir() -> Option<PathBuf> {
  unsafe {
    let pw = libc::getpwuid(libc::getuid());
    if pw.is_null() {
      return None;
    }
    let dir = std::ffi::CStr::from_ptr((*pw).pw_dir)
      .to_string_lossy()
      .into_owned();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
  }
}
