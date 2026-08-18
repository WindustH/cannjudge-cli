use crate::auth::Auth;
use crate::cannjudge::{ContestRef, ProblemRef, SubmissionQuota};
use crate::state::LocalSubmission;
use crate::util::{print_json, truncate, value_string};
use anyhow::Result;
use serde_json::{Value, json};

pub fn print_auth(auth: &Auth, json_output: bool) -> Result<()> {
  if json_output {
    return print_json(&auth.user);
  }
  println!("user: {}", auth.user_label());
  println!("user_id: {}", auth.user_id());
  if let Some(id) = auth.user_numeric_id() {
    println!("ID: {id}");
  }
  println!("source: {}", auth.source);
  println!("cookies: {}", auth.cookies.len());
  Ok(())
}

pub fn print_submission_quota(
  quota: &SubmissionQuota,
  account: &str,
  json_output: bool,
) -> Result<()> {
  if json_output {
    let mut value = serde_json::to_value(quota)?;
    value["account"] = Value::String(account.to_string());
    return print_json(&value);
  }
  println!("account: {account}");
  println!("user_id: {}", quota.user_id);
  println!("beijing_date: {}", quota.beijing_date);
  println!("used_today: {}/{}", quota.used_today, quota.daily_limit);
  println!("remaining_today: {}", quota.remaining_today);
  println!("reset: {} {}", quota.reset_timezone, quota.reset_time);
  Ok(())
}

pub fn print_problem(problem: &ProblemRef, json_output: bool) -> Result<()> {
  if json_output {
    return print_json(&json!({
        "group": problem.group,
        "contest": problem.contest,
        "problem": problem.problem,
        "problem_url": problem.problem_url,
    }));
  }
  println!(
    "problem: {}",
    value_string(&problem.problem, &["title", "name"])
  );
  println!("problem_id: {}", problem.problem_id);
  println!(
    "contest: {}",
    value_string(&problem.contest, &["title", "name"])
  );
  println!("contest_id: {}", problem.contest_id);
  println!(
    "group: {}",
    value_string(&problem.group, &["title", "name"])
  );
  println!("url: {}", problem.problem_url);
  let tags = problem
    .problem
    .get("tags")
    .and_then(Value::as_array)
    .map(|tags| {
      tags
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(",")
    })
    .unwrap_or_default();
  if !tags.is_empty() {
    println!("tags: {tags}");
  }
  Ok(())
}

pub fn print_problem_content(problem: &ProblemRef, value: &Value, json_output: bool) -> Result<()> {
  if json_output {
    return print_json(value);
  }
  println!("problem: {}", value_string(value, &["title", "name"]));
  println!("problem_id: {}", value_string(value, &["_id", "id"]));
  println!("url: {}", problem.problem_url);
  println!();
  let content = value_string(value, &["desc", "description", "content", "statement"]);
  if content.trim().is_empty() {
    println!("no problem content");
  } else {
    println!("{content}");
  }
  Ok(())
}

pub fn print_contest(contest: &ContestRef, json_output: bool) -> Result<()> {
  if json_output {
    return print_json(&json!({
        "group": contest.group,
        "contest": contest.contest,
        "contest_url": contest.contest_url,
    }));
  }
  println!(
    "contest: {}",
    value_string(&contest.contest, &["title", "name"])
  );
  println!("contest_id: {}", contest.contest_id);
  println!(
    "group: {}",
    value_string(&contest.group, &["title", "name"])
  );
  println!("group_id: {}", contest.group_id);
  println!("url: {}", contest.contest_url);
  let desc = value_string(&contest.contest, &["desc", "description"]);
  if !desc.is_empty() {
    println!("desc: {}", truncate(&desc, 160));
  }
  Ok(())
}

pub fn print_problem_list(contest: &ContestRef, value: &Value, json_output: bool) -> Result<()> {
  let rows = problem_rows(value);
  if json_output {
    return print_json(&json!({
        "contest": contest.contest,
        "contest_url": contest.contest_url,
        "problems": rows,
    }));
  }
  println!(
    "contest: {}",
    value_string(&contest.contest, &["title", "name"])
  );
  println!("url: {}", contest.contest_url);
  println!();
  println!(
    "{:<4} {:<26} {:<24} {:<28} {:<8} {}",
    "#", "problem_id", "name", "title", "score", "url"
  );
  for (index, problem) in rows.iter().enumerate() {
    println!(
      "{:<4} {:<26} {:<24} {:<28} {:<8} {}",
      index + 1,
      value_string(problem, &["_id", "id"]),
      truncate(&value_string(problem, &["name", "canonical_name"]), 24),
      truncate(&value_string(problem, &["title"]), 28),
      metric(problem, &["score_mode", "score"]),
      problem_url(contest, problem)
    );
  }
  Ok(())
}

