# cannjudge-cli

Rust CLI for shortcut access to CANNJudge operations.

## Build

```bash
cargo build --release
```

The binary is `target/release/cannjudge`.

## Auth

The login flow follows the browser/CDP style used by `gitcode-jupyter-tool`:

```bash
cannjudge auth login
```

If no valid cache exists, the tool launches Chrome with a dedicated profile and polls
`localStorage.cannjudge_user` from `https://cannjudge.cn`. Finish login in the browser;
the CLI then saves credentials to:

```text
${XDG_CONFIG_HOME:-~/.config}/cannjudge-cli/auth.json
```

The browser profile is kept in the cache directory:

```text
${XDG_CACHE_HOME:-~/.cache}/cannjudge-cli/chrome-profile
```

Accounts are independent. Select one with `--account` (or `CANNJUDGE_ACCOUNT`):

```bash
cannjudge accounts list
cannjudge --account default auth login
cannjudge --account work auth login
cannjudge --account work problem info https://cannjudge.cn/public/s1/addcmul
cannjudge --account work auth status
```

Every account gets its own auth cache, Chrome profile, and Chrome DevTools port.
The CDP port is selected at runtime from `61600–61799` and persisted in the
account config. Let the CLI manage this port per account.

Removing the Chrome profile only requires logging in again; the auth cache remains
under the config directory.

Useful auth commands:

```bash
cannjudge auth status
cannjudge auth logout
```

## Daily submission quota

Query the selected account's submissions for the current Beijing calendar day.
The daily limit is 50 submissions and resets at 00:00 Beijing time. The command
always fetches the submission list from CANNJudge instead of using the GET cache:

```bash
cannjudge --account default quota
cannjudge --account work quota
cannjudge --account work --json quota
```

Use `accounts list` to see which named accounts are available, then select the
same account with `--account` for quota checks and submissions. The JSON result
contains `used_today`, `daily_limit`, and `remaining_today`.

## Problems

All problem commands accept either a CANNJudge URL or an object id:

```bash
cannjudge problem info https://cannjudge.cn/public/s1/addcmul
cannjudge problem info https://cannjudge.cn/cann_2026_xjd/xjd_2026_0613/selugrad
```

Fetch problem statement/content as Markdown:

```bash
cannjudge problem content https://cannjudge.cn/public/s1/addcmul
cannjudge problem content https://cannjudge.cn/public/s1/addcmul -o addcmul.md
```

Download editable template files into a local folder:

```bash
cannjudge problem template https://cannjudge.cn/public/s1/addcmul -o addcmul
```

Download the platform zip package instead:

```bash
cannjudge problem template https://cannjudge.cn/public/s1/addcmul -o . --zip
```

## Contests

Contest commands accept contest URLs such as `https://cannjudge.cn/public/s1/`
or `https://cannjudge.cn/cann_2026_xjd/xjd_2026_0613`.

```bash
cannjudge contest info https://cannjudge.cn/public/s1/
cannjudge contest problems https://cannjudge.cn/public/s1/
cannjudge contest problems https://cannjudge.cn/cann_2026_xjd/xjd_2026_0613
```

## Submit

CANNJudge submissions are made by reading the problem template, taking only editable
files, replacing their contents from the local folder, and POSTing the same payload
the web editor uses.

```bash
cannjudge submit https://cannjudge.cn/public/s1/addcmul ./addcmul
```

Queue and retry when the platform says submissions are too frequent:

```bash
cannjudge submit https://cannjudge.cn/public/s1/addcmul ./addcmul --queue --max-wait 3600
```

Submit and watch until the judge reaches a terminal status:

```bash
cannjudge submit https://cannjudge.cn/public/s1/addcmul ./addcmul --watch
```

If the platform says the daily submission count is exhausted, the CLI cancels that
submit attempt instead of queueing.

## Submissions

Fetch status and per-testcase details:

```bash
cannjudge submission status https://cannjudge.cn/public/s1/addcmul/submission/6a4bcd7ae942bddcf7b0fd0d
```

The default output includes:

- submission status
- testcase status
- runtime
- best time
- `precision_ratio` / pass ratio returned by CANNJudge
- first line of testcase log or error message

Show full testcase logs for compile/runtime errors:

```bash
cannjudge submission status <submission-id> --logs
```

Poll until done:

```bash
cannjudge submission status <submission-id> --watch
```

Download viewable submission code to a folder:

```bash
cannjudge submission download <submission-id> -o ./submission-code
```

Download the platform submission package:

```bash
cannjudge submission download <submission-id> -o . --zip
```

## History

Remote history:

```bash
cannjudge history https://cannjudge.cn/public/s1/addcmul --limit 20
```

Only your remote history:

```bash
cannjudge history https://cannjudge.cn/public/s1/addcmul --mine
```

Local submissions made by this CLI are also stored in:

```text
${XDG_CONFIG_HOME:-~/.config}/cannjudge-cli/state.json
```

## Ranking

Problem ranking:

```bash
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --page 1 --size 50
```

Only yourself:

```bash
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --mine
```

Specified user id or nickname substring:

```bash
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --user 6a2182cf05b7e33341306d91
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --user chenying
```

Specified submitter id or nickname substring:

```bash
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --submitter chenying
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --submitter chenying --scan-pages 3
```

Baseline testcase data:

```bash
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --baseline
```

Specific rank positions or ranges:

```bash
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --rank 1
cannjudge ranking https://cannjudge.cn/public/s1/addcmul --rank 1,3,10-20
```

When `--rank` is used, the CLI fetches larger ranking pages by default so nearby
rank data is cached together instead of requesting one row at a time.

## Cache

GET JSON responses are cached by default in:

```text
${XDG_CACHE_HOME:-~/.cache}/cannjudge-cli
```

Use this to reduce repeated requests to CANNJudge. POST submissions, zip downloads,
and non-terminal submission polling are not cached.

Useful cache flags:

```bash
cannjudge --refresh-cache contest problems https://cannjudge.cn/public/s1/
cannjudge --no-cache ranking https://cannjudge.cn/public/s1/addcmul
cannjudge --cache-ttl 86400 problem content https://cannjudge.cn/public/s1/addcmul
```

## Raw JSON

Most commands support `--json` and return the raw CANNJudge response or a thin
wrapper around it. Use this when you want every available field, including fields
not summarized by the human-readable output.

```bash
cannjudge --json submission status <submission-id>
cannjudge --json ranking https://cannjudge.cn/public/s1/addcmul
```

There is also a raw API escape hatch:

```bash
cannjudge api get /api/groups/public
cannjudge api post /api/submissions/submit --auth --data '{"problemId":"..."}'
```
