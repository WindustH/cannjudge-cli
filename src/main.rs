mod auth;
mod cannjudge;
mod cdp;
mod client;
mod config;
mod output;
mod state;
mod util;

use anyhow::{Context, Result};
use auth::{Auth, LoginOptions, load_or_login, login_with_browser};
use cannjudge::{
  SubmitOptions, download_submission_package, download_template_package, fetch_contest_problems,
  fetch_problem, fetch_ranking, fetch_ranking_by_ranks, fetch_ranking_pages, fetch_remote_history,
  fetch_submission, fetch_template, parse_rank_selectors, parse_submission_id, raw_api,
  resolve_contest, resolve_problem, submit_local, watch_submission, write_template_dir,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use client::ApiClient;
use reqwest::Method;
use serde_json::Value;
use state::{LocalSubmission, State};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use util::print_json;

#[derive(Parser)]
#[command(name = "cannjudge")]
#[command(about = "CANNJudge command-line helper", version)]
struct Cli {
  #[arg(long, env = "CANNJUDGE_BASE_URL", default_value = config::DEFAULT_BASE_URL)]
  base_url: String,

  #[arg(long, default_value_t = config::default_auth_cache())]
  auth_cache: String,

  #[arg(long, default_value_t = config::default_state_file())]
  state_file: String,

  #[arg(long, default_value_t = config::default_cache_dir())]
  cache_dir: String,

  #[arg(long, global = true)]
  no_cache: bool,

  #[arg(long, global = true)]
  refresh_cache: bool,

  #[arg(long, global = true, default_value_t = 3600)]
  cache_ttl: u64,

  #[arg(long, default_value_t = config::default_cdp_list_url())]
  cdp_list_url: String,

  #[arg(long, default_value_t = config::default_chrome_bin())]
  chrome_bin: String,

  #[arg(long, default_value_t = config::default_chrome_profile())]
  chrome_profile: String,

  #[arg(long, global = true)]
  json: bool,

  #[arg(long, global = true)]
  debug: bool,

  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  Auth(AuthCommand),
  Problem(ProblemCommand),
  Contest(ContestCommand),
  Submit(SubmitCommand),
  Submission(SubmissionCommand),
  History(HistoryCommand),
  Ranking(RankingCommand),
  Api(ApiCommand),
}

#[derive(Args)]
struct AuthCommand {
  #[command(subcommand)]
  command: AuthSubcommand,
}

#[derive(Subcommand)]
enum AuthSubcommand {
  Login {
    #[arg(long)]
    force: bool,
    #[arg(long)]
    no_launch: bool,
    #[arg(long, default_value_t = 300)]
    timeout: u64,
  },
  Status,
  Logout,
}

#[derive(Args)]
struct ProblemCommand {
  #[command(subcommand)]
  command: ProblemSubcommand,
}

#[derive(Subcommand)]
enum ProblemSubcommand {
  Info {
    problem: String,
  },
  Content {
    problem: String,
    #[arg(short, long)]
    out: Option<PathBuf>,
  },
  Template {
    problem: String,
    #[arg(short, long, default_value = ".")]
    out: PathBuf,
    #[arg(long)]
    zip: bool,
  },
}

#[derive(Args)]
struct ContestCommand {
  #[command(subcommand)]
  command: ContestSubcommand,
}

#[derive(Subcommand)]
enum ContestSubcommand {
  Info { contest: String },
  Problems { contest: String },
}

#[derive(Args)]
struct SubmitCommand {
  problem: String,
  dir: PathBuf,
  #[arg(long)]
  allow_missing: bool,
  #[arg(long)]
  queue: bool,
  #[arg(long, default_value_t = 3600)]
  max_wait: u64,
  #[arg(long, default_value_t = 30)]
  retry_interval: u64,
  #[arg(long)]
  watch: bool,
  #[arg(long, default_value_t = 10)]
  watch_interval: u64,
  #[arg(long, default_value_t = 7200)]
  watch_timeout: u64,
}

#[derive(Args)]
struct SubmissionCommand {
  #[command(subcommand)]
  command: SubmissionSubcommand,
}

#[derive(Subcommand)]
enum SubmissionSubcommand {
  Status {
    submission: String,
    #[arg(long)]
    watch: bool,
    #[arg(long)]
    logs: bool,
    #[arg(long, default_value_t = 10)]
    interval: u64,
    #[arg(long, default_value_t = 7200)]
    timeout: u64,
  },
  Download {
    submission: String,
    #[arg(short, long, default_value = ".")]
    out: PathBuf,
    #[arg(long)]
    zip: bool,
  },
}

#[derive(Args)]
struct HistoryCommand {
  problem: String,
  #[arg(long)]
  mine: bool,
  #[arg(long)]
  user: Option<String>,
  #[arg(long, default_value_t = 50)]
  limit: usize,
  #[arg(long)]
  local_only: bool,
  #[arg(long)]
  remote_only: bool,
}

#[derive(Args)]
struct RankingCommand {
  problem: String,
  #[arg(long)]
  mine: bool,
  #[arg(long)]
  user: Option<String>,
  #[arg(long)]
  submitter: Option<String>,
  #[arg(long)]
  baseline: bool,
  #[arg(long, default_value_t = 1)]
  page: usize,
  #[arg(long, default_value_t = 50)]
  size: usize,
  #[arg(long, default_value_t = 1)]
  scan_pages: usize,
  #[arg(long = "rank", value_delimiter = ',')]
  ranks: Vec<String>,
}

#[derive(Args)]
struct ApiCommand {
  method: ApiMethod,
  path: String,
  #[arg(long)]
  data: Option<String>,
  #[arg(long)]
  auth: bool,
}

#[derive(Clone, ValueEnum)]
enum ApiMethod {
  Get,
  Post,
  Put,
  Delete,
}

fn main() -> Result<()> {
  let cli = Cli::parse();
  run(cli)
}

fn run(cli: Cli) -> Result<()> {
  match &cli.command {
    Command::Auth(command) => run_auth(&cli, command),
    Command::Problem(command) => run_problem(&cli, command),
    Command::Contest(command) => run_contest(&cli, command),
    Command::Submit(command) => run_submit(&cli, command),
    Command::Submission(command) => run_submission(&cli, command),
    Command::History(command) => run_history(&cli, command),
    Command::Ranking(command) => run_ranking(&cli, command),
    Command::Api(command) => run_api(&cli, command),
  }
}

fn run_auth(cli: &Cli, command: &AuthCommand) -> Result<()> {
  let auth_path = config::expand_tilde(&cli.auth_cache);
  match &command.command {
    AuthSubcommand::Login {
      force,
      no_launch,
      timeout,
    } => {
      let mut options = login_options(cli);
      options.no_launch = *no_launch;
      options.login_timeout = Duration::from_secs(*timeout);
      let auth = if *force {
        login_with_browser(&options)?
      } else {
        load_or_login(&options, false)?
      };
      output::print_auth(&auth, cli.json)
    }
    AuthSubcommand::Status => {
      let auth = Auth::load(&auth_path)
        .with_context(|| format!("no valid auth cache at {}", auth_path.display()))?;
      output::print_auth(&auth, cli.json)
    }
    AuthSubcommand::Logout => {
      if auth_path.exists() {
        fs::remove_file(&auth_path).with_context(|| format!("remove {}", auth_path.display()))?;
      }
      println!("logged out");
      Ok(())
    }
  }
}

fn run_problem(cli: &Cli, command: &ProblemCommand) -> Result<()> {
  let mut client = client(cli, false)?;
  match &command.command {
    ProblemSubcommand::Info { problem } => {
      let problem = resolve_problem(&mut client, problem)?;
      output::print_problem(&problem, cli.json)
    }
    ProblemSubcommand::Content { problem, out } => {
      let problem_ref = resolve_problem(&mut client, problem)?;
      let value = fetch_problem(&mut client, &problem_ref.problem_id)?;
      if let Some(out) = out {
        let content = util::value_string(&value, &["desc", "description", "content", "statement"]);
        if let Some(parent) = out.parent() {
          fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(out, content).with_context(|| format!("write {}", out.display()))?;
        return Ok(());
      }
      output::print_problem_content(&problem_ref, &value, cli.json)
    }
    ProblemSubcommand::Template { problem, out, zip } => {
      let problem_ref = resolve_problem(&mut client, problem)?;
      if *zip {
        let path = download_template_package(&mut client, &problem_ref.problem_id, out)?;
        println!("{path}");
        return Ok(());
      }
      let template = fetch_template(&mut client, &problem_ref.problem_id)?;
      write_template_dir(&template, out, &problem_ref)?;
      if cli.json {
        print_json(&serde_json::to_value(&template)?)?;
      } else {
        println!("wrote {} files to {}", template.files.len(), out.display());
      }
      Ok(())
    }
  }
}

fn run_contest(cli: &Cli, command: &ContestCommand) -> Result<()> {
  let mut client = client(cli, false)?;
  match &command.command {
    ContestSubcommand::Info { contest } => {
      let contest = resolve_contest(&mut client, contest)?;
      output::print_contest(&contest, cli.json)
    }
    ContestSubcommand::Problems { contest } => {
      let contest = resolve_contest(&mut client, contest)?;
      let problems = fetch_contest_problems(&mut client, &contest.contest_id)?;
      output::print_problem_list(&contest, &problems, cli.json)
    }
  }
}

fn run_submit(cli: &Cli, command: &SubmitCommand) -> Result<()> {
  let mut client = client(cli, true)?;
  let problem = resolve_problem(&mut client, &command.problem)?;
  let outcome = submit_local(
    &mut client,
    &problem,
    &SubmitOptions {
      local_dir: command.dir.clone(),
      allow_missing: command.allow_missing,
      queue: command.queue,
      max_wait: Duration::from_secs(command.max_wait),
      poll_interval: Duration::from_secs(command.retry_interval),
    },
  )?;

  let state_path = config::expand_tilde(&cli.state_file);
  let mut state = State::load(&state_path);
  state.record_submission(LocalSubmission::new(
    outcome.submission_id.clone(),
    problem.problem_id.clone(),
    problem.problem_url.clone(),
    command.dir.to_string_lossy().into_owned(),
    outcome.queued_wait_seconds,
  ));
  state.save(&state_path)?;

  if cli.json {
    print_json(&serde_json::to_value(&outcome)?)?;
  } else {
    println!("submission_id: {}", outcome.submission_id);
    println!(
      "url: {}/submission/{}",
      problem.problem_url, outcome.submission_id
    );
    if outcome.queued_wait_seconds > 0 {
      println!("queued_wait_seconds: {}", outcome.queued_wait_seconds);
    }
  }

  if command.watch {
    let value = watch_submission(
      &mut client,
      &outcome.submission_id,
      Duration::from_secs(command.watch_interval),
      Duration::from_secs(command.watch_timeout),
    )?;
    output::print_submission(&value, cli.json, true)?;
  }
  Ok(())
}

fn run_submission(cli: &Cli, command: &SubmissionCommand) -> Result<()> {
  match &command.command {
    SubmissionSubcommand::Status {
      submission,
      watch,
      logs,
      interval,
      timeout,
    } => {
      let mut client = client(cli, false)?;
      let id = parse_submission_id(submission)?;
      let value = if *watch {
        watch_submission(
          &mut client,
          &id,
          Duration::from_secs(*interval),
          Duration::from_secs(*timeout),
        )?
      } else {
        fetch_submission(&mut client, &id)?
      };
      output::print_submission(&value, cli.json, *logs)
    }
    SubmissionSubcommand::Download {
      submission,
      out,
      zip,
    } => {
      let mut client = client(cli, true)?;
      let id = parse_submission_id(submission)?;
      if *zip {
        let path = download_submission_package(&mut client, &id, out)?;
        println!("{path}");
        return Ok(());
      }
      let value = fetch_submission(&mut client, &id)?;
      let count = write_submission_code(&value, out)?;
      if cli.json {
        print_json(&value)?;
      } else {
        println!("wrote {count} files to {}", out.display());
      }
      Ok(())
    }
  }
}

fn run_history(cli: &Cli, command: &HistoryCommand) -> Result<()> {
  let mut client = client(cli, command.mine)?;
  let problem = resolve_problem(&mut client, &command.problem)?;
  let state = State::load(&config::expand_tilde(&cli.state_file));
  let local = if command.remote_only {
    Vec::new()
  } else {
    state
      .submissions
      .into_iter()
      .filter(|entry| entry.problem_id == problem.problem_id)
      .collect::<Vec<_>>()
  };

  let remote = if command.local_only {
    None
  } else {
    let user_id = if command.mine {
      client.user_id()
    } else {
      command.user.clone()
    };
    Some(fetch_remote_history(
      &mut client,
      &problem.problem_id,
      user_id.as_deref(),
      command.limit,
    )?)
  };
  output::print_history(remote.as_ref(), &local, cli.json)
}

fn run_ranking(cli: &Cli, command: &RankingCommand) -> Result<()> {
  let mut client = client(cli, command.mine)?;
  let problem = resolve_problem(&mut client, &command.problem)?;
  let ranks = parse_rank_selectors(&command.ranks)?;
  let ranking = if ranks.is_empty() {
    let should_scan = command.user.is_some() || command.submitter.is_some();
    if should_scan {
      fetch_ranking_pages(
        &mut client,
        &problem.problem_id,
        command.page,
        command.scan_pages.max(1),
        command.size.max(200),
      )?
    } else {
      fetch_ranking(&mut client, &problem.problem_id, command.page, command.size)?
    }
  } else {
    fetch_ranking_by_ranks(
      &mut client,
      &problem.problem_id,
      &ranks,
      command.size.max(200),
    )?
  };
  let filter_user = if command.mine {
    client.user_id()
  } else {
    command.user.clone()
  };
  output::print_ranking(
    &ranking,
    cli.json,
    filter_user.as_deref(),
    command.submitter.as_deref(),
    command.baseline,
  )
}

fn run_api(cli: &Cli, command: &ApiCommand) -> Result<()> {
  let mut client = client(cli, command.auth)?;
  let method = match command.method {
    ApiMethod::Get => Method::GET,
    ApiMethod::Post => Method::POST,
    ApiMethod::Put => Method::PUT,
    ApiMethod::Delete => Method::DELETE,
  };
  let data = command
    .data
    .as_ref()
    .map(|text| serde_json::from_str::<Value>(text))
    .transpose()?;
  let value = raw_api(&mut client, method, &command.path, data)?;
  print_json(&value)
}

fn client(cli: &Cli, auth_required: bool) -> Result<ApiClient> {
  let auth_path = config::expand_tilde(&cli.auth_cache);
  let auth = if auth_required {
    let options = login_options(cli);
    Some(load_or_login(&options, false)?)
  } else {
    Auth::load(&auth_path).ok()
  };
  let cache = if cli.no_cache {
    None
  } else {
    Some(client::CacheConfig {
      dir: config::expand_tilde(&cli.cache_dir),
      refresh: cli.refresh_cache,
      ttl: Duration::from_secs(cli.cache_ttl),
    })
  };
  ApiClient::new(cli.base_url.clone(), auth, Some(auth_path), cache)
}

fn login_options(cli: &Cli) -> LoginOptions {
  LoginOptions {
    base_url: cli.base_url.clone(),
    cdp_list_url: cli.cdp_list_url.clone(),
    chrome_bin: cli.chrome_bin.clone(),
    chrome_profile: config::expand_tilde(&cli.chrome_profile),
    auth_cache: config::expand_tilde(&cli.auth_cache),
    no_launch: false,
    login_timeout: Duration::from_secs(300),
    probe_interval: Duration::from_secs(2),
    debug: cli.debug,
  }
}

fn write_submission_code(value: &Value, out: &PathBuf) -> Result<usize> {
  if value.get("can_view_code").and_then(Value::as_bool) == Some(false) {
    anyhow::bail!(
      "CANNJudge did not allow viewing this submission code; re-login or check permission"
    );
  }
  let files = submission_files(value);
  if files.is_empty() {
    anyhow::bail!("submission response has no code files");
  }
  fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
  for (path, content) in &files {
    let target = util::safe_join(out, path)?;
    if let Some(parent) = target.parent() {
      fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&target, content).with_context(|| format!("write {}", target.display()))?;
  }
  fs::write(
    out.join(".cannjudge-submission.json"),
    serde_json::to_string_pretty(value)? + "\n",
  )
  .with_context(|| format!("write {}", out.join(".cannjudge-submission.json").display()))?;
  Ok(files.len())
}

fn submission_files(value: &Value) -> Vec<(String, String)> {
  let files = value
    .get("files")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|file| {
      let path = util::value_string(file, &["path", "key", "label"]);
      if path.is_empty() || path == "permission.txt" {
        return None;
      }
      Some((
        path,
        file
          .get("content")
          .and_then(Value::as_str)
          .unwrap_or("")
          .to_string(),
      ))
    })
    .collect::<Vec<_>>();
  if !files.is_empty() {
    return files;
  }

  [
    ("tiling.h", "tiling_h"),
    ("tiling_key.h", "tiling_key_h"),
    ("host.cpp", "host_cpp"),
    ("kernel.cpp", "kernel_cpp"),
  ]
  .into_iter()
  .filter_map(|(path, key)| {
    value.get(key).and_then(Value::as_str).and_then(|content| {
      if content.trim().is_empty() || content.contains("你没有权限查看") {
        None
      } else {
        Some((path.to_string(), content.to_string()))
      }
    })
  })
  .collect()
}