pub fn print_submission(value: &Value, json_output: bool, show_logs: bool) -> Result<()> {
  if json_output {
    return print_json(value);
  }
  println!("submission_id: {}", value_string(value, &["_id", "id"]));
  println!("ID: {}", value_string(value, &["ID"]));
  println!("status: {}", value_string(value, &["status"]));
  println!(
    "problem: {}",
    value_string(
      value.get("problem").unwrap_or(&Value::Null),
      &["title", "name"]
    )
  );
  println!(
    "user: {}",
    user_label(value.get("user").unwrap_or(&Value::Null))
  );
  println!("created: {}", value_string(value, &["create_time"]));

  let rows = value
    .get("result")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  if rows.is_empty() {
    println!("testcases: none");
    return Ok(());
  }
  println!();
  println!(
    "{:<6} {:<16} {:>10} {:>12} {:>10} {:>10} {}",
    "case", "status", "time", "best_time", "ratio", "type", "message"
  );
  for (index, row) in rows.iter().enumerate() {
    let msg = row.get("msg").and_then(Value::as_str).unwrap_or("");
    println!(
      "{:<6} {:<16} {:>10} {:>12} {:>10} {:>10} {}",
      index + 1,
      truncate(&value_string(row, &["testcase_status", "status"]), 16),
      metric(row, &["time"]),
      metric(row, &["best_time", "bestTime"]),
      metric(row, &["precision_ratio"]),
      truncate(&value_string(row, &["type", "case_type"]), 10),
      truncate(msg.lines().next().unwrap_or(""), 80)
    );
    if show_logs && !msg.trim().is_empty() {
      println!("--- case {} log ---", index + 1);
      println!("{msg}");
      println!("--- end case {} log ---", index + 1);
    }
  }
  let pass = rows
    .iter()
    .filter(|row| value_string(row, &["testcase_status", "status"]).eq_ignore_ascii_case("pass"))
    .count();
  println!();
  println!("pass: {pass}/{}", rows.len());
  Ok(())
}

pub fn print_history(
  remote: Option<&Value>,
  local: &[LocalSubmission],
  json_output: bool,
) -> Result<()> {
  if json_output {
    return print_json(&json!({
        "remote": remote,
        "local": local,
    }));
  }
  if let Some(remote) = remote {
    let rows = remote_rows(remote);
    if !rows.is_empty() {
      println!("remote:");
      println!(
        "{:<26} {:<10} {:<16} {}",
        "submission_id", "ID", "status", "created"
      );
      for row in rows {
        println!(
          "{:<26} {:<10} {:<16} {}",
          value_string(row, &["_id", "id"]),
          value_string(row, &["ID"]),
          truncate(&value_string(row, &["status"]), 16),
          value_string(row, &["create_time"])
        );
      }
    }
  }
  if !local.is_empty() {
    println!("local:");
    println!(
      "{:<26} {:<16} {:<8} {}",
      "submission_id", "status", "queued", "local_dir"
    );
    for row in local {
      println!(
        "{:<26} {:<16} {:<8} {}",
        row.submission_id, row.status, row.queued_wait_seconds, row.local_dir
      );
    }
  }
  Ok(())
}

pub fn print_ranking(
  value: &Value,
  json_output: bool,
  filter_user: Option<&str>,
  filter_submitter: Option<&str>,
  baseline_only: bool,
) -> Result<()> {
  let view = filtered_ranking_value(value, filter_user, filter_submitter, baseline_only);
  if json_output {
    return print_json(&view);
  }
  if baseline_only {
    let testcases = view
      .get("testcases")
      .and_then(Value::as_array)
      .cloned()
      .unwrap_or_default();
    println!(
      "{:<8} {:<26} {:>12} {}",
      "case", "testcase_id", "baseline", "type"
    );
    for (index, testcase) in testcases.iter().enumerate() {
      println!(
        "{:<8} {:<26} {:>12} {}",
        index + 1,
        value_string(testcase, &["_id", "id"]),
        metric(testcase, &["baseline"]),
        value_string(testcase, &["type"])
      );
    }
    return Ok(());
  }

  let rows = view
    .get("rows")
    .or_else(|| view.get("list"))
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  println!(
    "{:<6} {:<24} {:<24} {:<16} {:>8} {:<16} {}",
    "rank", "user/team", "submitter", "status", "score", "submission", "created"
  );
  for row in rows.iter() {
    println!(
      "{:<6} {:<24} {:<24} {:<16} {:>8} {:<16} {}",
      value_string(row, &["rank"]),
      truncate(&ranking_subject_label(row), 24),
      truncate(&ranking_submitter_label(row), 24),
      truncate(&value_string(row, &["status"]), 16),
      metric(row, &["score"]),
      value_string(row, &["submission_id"]),
      value_string(row, &["create_time"])
    );
  }
  Ok(())
}

fn filtered_ranking_value(
  value: &Value,
  filter_user: Option<&str>,
  filter_submitter: Option<&str>,
  baseline_only: bool,
) -> Value {
  if baseline_only {
    return json!({
        "testcases": value.get("testcases").cloned().unwrap_or_else(|| json!([])),
    });
  }
  let mut out = value.clone();
  if (filter_user.is_some() || filter_submitter.is_some())
    && let Some(rows) = ranking_rows_mut(&mut out)
  {
    rows.retain(|row| {
      filter_user.is_none_or(|needle| ranking_user_matches(row, needle))
        && filter_submitter.is_none_or(|needle| ranking_submitter_matches(row, needle))
    });
  }
  out
}

