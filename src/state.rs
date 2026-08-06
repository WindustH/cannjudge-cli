use crate::util::{now_secs, read_json_file, write_json_0600};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
  #[serde(default)]
  pub submissions: Vec<LocalSubmission>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalSubmission {
  pub submission_id: String,
  pub problem_id: String,
  pub problem_url: String,
  pub local_dir: String,
  pub status: String,
  pub created: f64,
  pub queued_wait_seconds: u64,
}

impl State {
  pub fn load(path: &Path) -> Self {
    read_json_file(path).unwrap_or_default()
  }

  pub fn save(&self, path: &Path) -> Result<()> {
    write_json_0600(path, self)
  }

  pub fn record_submission(&mut self, entry: LocalSubmission) {
    self
      .submissions
      .retain(|old| old.submission_id != entry.submission_id);
    self.submissions.insert(0, entry);
    if self.submissions.len() > 1000 {
      self.submissions.truncate(1000);
    }
  }
}

impl LocalSubmission {
  pub fn new(
    submission_id: String,
    problem_id: String,
    problem_url: String,
    local_dir: String,
    queued_wait_seconds: u64,
  ) -> Self {
    Self {
      submission_id,
      problem_id,
      problem_url,
      local_dir,
      status: "Submitted".to_string(),
      created: now_secs(),
      queued_wait_seconds,
    }
  }
}
