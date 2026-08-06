use crate::auth::is_object_id;
use crate::client::{ApiClient, downcast_api_error, ensure_auth};
use crate::util::{object_id, safe_join, value_string};
use anyhow::{Context, Result, anyhow, bail};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProblemRef {
  pub group: Value,
  pub contest: Value,
  pub problem: Value,
  pub group_id: String,
  pub contest_id: String,
  pub problem_id: String,
  pub problem_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContestRef {
  pub group: Value,
  pub contest: Value,
  pub group_id: String,
  pub contest_id: String,
  pub contest_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectFile {
  pub path: String,
  pub content: String,
  pub editable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {
  pub problem_id: String,
  pub raw: Value,
  pub files: Vec<ProjectFile>,
}

#[derive(Clone, Debug)]
pub struct SubmitOptions {
  pub local_dir: PathBuf,
  pub allow_missing: bool,
  pub queue: bool,
  pub max_wait: Duration,
  pub poll_interval: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitOutcome {
  pub submission_id: String,
  pub response: Value,
  pub queued_wait_seconds: u64,
}

pub fn resolve_problem(client: &mut ApiClient, input: &str) -> Result<ProblemRef> {
  let spec = parse_problem_spec(input)?;
  let group = match spec.group.as_deref() {
    Some("public") | None if spec.public => client.get_json("/api/groups/public", &[])?,
    Some(group) if is_object_id(group) => client.get_json(&format!("/api/groups/{group}"), &[])?,
    Some(group) => client.get_json(&format!("/api/groups/name/{}", enc(group)), &[])?,
    None => json!({}),
  };
  let group_id = object_id(&group);

  let contest = if let Some(contest) = spec.contest.as_deref() {
    if is_object_id(contest) {
      client.get_json(&format!("/api/contests/{contest}"), &[])?
    } else {
      client.get_json(
        &format!("/api/contests/name/{}", enc(contest)),
        &[("groupId", group_id.clone())],
      )?
    }
  } else {
    json!({})
  };
  let contest_id = object_id(&contest);

  let problem = if is_object_id(&spec.problem) {
    client.get_json(
      &format!("/api/problems/{}", spec.problem),
      &user_query(client),
    )?
  } else {
    client.get_json(
      &format!("/api/problems/name/{}", enc(&spec.problem)),
      &[("contestId", contest_id.clone())],
    )?
  };
  let problem_id = object_id(&problem);
  if problem_id.is_empty() {
    bail!("resolved problem has no _id");
  }

  let problem_url = build_problem_url(client, &group, &contest, &problem);

  Ok(ProblemRef {
    group,
    contest,
    problem,
    group_id,
    contest_id,
    problem_id,
    problem_url,
  })
}

pub fn resolve_contest(client: &mut ApiClient, input: &str) -> Result<ContestRef> {
  let spec = parse_contest_spec(input)?;
  let group = match spec.group.as_deref() {
    Some("public") | None if spec.public => client.get_json("/api/groups/public", &[])?,
    Some(group) if is_object_id(group) => client.get_json(&format!("/api/groups/{group}"), &[])?,
    Some(group) => client.get_json(&format!("/api/groups/name/{}", enc(group)), &[])?,
    None => json!({}),
  };
  let mut group_id = object_id(&group);

  let contest = if is_object_id(&spec.contest) {
    client.get_json(&format!("/api/contests/{}", spec.contest), &[])?
  } else {
    client.get_json(
      &format!("/api/contests/name/{}", enc(&spec.contest)),
      &[("groupId", group_id.clone())],
    )?
  };
  if group_id.is_empty() {
    group_id = value_string(&contest, &["group_id"]);
  }
  let contest_id = object_id(&contest);
  if contest_id.is_empty() {
    bail!("resolved contest has no _id");
  }

  let contest_url = build_contest_url(client, &group, &contest);

  Ok(ContestRef {
    group,
    contest,
    group_id,
    contest_id,
    contest_url,
  })
}

#[derive(Debug)]
struct ProblemSpec {
  public: bool,
  group: Option<String>,
  contest: Option<String>,
  problem: String,
}

#[derive(Debug)]
struct ContestSpec {
  public: bool,
  group: Option<String>,
  contest: String,
}

fn parse_problem_spec(input: &str) -> Result<ProblemSpec> {
  let input = input.trim();
  if input.is_empty() {
    bail!("problem URL or id is empty");
  }
  if is_object_id(input) {
    return Ok(ProblemSpec {
      public: false,
      group: None,
      contest: None,
      problem: input.to_string(),
    });
  }

  let parsed = if input.starts_with("http://") || input.starts_with("https://") {
    url::Url::parse(input)?
  } else {
    url::Url::parse(&format!(
      "https://cannjudge.cn{}",
      ensure_leading_slash(input)
    ))?
  };
  let parts: Vec<String> = parsed
    .path_segments()
    .into_iter()
    .flatten()
    .map(|seg| {
      urlencoding::decode(seg)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| seg.to_string())
    })
    .filter(|seg| !seg.is_empty())
    .collect();

  if parts.len() >= 2 && parts[0] == "problem" {
    return Ok(ProblemSpec {
      public: false,
      group: None,
      contest: None,
      problem: parts[1].clone(),
    });
  }

  if parts.len() >= 3 && parts[0] == "public" {
    return Ok(ProblemSpec {
      public: true,
      group: Some("public".to_string()),
      contest: Some(parts[1].clone()),
      problem: parts[2].clone(),
    });
  }

  if parts.len() >= 3 {
    return Ok(ProblemSpec {
      public: false,
      group: Some(parts[0].clone()),
      contest: Some(parts[1].clone()),
      problem: parts[2].clone(),
    });
  }

  bail!("cannot parse problem URL: {input}");
}

fn parse_contest_spec(input: &str) -> Result<ContestSpec> {
  let input = input.trim();
  if input.is_empty() {
    bail!("contest URL or id is empty");
  }
  if is_object_id(input) {
    return Ok(ContestSpec {
      public: false,
      group: None,
      contest: input.to_string(),
    });
  }

  let parsed = if input.starts_with("http://") || input.starts_with("https://") {
    url::Url::parse(input)?
  } else {
    url::Url::parse(&format!(
      "https://cannjudge.cn{}",
      ensure_leading_slash(input)
    ))?
  };
  let parts: Vec<String> = parsed
    .path_segments()
    .into_iter()
    .flatten()
    .map(|seg| {
      urlencoding::decode(seg)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| seg.to_string())
    })
    .filter(|seg| !seg.is_empty())
    .collect();

  if parts.len() >= 2 && parts[0] == "contest" {
    return Ok(ContestSpec {
      public: false,
      group: None,
      contest: parts[1].clone(),
    });
  }

  if parts.len() >= 4 && parts[0] == "group" && parts[2] == "contest" {
    return Ok(ContestSpec {
      public: false,
      group: Some(parts[1].clone()),
      contest: parts[3].clone(),
    });
  }

  if parts.len() >= 2 && parts[0] == "public" {
    return Ok(ContestSpec {
      public: true,
      group: Some("public".to_string()),
      contest: parts[1].clone(),
    });
  }

  if parts.len() >= 2 {
    return Ok(ContestSpec {
      public: false,
      group: Some(parts[0].clone()),
      contest: parts[1].clone(),
    });
  }

  bail!("cannot parse contest URL: {input}");
}

fn ensure_leading_slash(input: &str) -> String {
  if input.starts_with('/') {
    input.to_string()
  } else {
    format!("/{input}")
  }
}

fn enc(value: &str) -> String {
  urlencoding::encode(value).into_owned()
}

fn user_query(client: &ApiClient) -> Vec<(&'static str, String)> {
  client
    .user_id()
    .map(|id| vec![("userId", id)])
    .unwrap_or_default()
}

fn build_problem_url(
  client: &ApiClient,
  group: &Value,
  contest: &Value,
  problem: &Value,
) -> String {
  let group_name = value_string(group, &["name"]);
  let contest_name = value_string(contest, &["name"]);
  let problem_name = value_string(problem, &["name", "canonical_name"]);
  if group_name == "public" && !contest_name.is_empty() && !problem_name.is_empty() {
    format!(
      "{}/public/{}/{}",
      client.base_url,
      group_name_escape(&contest_name),
      group_name_escape(&problem_name)
    )
  } else if !group_name.is_empty() && !contest_name.is_empty() && !problem_name.is_empty() {
    format!(
      "{}/{}/{}/{}",
      client.base_url,
      group_name_escape(&group_name),
      group_name_escape(&contest_name),
      group_name_escape(&problem_name)
    )
  } else {
    format!("{}/problem/{}", client.base_url, object_id(problem))
  }
}

fn build_contest_url(client: &ApiClient, group: &Value, contest: &Value) -> String {
  let group_name = value_string(group, &["name"]);
  let contest_name = value_string(contest, &["name"]);
  if group_name == "public" && !contest_name.is_empty() {
    format!(
      "{}/public/{}",
      client.base_url,
      group_name_escape(&contest_name)
    )
  } else if !group_name.is_empty() && !contest_name.is_empty() {
    format!(
      "{}/{}/{}",
      client.base_url,
      group_name_escape(&group_name),
      group_name_escape(&contest_name)
    )
  } else {
    format!("{}/contest/{}", client.base_url, object_id(contest))
  }
}

fn group_name_escape(value: &str) -> String {
  urlencoding::encode(value).into_owned()
}

pub fn fetch_template(client: &mut ApiClient, problem_id: &str) -> Result<Template> {
  let raw = client.get_json(
    &format!("/api/problems/{problem_id}/template"),
    &user_query(client),
  )?;
  let data = raw.get("data").unwrap_or(&raw);
  let mut files = normalize_project_file_items(data.get("files").and_then(Value::as_array));
  if files.is_empty() {
    files = build_legacy_code_files(data);
  }
  Ok(Template {
    problem_id: problem_id.to_string(),
    raw,
    files,
  })
}

pub fn fetch_problem(client: &mut ApiClient, problem_id: &str) -> Result<Value> {
  client.get_json(&format!("/api/problems/{problem_id}"), &user_query(client))
}

pub fn fetch_contest_problems(client: &mut ApiClient, contest_id: &str) -> Result<Value> {
  client.get_json(
    &format!("/api/problems/contest/{contest_id}"),
    &user_query(client),
  )
}

fn normalize_project_file_items(items: Option<&Vec<Value>>) -> Vec<ProjectFile> {
  items
    .into_iter()
    .flatten()
    .filter_map(|item| {
      let path = value_string(item, &["path", "key", "label"]);
      if path.is_empty() {
        return None;
      }
      Some(ProjectFile {
        path,
        content: item
          .get("content")
          .and_then(Value::as_str)
          .unwrap_or("")
          .to_string(),
        editable: item
          .get("editable")
          .and_then(Value::as_bool)
          .unwrap_or(true),
      })
    })
    .collect()
}

fn build_legacy_code_files(data: &Value) -> Vec<ProjectFile> {
  vec![
    ProjectFile {
      path: "tiling.h".to_string(),
      content: value_string(data, &["tiling_h"]),
      editable: true,
    },
    ProjectFile {
      path: "tiling_key.h".to_string(),
      content: value_string(data, &["tiling_key_h", "tiling_key_cpp"]),
      editable: true,
    },
    ProjectFile {
      path: "host.cpp".to_string(),
      content: value_string(data, &["host_cpp"]),
      editable: true,
    },
    ProjectFile {
      path: "kernel.cpp".to_string(),
      content: value_string(data, &["kernel_cpp"]),
      editable: true,
    },
  ]
}

pub fn write_template_dir(template: &Template, out_dir: &Path, problem: &ProblemRef) -> Result<()> {
  fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
  for file in &template.files {
    let target = safe_join(out_dir, &file.path)?;
    if let Some(parent) = target.parent() {
      fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&target, &file.content).with_context(|| format!("write {}", target.display()))?;
  }
  let manifest = json!({
      "problem_url": problem.problem_url,
      "problem_id": problem.problem_id,
      "contest_id": problem.contest_id,
      "group_id": problem.group_id,
      "editable_files": template.files.iter().filter(|file| file.editable).map(|file| file.path.clone()).collect::<Vec<_>>(),
      "files": template.files,
  });
  let manifest_path = out_dir.join(".cannjudge-template.json");
  fs::write(
    &manifest_path,
    serde_json::to_string_pretty(&manifest)? + "\n",
  )
  .with_context(|| format!("write {}", manifest_path.display()))?;
  Ok(())
}

pub fn download_template_package(
  client: &mut ApiClient,
  problem_id: &str,
  out: &Path,
) -> Result<String> {
  let download = client.download(
    &format!("/api/problems/{problem_id}/package"),
    &user_query(client),
  )?;
  let target = if out.is_dir() || out.extension().is_none() {
    fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    out.join(&download.file_name)
  } else {
    if let Some(parent) = out.parent() {
      fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    out.to_path_buf()
  };
  fs::write(&target, download.bytes).with_context(|| format!("write {}", target.display()))?;
  Ok(target.to_string_lossy().into_owned())
}

pub fn submit_local(
  client: &mut ApiClient,
  problem: &ProblemRef,
  options: &SubmitOptions,
) -> Result<SubmitOutcome> {
  let user_id = ensure_auth(client)?;
  let deadline = Instant::now() + options.max_wait;
  let mut waited = 0u64;

  loop {
    let template = fetch_template(client, &problem.problem_id)?;
    let payload = build_submit_payload(
      &template,
      &options.local_dir,
      &user_id,
      options.allow_missing,
    )?;
    match client.post_json("/api/submissions/submit", payload) {
      Ok(response) => {
        let submission_id = extract_submission_id(&response)
          .ok_or_else(|| anyhow!("submit response does not contain submissionId: {response}"))?;
        return Ok(SubmitOutcome {
          submission_id,
          response,
          queued_wait_seconds: waited,
        });
      }
      Err(err) => {
        if is_daily_quota_error(&err) {
          return Err(err).context("daily submission quota is exhausted");
        }
        if options.queue && is_too_frequent_error(&err) && Instant::now() < deadline {
          let delay = retry_delay(&err).unwrap_or(options.poll_interval);
          let capped = delay.min(deadline.saturating_duration_since(Instant::now()));
          if capped.is_zero() {
            return Err(err);
          }
          eprintln!(
            "submit is rate-limited; waiting {}s before retrying",
            capped.as_secs()
          );
          thread::sleep(capped);
          waited += capped.as_secs();
          continue;
        }
        return Err(err);
      }
    }
  }
}

fn build_submit_payload(
  template: &Template,
  dir: &Path,
  user_id: &str,
  allow_missing: bool,
) -> Result<Value> {
  let mut project_files = Vec::new();
  for file in &template.files {
    let mut next = file.clone();
    if file.editable {
      let local = safe_join(dir, &file.path)?;
      if local.exists() {
        next.content =
          fs::read_to_string(&local).with_context(|| format!("read {}", local.display()))?;
      } else if !allow_missing {
        bail!(
          "editable file is missing in local folder: {} (use --allow-missing to submit template content for missing files)",
          local.display()
        );
      }
    }
    project_files.push(next);
  }
  let legacy = project_files_to_legacy_payload(&project_files);
  let files = project_files
    .iter()
    .filter(|file| file.editable)
    .map(|file| json!({"path": file.path, "content": file.content}))
    .collect::<Vec<_>>();
  Ok(json!({
      "problemId": template.problem_id,
      "userId": user_id,
      "files": files,
      "tiling_h": legacy.tiling_h,
      "tiling_key_h": legacy.tiling_key_h,
      "host_cpp": legacy.host_cpp,
      "kernel_cpp": legacy.kernel_cpp,
  }))
}

struct LegacyPayload {
  tiling_h: String,
  tiling_key_h: String,
  host_cpp: String,
  kernel_cpp: String,
}

fn project_files_to_legacy_payload(files: &[ProjectFile]) -> LegacyPayload {
  let find = |predicate: fn(&str, &str) -> bool| -> String {
    files
      .iter()
      .find(|file| {
        let normalized = file.path.replace('\\', "/");
        let base = normalized.rsplit('/').next().unwrap_or("");
        predicate(&normalized, base)
      })
      .map(|file| file.content.clone())
      .unwrap_or_default()
  };

  LegacyPayload {
    tiling_h: find(|path, base| {
      base == "tiling.h" || (path.starts_with("op_kernel/") && base.ends_with("_tiling.h"))
    }),
    tiling_key_h: find(|path, base| {
      base == "tiling_key.h"
        || (path.starts_with("op_kernel/")
          && (base.starts_with("tiling_key_") || base.ends_with("_tiling_key.h")))
    }),
    host_cpp: find(|path, base| {
      path.starts_with("op_host/")
        && base != "CMakeLists.txt"
        && has_ext(base, &["cc", "cpp", "cxx", "h", "hpp"])
        || base == "host.cpp"
    }),
    kernel_cpp: find(|path, base| {
      path.starts_with("op_kernel/")
        && base != "CMakeLists.txt"
        && has_ext(base, &["cc", "cpp", "cxx"])
        || base == "kernel.cpp"
    }),
  }
}

fn has_ext(path: &str, exts: &[&str]) -> bool {
  let Some((_, ext)) = path.rsplit_once('.') else {
    return false;
  };
  exts
    .iter()
    .any(|candidate| ext.eq_ignore_ascii_case(candidate))
}

pub fn extract_submission_id(response: &Value) -> Option<String> {
  for pointer in [
    "/data/submissionId",
    "/data/submission_id",
    "/submissionId",
    "/submission_id",
    "/_id",
    "/id",
  ] {
    if let Some(text) = response.pointer(pointer).and_then(Value::as_str)
      && !text.trim().is_empty()
    {
      return Some(text.trim().to_string());
    }
  }
  None
}

pub fn parse_submission_id(input: &str) -> Result<String> {
  let input = input.trim();
  if is_object_id(input) {
    return Ok(input.to_string());
  }
  let parsed = if input.starts_with("http://") || input.starts_with("https://") {
    url::Url::parse(input)?
  } else {
    url::Url::parse(&format!(
      "https://cannjudge.cn{}",
      ensure_leading_slash(input)
    ))?
  };
  let parts: Vec<String> = parsed
    .path_segments()
    .into_iter()
    .flatten()
    .map(ToString::to_string)
    .collect();
  for window in parts.windows(2) {
    if window[0] == "submission" && is_object_id(&window[1]) {
      return Ok(window[1].clone());
    }
  }
  bail!("cannot parse submission id: {input}");
}

pub fn fetch_submission(client: &mut ApiClient, submission_id: &str) -> Result<Value> {
  client.get_json(
    &format!("/api/submissions/{submission_id}"),
    &user_query(client),
  )
}

pub fn download_submission_package(
  client: &mut ApiClient,
  submission_id: &str,
  out: &Path,
) -> Result<String> {
  let download = client.download(
    &format!("/api/submissions/{submission_id}/package"),
    &user_query(client),
  )?;
  let target = if out.is_dir() || out.extension().is_none() {
    fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    out.join(&download.file_name)
  } else {
    if let Some(parent) = out.parent() {
      fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    out.to_path_buf()
  };
  fs::write(&target, download.bytes).with_context(|| format!("write {}", target.display()))?;
  Ok(target.to_string_lossy().into_owned())
}

pub fn fetch_remote_history(
  client: &mut ApiClient,
  problem_id: &str,
  user_id: Option<&str>,
  limit: usize,
) -> Result<Value> {
  if let Some(user_id) = user_id
    && !user_id.is_empty()
  {
    return client.get_json(
      &format!("/api/submissions/user/{user_id}/problem/{problem_id}"),
      &[],
    );
  }
  client.get_json(
    "/api/submissions/global/list",
    &[
      ("problemId", problem_id.to_string()),
      ("limit", limit.to_string()),
    ],
  )
}

pub fn fetch_ranking(
  client: &mut ApiClient,
  problem_id: &str,
  page: usize,
  size: usize,
) -> Result<Value> {
  let mut query = user_query(client);
  let page = page.max(1);
  let size = size.max(1);
  query.push(("page", page.to_string()));
  query.push(("size", size.to_string()));
  let mut value = client.get_json(&format!("/api/problems/{problem_id}/ranking"), &query)?;
  annotate_ranking_rows(&mut value, page, size);
  Ok(value)
}

pub fn fetch_ranking_by_ranks(
  client: &mut ApiClient,
  problem_id: &str,
  ranks: &[usize],
  size: usize,
) -> Result<Value> {
  if ranks.is_empty() {
    return fetch_ranking(client, problem_id, 1, size);
  }
  let size = size.max(1);
  let wanted = ranks.iter().copied().collect::<BTreeSet<_>>();
  let pages = wanted
    .iter()
    .map(|rank| (rank - 1) / size + 1)
    .collect::<BTreeSet<_>>();
  let mut combined = json!({
      "rows": [],
      "rank_filter": ranks,
      "size": size,
  });
  let mut selected = Vec::new();

  for page in pages {
    let value = fetch_ranking(client, problem_id, page, size)?;
    if combined.get("testcases").is_none()
      && let Some(testcases) = value.get("testcases")
    {
      combined["testcases"] = testcases.clone();
    }
    for row in ranking_rows(&value) {
      let rank = row.get("rank").and_then(Value::as_u64).unwrap_or(0) as usize;
      if wanted.contains(&rank) {
        selected.push(row.clone());
      }
    }
    if let Some(total) = value.get("total") {
      combined["total"] = total.clone();
    }
    if let Some(pages) = value.get("pages") {
      combined["pages"] = pages.clone();
    }
  }

  combined["rows"] = Value::Array(selected);
  Ok(combined)
}

pub fn fetch_ranking_pages(
  client: &mut ApiClient,
  problem_id: &str,
  start_page: usize,
  page_count: usize,
  size: usize,
) -> Result<Value> {
  let start_page = start_page.max(1);
  let page_count = page_count.max(1);
  let size = size.max(1);
  let mut combined = json!({
      "rows": [],
      "page": start_page,
      "scan_pages": page_count,
      "size": size,
  });
  let mut rows = Vec::new();

  for page in start_page..start_page.saturating_add(page_count) {
    let value = fetch_ranking(client, problem_id, page, size)?;
    if combined.get("testcases").is_none()
      && let Some(testcases) = value.get("testcases")
    {
      combined["testcases"] = testcases.clone();
    }
    rows.extend(ranking_rows(&value).into_iter().cloned());
    if let Some(total) = value.get("total") {
      combined["total"] = total.clone();
    }
    if let Some(pages) = value.get("pages") {
      combined["pages"] = pages.clone();
      if page >= pages.as_u64().unwrap_or(page as u64) as usize {
        break;
      }
    }
  }

  combined["rows"] = Value::Array(rows);
  Ok(combined)
}

pub fn parse_rank_selectors(values: &[String]) -> Result<Vec<usize>> {
  let mut ranks = BTreeSet::new();
  for value in values {
    for part in value
      .split(',')
      .map(str::trim)
      .filter(|part| !part.is_empty())
    {
      if let Some((start, end)) = part.split_once("..").or_else(|| part.split_once('-')) {
        let start = parse_rank_number(start)?;
        let end = parse_rank_number(end)?;
        if start > end {
          bail!("rank range start is greater than end: {part}");
        }
        for rank in start..=end {
          ranks.insert(rank);
        }
      } else {
        ranks.insert(parse_rank_number(part)?);
      }
    }
  }
  Ok(ranks.into_iter().collect())
}

fn parse_rank_number(value: &str) -> Result<usize> {
  let rank = value
    .trim()
    .parse::<usize>()
    .with_context(|| format!("invalid rank: {value}"))?;
  if rank == 0 {
    bail!("rank must be >= 1");
  }
  Ok(rank)
}

fn annotate_ranking_rows(value: &mut Value, page: usize, size: usize) {
  let current = value
    .get("current")
    .or_else(|| value.get("page"))
    .and_then(Value::as_u64)
    .map(|v| v.max(1) as usize)
    .unwrap_or(page.max(1));
  let page_size = value
    .get("size")
    .and_then(Value::as_u64)
    .map(|v| v.max(1) as usize)
    .unwrap_or(size.max(1));
  if let Some(rows) = ranking_rows_mut(value) {
    for (index, row) in rows.iter_mut().enumerate() {
      if let Some(object) = row.as_object_mut() {
        object.insert(
          "rank".to_string(),
          json!((current - 1) * page_size + index + 1),
        );
      }
    }
  }
}

fn ranking_rows(value: &Value) -> Vec<&Value> {
  value
    .get("rows")
    .or_else(|| value.get("list"))
    .and_then(Value::as_array)
    .map(|rows| rows.iter().collect())
    .unwrap_or_default()
}

fn ranking_rows_mut(value: &mut Value) -> Option<&mut Vec<Value>> {
  if value.get("rows").is_some() {
    return value.get_mut("rows").and_then(Value::as_array_mut);
  }
  value.get_mut("list").and_then(Value::as_array_mut)
}

pub fn watch_submission(
  client: &mut ApiClient,
  submission_id: &str,
  interval: Duration,
  timeout: Duration,
) -> Result<Value> {
  let deadline = Instant::now() + timeout;
  loop {
    let value = fetch_submission(client, submission_id)?;
    let status = value
      .get("status")
      .and_then(Value::as_str)
      .unwrap_or("")
      .to_string();
    if is_terminal_status(&status) {
      return Ok(value);
    }
    if Instant::now() >= deadline {
      return Ok(value);
    }
    thread::sleep(interval);
  }
}

pub fn is_terminal_status(status: &str) -> bool {
  let key = status.trim().to_ascii_lowercase();
  !(key.is_empty()
    || key == "running"
    || key == "pending"
    || key == "queued"
    || key == "judging"
    || key == "compiling")
}

pub fn is_too_frequent_error(err: &anyhow::Error) -> bool {
  let text = error_text(err);
  text.contains("频繁")
    || text.contains("过于")
    || text.contains("后再")
    || text.contains("too frequent")
    || text.contains("rate")
    || downcast_api_error(err).is_some_and(|api| api.status == 429)
}

pub fn is_daily_quota_error(err: &anyhow::Error) -> bool {
  let text = error_text(err);
  (text.contains("次数")
    && (text.contains("用完") || text.contains("上限") || text.contains("今日")))
    || text.contains("daily")
    || text.contains("quota")
}

pub fn retry_delay(err: &anyhow::Error) -> Option<Duration> {
  let text = error_text(err);
  let patterns = [
    (r"(\d+)\s*秒", 1u64),
    (r"(\d+)\s*s(?:ec(?:ond)?s?)?\b", 1u64),
    (r"(\d+)\s*分钟", 60u64),
    (r"(\d+)\s*m(?:in(?:ute)?s?)?\b", 60u64),
  ];
  for (pattern, unit) in patterns {
    let re = regex::Regex::new(pattern).ok()?;
    if let Some(captures) = re.captures(&text)
      && let Some(number) = captures.get(1)
      && let Ok(value) = number.as_str().parse::<u64>()
    {
      return Some(Duration::from_secs(value.saturating_mul(unit).max(1)));
    }
  }
  None
}

fn error_text(err: &anyhow::Error) -> String {
  let mut text = err.to_string().to_ascii_lowercase();
  if let Some(api) = downcast_api_error(err) {
    text.push(' ');
    text.push_str(&api.message.to_ascii_lowercase());
    text.push(' ');
    text.push_str(&api.payload.to_string().to_ascii_lowercase());
  }
  text
}

pub fn raw_api(
  client: &mut ApiClient,
  method: Method,
  path: &str,
  data: Option<Value>,
) -> Result<Value> {
  client.request_json(method, path, &[], data)
}
