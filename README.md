# tmuxlet

`tmuxlet` runs coding CLIs inside tmux while exposing a small print-mode
programmatic interface.

```bash
tmuxlet -p "say ready"
tmuxlet --target claude -p --output-format json "say ready"
tmuxlet --target codex -p --cwd /tmp "say ready"
printf "say ready" | tmuxlet -p -
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
up to three times.

If a run stalls later on a permission or confirmation prompt, tmuxlet returns a
nonzero `blocked` status with the captured pane output. Rerun with
`--dangerously-skip-permissions` where supported, or inspect the run with
`tmuxlet attach <id>`.

## Development

```bash
cargo test
cargo build
```
