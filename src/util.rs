use anyhow::{Context, Result, bail};
use rand::RngCore;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> f64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_secs_f64())
    .unwrap_or(0.0)
}

pub fn token_hex(bytes: usize) -> String {
  let mut data = vec![0u8; bytes];
  rand::thread_rng().fill_bytes(&mut data);
  data.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
  let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
  serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

pub fn write_json_0600<T: Serialize>(path: &Path, value: &T) -> Result<()> {
  let text = serde_json::to_string_pretty(value)? + "\n";
  write_atomic_0600(path, text.as_bytes())
}

pub fn write_atomic_0600(path: &Path, bytes: &[u8]) -> Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
  }
  let tmp = path.with_file_name(format!(
    "{}.tmp-{}",
    path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp"),
    token_hex(4)
  ));
  {
    let mut file = fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.flush()?;
  }
  fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).ok();
  fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
  fs::set_permissions(path, fs::Permissions::from_mode(0o600)).ok();
  Ok(())
}

pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
  let normalized = rel.replace('\\', "/");
  let rel_path = Path::new(&normalized);
  if rel_path.is_absolute() {
    bail!("template path is absolute: {rel}");
  }

  let mut out = PathBuf::from(base);
  for component in rel_path.components() {
    match component {
      Component::Normal(part) => out.push(part),
      Component::CurDir => {}
      Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
        bail!("template path escapes target dir: {rel}");
      }
    }
  }
  Ok(out)
}

pub fn value_string(value: &serde_json::Value, keys: &[&str]) -> String {
  for key in keys {
    if let Some(text) = value.get(*key).and_then(serde_json::Value::as_str)
      && !text.trim().is_empty()
    {
      return text.trim().to_string();
    }
    if let Some(number) = value.get(*key).and_then(serde_json::Value::as_i64) {
      return number.to_string();
    }
  }
  String::new()
}

pub fn object_id(value: &serde_json::Value) -> String {
  value_string(value, &["_id", "id"])
}

pub fn truncate(value: &str, max: usize) -> String {
  if value.chars().count() <= max {
    return value.to_string();
  }
  let mut out = value
    .chars()
    .take(max.saturating_sub(1))
    .collect::<String>();
  out.push('…');
  out
}

pub fn print_json(value: &serde_json::Value) -> Result<()> {
  println!("{}", serde_json::to_string_pretty(value)?);
  Ok(())
}