fn ranking_user_matches(row: &Value, needle: &str) -> bool {
  let needle = needle.trim();
  if needle.is_empty() {
    return true;
  }
  let needle_lower = needle.to_ascii_lowercase();
  for id in [
    value_string(row, &["user_id", "submitter_id"]),
    value_string(
      row.get("user").unwrap_or(&Value::Null),
      &["_id", "id", "ID"],
    ),
    value_string(
      row.get("submitter").unwrap_or(&Value::Null),
      &["_id", "id", "ID"],
    ),
    value_string(
      row.get("team").unwrap_or(&Value::Null),
      &["_id", "id", "ID", "team_id"],
    ),
  ] {
    if !id.is_empty() && id == needle {
      return true;
    }
  }
  for label in [
    user_label(row.get("user").unwrap_or(&Value::Null)),
    value_string(
      row.get("team").unwrap_or(&Value::Null),
      &["team_name", "name", "team_id"],
    ),
  ] {
    if !label.is_empty() && label.to_ascii_lowercase().contains(&needle_lower) {
      return true;
    }
  }
  false
}

fn ranking_submitter_matches(row: &Value, needle: &str) -> bool {
  let needle = needle.trim();
  if needle.is_empty() {
    return true;
  }
  let needle_lower = needle.to_ascii_lowercase();
  let submitter = row
    .get("submitter")
    .or_else(|| row.get("user"))
    .unwrap_or(&Value::Null);
  for id in [
    value_string(row, &["submitter_id", "user_id"]),
    value_string(submitter, &["_id", "id", "ID"]),
  ] {
    if !id.is_empty() && id == needle {
      return true;
    }
  }
  let label = user_label(submitter);
  !label.is_empty() && label.to_ascii_lowercase().contains(&needle_lower)
}

fn ranking_rows_mut(value: &mut Value) -> Option<&mut Vec<Value>> {
  if value.get("rows").is_some() {
    return value.get_mut("rows").and_then(Value::as_array_mut);
  }
  value.get_mut("list").and_then(Value::as_array_mut)
}

pub fn remote_rows(value: &Value) -> Vec<&Value> {
  if let Some(rows) = value.as_array() {
    return rows.iter().collect();
  }
  value
    .get("rows")
    .or_else(|| value.get("list"))
    .and_then(Value::as_array)
    .map(|rows| rows.iter().collect())
    .unwrap_or_default()
}

fn ranking_subject_label(row: &Value) -> String {
  let team = row.get("team").unwrap_or(&Value::Null);
  let team_label = value_string(team, &["team_name", "name", "team_id"]);
  if !team_label.is_empty() {
    return team_label;
  }
  let user = row
    .get("user")
    .or_else(|| row.get("submitter"))
    .unwrap_or(&Value::Null);
  user_label(user)
}

fn ranking_submitter_label(row: &Value) -> String {
  let submitter = row
    .get("submitter")
    .or_else(|| row.get("user"))
    .unwrap_or(&Value::Null);
  let label = user_label(submitter);
  if label.is_empty() {
    "-".to_string()
  } else {
    label
  }
}

fn problem_rows(value: &Value) -> Vec<Value> {
  if let Some(rows) = value.as_array() {
    return rows.clone();
  }
  value
    .get("rows")
    .or_else(|| value.get("list"))
    .or_else(|| value.get("problems"))
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default()
}

fn problem_url(contest: &ContestRef, problem: &Value) -> String {
  let name = value_string(problem, &["name", "canonical_name"]);
  let id = value_string(problem, &["_id", "id"]);
  if !name.is_empty() && !contest.contest_url.contains("/contest/") {
    return format!(
      "{}/{}",
      contest.contest_url.trim_end_matches('/'),
      urlencoding::encode(&name)
    );
  }
  let base = contest
    .contest_url
    .split("/contest/")
    .next()
    .unwrap_or_else(|| contest.contest_url.trim_end_matches('/'));
  format!("{}/problem/{id}", base.trim_end_matches('/'))
}

fn user_label(value: &Value) -> String {
  value_string(value, &["nickname", "email", "username", "ID", "_id"])
}

fn metric(value: &Value, keys: &[&str]) -> String {
  for key in keys {
    if let Some(number) = value.get(*key).and_then(Value::as_f64) {
      return if (number.fract()).abs() < 1e-9 {
        format!("{number:.0}")
      } else {
        format!("{number:.4}")
          .trim_end_matches('0')
          .trim_end_matches('.')
          .to_string()
      };
    }
    if let Some(text) = value.get(*key).and_then(Value::as_str)
      && !text.trim().is_empty()
    {
      return text.trim().to_string();
    }
  }
  "-".to_string()
}
