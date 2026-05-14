# tmuxlet

`tmuxlet` runs coding CLIs inside tmux while exposing a small print-mode
programmatic interface.

```bash
tmuxlet -p "say ready"
tmuxlet -p --target claude --output-format json "say ready"
tmuxlet -p --target codex --cwd /tmp "say ready"
printf "say ready" | tmuxlet -p -
```

## Install

Requirements:

- Rust and Cargo
- `tmux` on `PATH`
- At least one supported target CLI, such as `claude` or `codex`

Install from GitHub:

```bash
cargo install --git https://github.com/CodefiLabs/tmuxlet --force
tmuxlet --version
tmuxlet -p "say ready"
```

Install from a local checkout:

```bash
cargo install --path . --force
tmuxlet --version
```

During development, run without installing:

```bash
cargo run -- -p "say ready"
```

## Why

Some coding CLIs have useful behavior tied to a user's local subscription, auth
state, tools, and terminal session. `tmuxlet` keeps that runtime in tmux and
gives callers a blocking `-p` style interface for OpenClaw, Paperclip, scripts,
and other local automation.

## Targets

Initial adapters:

- `claude` (default)
- `gemini`
- `codex`
- `opencode`
- `pi`
- `cursor` / `cursor-agent`

Unsupported normalized flags fail fast instead of being ignored. Use
`--target-arg <arg>` for rare target-specific escape hatches.

## CLI

```bash
tmuxlet -p [options] [prompt]
tmuxlet status
tmuxlet read <id> [--lines N]
tmuxlet send <id> <message>
tmuxlet attach <id>
tmuxlet stop <id>
```

Important options:

- `-p, --print`: blocking programmatic mode
- `--target <name>`: target CLI, default `claude`
- `--output-format text|json`: output format
- `-C, --cwd <dir>`: working directory; aliases `--cd`, `--dir`
- `--model <model>`: target model where supported
- `-c, --continue`: continue the latest session where supported
- `-r, --resume [id]`: resume a session by id, or latest/picker where supported
- `--session-id <id>`: explicit normalized session id selector
- `--dangerously-skip-permissions`: normalized bypass flag; aliases `--yolo`,
  `--force`, `--dangerously-bypass-approvals-and-sandbox`
- `--timeout <seconds>`: print-mode timeout, default 1800

Non-print prompt launches are intentionally unsupported; pass `-p` for runs.

Session controls are mutually exclusive. Use one of `--continue`, `--resume`,
`--session`, or `--session-id` per launch.

Examples:

```bash
tmuxlet -p --target claude --continue "summarize the last task"
tmuxlet -p --target codex --session-id 01984d2f-... "continue the fix"
tmuxlet -p --target opencode --resume ses_abc123 "check status"
```

When running through Cargo during development, Cargo's build/status lines are
separate from tmuxlet output:

```bash
cargo run -- -p "hello"
```

The model response is tmuxlet's stdout. Lines such as `Compiling` and `Running`
come from Cargo.

## How Print Mode Works

For Claude and Codex, tmuxlet passes one full prompt in the launch command. The
target-visible prompt is ordered as:

```text
<user prompt>

TMUXLET COMPLETION CONTRACT:
...
```

The completion contract tells the target CLI to write its final response to
`answer.txt` and then run `tmuxlet bridge complete`. Other targets currently use
a paste fallback when tmuxlet cannot safely assume positional prompt support.

## Runtime State

Runtime files live in:

```text
~/.tmuxlet/runs/<run-id>/
  meta.json
  prompt.txt
  answer.txt
  complete.txt
  pane.log
  error.txt
```

Set `TMUXLET_HOME` to override the state directory.

## Confirmation Handling

For Claude and Codex, tmuxlet passes the full prompt in the launch command with
the user's prompt first and tmuxlet's completion contract after it. During the
startup window, tmuxlet watches for common confirmation gates and sends `Enter`
up to three times. This startup check runs during normal print-mode polling, so
successful runs can return as soon as the completion file appears.

If a run stalls later on a permission or confirmation prompt, tmuxlet returns a
nonzero `blocked` status with the captured pane output. Rerun with
`--dangerously-skip-permissions` where supported, or inspect the run with
`tmuxlet attach <id>`.

## Output And Failures

Text output prints only the target's final response when the completion contract
is satisfied. JSON output includes the run id, target, status, output, cwd, tmux
session, completion source, and elapsed time.

Nonzero statuses:

- `blocked`: the pane stopped changing and appears to be waiting on a
  permission or confirmation prompt.
- `timeout`: the target did not satisfy the completion contract before
  `--timeout`.
- `exited`: the tmux session ended before completion.

On `blocked` and `timeout`, tmuxlet writes `pane.log` and `error.txt` under the
run directory for debugging.

## Development

```bash
cargo test
cargo build
```
