use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_TARGET: &str = "claude";
const DEFAULT_TIMEOUT_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "stream-json" => {
                Err("tmuxlet v3 does not support --output-format stream-json yet".into())
            }
            other => Err(format!("unsupported output format: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
struct Options {
    print: bool,
    target: String,
    output_format: OutputFormat,
    cwd: PathBuf,
    prompt_parts: Vec<String>,
    model: Option<String>,
    resume: Option<Option<String>>,
    continue_latest: bool,
    session: Option<String>,
    session_id: Option<String>,
    system_prompt: Option<String>,
    append_system_prompts: Vec<String>,
    add_dirs: Vec<String>,
    dangerously_skip_permissions: bool,
    permission_mode: Option<String>,
    allowed_tools: Vec<String>,
    disallowed_tools: Vec<String>,
    tools: Vec<String>,
    mcp_configs: Vec<String>,
    settings: Option<String>,
    target_args: Vec<String>,
    timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            print: false,
            target: DEFAULT_TARGET.to_string(),
            output_format: OutputFormat::Text,
            cwd: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            prompt_parts: Vec::new(),
            model: None,
            resume: None,
            continue_latest: false,
            session: None,
            session_id: None,
            system_prompt: None,
            append_system_prompts: Vec::new(),
            add_dirs: Vec::new(),
            dangerously_skip_permissions: false,
            permission_mode: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            tools: Vec::new(),
            mcp_configs: Vec::new(),
            settings: None,
            target_args: Vec::new(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }
}

#[derive(Debug)]
struct RunPaths {
    root: PathBuf,
    meta: PathBuf,
    prompt: PathBuf,
    answer: PathBuf,
    complete: PathBuf,
    pane_log: PathBuf,
    error: PathBuf,
}

#[derive(Debug)]
struct RunResult {
    id: String,
    target: String,
    status: String,
    output: String,
    output_format: OutputFormat,
    cwd: PathBuf,
    tmux_session: String,
    completion_source: String,
    elapsed_ms: u128,
}

#[derive(Debug)]
struct TargetCommand {
    program: String,
    args: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("tmuxlet: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "-v" | "--version" => {
            println!("tmuxlet {VERSION}");
            Ok(())
        }
        "bridge" => handle_bridge(&args[1..]),
        "status" => handle_status(),
        "read" => handle_read(&args[1..]),
        "send" => handle_send(&args[1..]),
        "attach" => handle_attach(&args[1..]),
        "stop" => handle_stop(&args[1..]),
        _ => {
            let opts = parse_options(&mut args)?;
            if opts.print {
                let result = run_print(opts)?;
                emit_result(&result)?;
            } else {
                launch_interactive(opts)?;
            }
            Ok(())
        }
    }
}

fn parse_options(args: &mut [String]) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-p" | "--print" => opts.print = true,
            "--target" => opts.target = take_value(args, &mut i, arg)?,
            "--output-format" => {
                opts.output_format = OutputFormat::parse(&take_value(args, &mut i, arg)?)?;
            }
            "-C" | "--cwd" | "--cd" | "--dir" => {
                opts.cwd = expand_home(&take_value(args, &mut i, arg)?);
            }
            "-m" | "--model" => opts.model = Some(take_value(args, &mut i, arg)?),
            "-c" | "--continue" => opts.continue_latest = true,
            "-r" | "--resume" => {
                let value = optional_value(args, &mut i);
                opts.resume = Some(value);
            }
            "--session" => opts.session = Some(take_value(args, &mut i, arg)?),
            "--session-id" => opts.session_id = Some(take_value(args, &mut i, arg)?),
            "--system-prompt" => opts.system_prompt = Some(take_value(args, &mut i, arg)?),
            "--append-system-prompt" => {
                opts.append_system_prompts
                    .push(take_value(args, &mut i, arg)?);
            }
            "--add-dir" | "--include-directories" => {
                let value = take_value(args, &mut i, arg)?;
                opts.add_dirs.extend(split_csv(&value));
            }
            "--dangerously-skip-permissions"
            | "--yolo"
            | "--force"
            | "--dangerously-bypass-approvals-and-sandbox" => {
                opts.dangerously_skip_permissions = true;
            }
            "--permission-mode" => opts.permission_mode = Some(take_value(args, &mut i, arg)?),
            "--allowedTools" | "--allowed-tools" => {
                opts.allowed_tools.push(take_value(args, &mut i, arg)?);
            }
            "--disallowedTools" | "--disallowed-tools" => {
                opts.disallowed_tools.push(take_value(args, &mut i, arg)?);
            }
            "--tools" => opts.tools.push(take_value(args, &mut i, arg)?),
            "--mcp-config" => opts.mcp_configs.push(take_value(args, &mut i, arg)?),
            "--settings" => opts.settings = Some(take_value(args, &mut i, arg)?),
            "--target-arg" => opts.target_args.push(take_value(args, &mut i, arg)?),
            "--timeout" => {
                let seconds = take_value(args, &mut i, arg)?
                    .parse::<u64>()
                    .map_err(|_| "--timeout must be an integer number of seconds".to_string())?;
                opts.timeout = Duration::from_secs(seconds);
            }
            "--" => {
                opts.prompt_parts.extend(args[i + 1..].iter().cloned());
                break;
            }
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            _ => opts.prompt_parts.push(arg.clone()),
        }
        i += 1;
    }

    Ok(opts)
}

fn take_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn optional_value(args: &[String], i: &mut usize) -> Option<String> {
    if let Some(next) = args.get(*i + 1)
        && !next.starts_with('-')
    {
        *i += 1;
        return Some(next.clone());
    }
    None
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn prompt_from(opts: &Options) -> Result<String, String> {
    let joined = opts.prompt_parts.join(" ");
    if joined == "-" || joined.is_empty() {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| format!("failed to read stdin: {e}"))?;
        Ok(input)
    } else {
        Ok(joined)
    }
}

fn run_print(opts: Options) -> Result<RunResult, String> {
    let command = target_command(&opts)?;
    ensure_tmux()?;
    let started = Instant::now();
    let run_id = make_run_id();
    let tmux_session = format!("tmuxlet-{run_id}");
    let paths = run_paths(&run_id)?;
    fs::create_dir_all(&paths.root).map_err(|e| format!("failed to create run dir: {e}"))?;

    let prompt = prompt_from(&opts)?;
    fs::write(&paths.prompt, &prompt).map_err(|e| format!("failed to write prompt: {e}"))?;

    write_meta(&paths, &run_id, &tmux_session, &opts, &command)?;
    start_tmux_session(&tmux_session, &opts.cwd)?;
    send_launch_command(&tmux_session, &command)?;

    thread::sleep(Duration::from_millis(700));
    let instruction = completion_prompt(&prompt, &run_id, &paths.answer);
    paste_text(&tmux_session, &instruction)?;

    loop {
        if paths.complete.exists() {
            let output = fs::read_to_string(&paths.complete)
                .map_err(|e| format!("failed to read completion file: {e}"))?;
            capture_pane_to_file(&tmux_session, &paths.pane_log).ok();
            return Ok(RunResult {
                id: run_id,
                target: opts.target,
                status: "completed".into(),
                output,
                output_format: opts.output_format,
                cwd: opts.cwd,
                tmux_session,
                completion_source: "explicit".into(),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }

        if started.elapsed() >= opts.timeout {
            let pane = capture_pane(&tmux_session, 200).unwrap_or_default();
            fs::write(&paths.pane_log, &pane).ok();
            fs::write(&paths.error, "timeout waiting for explicit completion\n").ok();
            return Ok(RunResult {
                id: run_id,
                target: opts.target,
                status: "timeout".into(),
                output: pane,
                output_format: opts.output_format,
                cwd: opts.cwd,
                tmux_session,
                completion_source: "fallback".into(),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }

        if !tmux_session_exists(&tmux_session) {
            let pane = fs::read_to_string(&paths.pane_log).unwrap_or_default();
            return Ok(RunResult {
                id: run_id,
                target: opts.target,
                status: "exited".into(),
                output: pane,
                output_format: opts.output_format,
                cwd: opts.cwd,
                tmux_session,
                completion_source: "fallback".into(),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }

        thread::sleep(Duration::from_millis(500));
    }
}

fn launch_interactive(opts: Options) -> Result<(), String> {
    let command = target_command(&opts)?;
    ensure_tmux()?;
    let run_id = make_run_id();
    let tmux_session = format!("tmuxlet-{run_id}");
    let paths = run_paths(&run_id)?;
    fs::create_dir_all(&paths.root).map_err(|e| format!("failed to create run dir: {e}"))?;
    write_meta(&paths, &run_id, &tmux_session, &opts, &command)?;
    start_tmux_session(&tmux_session, &opts.cwd)?;
    send_launch_command(&tmux_session, &command)?;
    println!("{tmux_session}");
    Ok(())
}

fn target_command(opts: &Options) -> Result<TargetCommand, String> {
    validate_session_controls(opts)?;

    let target = opts.target.as_str();
    let mut cmd = match target {
        "claude" => claude_command(opts)?,
        "gemini" => gemini_command(opts)?,
        "codex" => codex_command(opts)?,
        "opencode" => opencode_command(opts)?,
        "pi" => pi_command(opts)?,
        "cursor" | "cursor-agent" => cursor_command(opts)?,
        other => return Err(format!("unsupported target: {other}")),
    };
    cmd.args.extend(opts.target_args.iter().cloned());
    Ok(cmd)
}

fn validate_session_controls(opts: &Options) -> Result<(), String> {
    let selectors = [
        ("--resume", opts.resume.is_some()),
        ("--session", opts.session.is_some()),
        ("--session-id", opts.session_id.is_some()),
    ];
    let selected = selectors
        .iter()
        .filter_map(|(flag, present)| present.then_some(*flag))
        .collect::<Vec<_>>();

    if opts.continue_latest && !selected.is_empty() {
        return Err(format!(
            "--continue cannot be combined with {}",
            selected.join(", ")
        ));
    }

    if selected.len() > 1 {
        return Err(format!(
            "session selectors are mutually exclusive: {}",
            selected.join(", ")
        ));
    }

    Ok(())
}

fn claude_command(opts: &Options) -> Result<TargetCommand, String> {
    let mut args = Vec::new();
    push_opt(&mut args, "--model", &opts.model);
    push_opt(&mut args, "--permission-mode", &opts.permission_mode);
    if opts.dangerously_skip_permissions {
        args.push("--dangerously-skip-permissions".into());
    }
    push_opt(&mut args, "--system-prompt", &opts.system_prompt);
    for value in &opts.append_system_prompts {
        args.extend(["--append-system-prompt".into(), value.clone()]);
    }
    for value in &opts.add_dirs {
        args.extend(["--add-dir".into(), value.clone()]);
    }
    for value in &opts.allowed_tools {
        args.extend(["--allowed-tools".into(), value.clone()]);
    }
    for value in &opts.disallowed_tools {
        args.extend(["--disallowed-tools".into(), value.clone()]);
    }
    for value in &opts.tools {
        args.extend(["--tools".into(), value.clone()]);
    }
    for value in &opts.mcp_configs {
        args.extend(["--mcp-config".into(), value.clone()]);
    }
    push_opt(&mut args, "--settings", &opts.settings);
    if opts.continue_latest {
        args.push("--continue".into());
    }
    if let Some(value) = &opts.resume {
        args.push("--resume".into());
        if let Some(value) = value {
            args.push(value.clone());
        }
    }
    push_opt(&mut args, "--session-id", &opts.session_id);
    if opts.session.is_some() {
        unsupported("claude", "--session", "use --session-id or --resume")?;
    }
    Ok(TargetCommand {
        program: "claude".into(),
        args,
    })
}

fn gemini_command(opts: &Options) -> Result<TargetCommand, String> {
    reject_any("gemini", "--system-prompt", &opts.system_prompt)?;
    reject_nonempty(
        "gemini",
        "--append-system-prompt",
        &opts.append_system_prompts,
    )?;
    reject_nonempty("gemini", "--allowed-tools/--tools", &opts.allowed_tools)?;
    reject_nonempty("gemini", "--disallowed-tools", &opts.disallowed_tools)?;
    reject_nonempty("gemini", "--tools", &opts.tools)?;
    reject_nonempty("gemini", "--mcp-config", &opts.mcp_configs)?;
    reject_any("gemini", "--settings", &opts.settings)?;
    reject_any("gemini", "--permission-mode", &opts.permission_mode)?;
    reject_any("gemini", "--session-id", &opts.session_id)?;
    reject_any("gemini", "--session", &opts.session)?;

    let mut args = Vec::new();
    push_opt(&mut args, "--model", &opts.model);
    if opts.dangerously_skip_permissions {
        args.push("--yolo".into());
    }
    for value in &opts.add_dirs {
        args.extend(["--include-directories".into(), value.clone()]);
    }
    if opts.continue_latest {
        args.extend(["--resume".into(), "latest".into()]);
    }
    if let Some(value) = &opts.resume {
        args.push("--resume".into());
        args.push(value.clone().unwrap_or_else(|| "latest".into()));
    }
    Ok(TargetCommand {
        program: "gemini".into(),
        args,
    })
}

fn codex_command(opts: &Options) -> Result<TargetCommand, String> {
    reject_any("codex", "--system-prompt", &opts.system_prompt)?;
    reject_nonempty(
        "codex",
        "--append-system-prompt",
        &opts.append_system_prompts,
    )?;
    reject_nonempty("codex", "--allowed-tools", &opts.allowed_tools)?;
    reject_nonempty("codex", "--disallowed-tools", &opts.disallowed_tools)?;
    reject_nonempty("codex", "--tools", &opts.tools)?;
    reject_nonempty("codex", "--mcp-config", &opts.mcp_configs)?;
    reject_any("codex", "--settings", &opts.settings)?;
    reject_any("codex", "--permission-mode", &opts.permission_mode)?;
    reject_any("codex", "--session", &opts.session)?;

    let mut args = Vec::new();
    if opts.continue_latest {
        args.extend(["resume".into(), "--last".into()]);
    } else if let Some(value) = &opts.resume {
        args.push("resume".into());
        if let Some(value) = value {
            args.push(value.clone());
        } else {
            args.push("--last".into());
        }
    } else if let Some(value) = &opts.session_id {
        args.extend(["resume".into(), value.clone()]);
    }
    push_opt(&mut args, "--model", &opts.model);
    args.extend(["--cd".into(), opts.cwd.display().to_string()]);
    for value in &opts.add_dirs {
        args.extend(["--add-dir".into(), value.clone()]);
    }
    if opts.dangerously_skip_permissions {
        args.push("--dangerously-bypass-approvals-and-sandbox".into());
    }
    Ok(TargetCommand {
        program: "codex".into(),
        args,
    })
}

fn opencode_command(opts: &Options) -> Result<TargetCommand, String> {
    reject_any("opencode", "--system-prompt", &opts.system_prompt)?;
    reject_nonempty(
        "opencode",
        "--append-system-prompt",
        &opts.append_system_prompts,
    )?;
    reject_nonempty("opencode", "--allowed-tools", &opts.allowed_tools)?;
    reject_nonempty("opencode", "--disallowed-tools", &opts.disallowed_tools)?;
    reject_nonempty("opencode", "--tools", &opts.tools)?;
    reject_nonempty("opencode", "--mcp-config", &opts.mcp_configs)?;
    reject_any("opencode", "--settings", &opts.settings)?;
    reject_any("opencode", "--permission-mode", &opts.permission_mode)?;
    reject_nonempty("opencode", "--add-dir", &opts.add_dirs)?;

    let mut args = Vec::new();
    push_opt(&mut args, "--model", &opts.model);
    if opts.continue_latest {
        args.push("--continue".into());
    }
    push_opt(&mut args, "--session", &opts.session);
    push_opt(&mut args, "--session", &opts.session_id);
    if let Some(value) = &opts.resume {
        args.push("--session".into());
        if let Some(value) = value {
            args.push(value.clone());
        } else {
            return Err(
                "opencode --resume requires a session id; use --continue for latest".into(),
            );
        }
    }
    args.extend(["--dir".into(), opts.cwd.display().to_string()]);
    if opts.dangerously_skip_permissions {
        args.push("--dangerously-skip-permissions".into());
    }
    Ok(TargetCommand {
        program: "opencode".into(),
        args,
    })
}

fn pi_command(opts: &Options) -> Result<TargetCommand, String> {
    reject_any("pi", "--permission-mode", &opts.permission_mode)?;
    reject_nonempty("pi", "--mcp-config", &opts.mcp_configs)?;
    reject_any("pi", "--settings", &opts.settings)?;
    reject_nonempty("pi", "--add-dir", &opts.add_dirs)?;
    reject_nonempty("pi", "--disallowed-tools", &opts.disallowed_tools)?;

    let mut args = Vec::new();
    push_opt(&mut args, "--model", &opts.model);
    push_opt(&mut args, "--system-prompt", &opts.system_prompt);
    for value in &opts.append_system_prompts {
        args.extend(["--append-system-prompt".into(), value.clone()]);
    }
    for value in &opts.tools {
        args.extend(["--tools".into(), value.clone()]);
    }
    for value in &opts.allowed_tools {
        args.extend(["--tools".into(), value.clone()]);
    }
    if opts.continue_latest {
        args.push("--continue".into());
    }
    push_opt(&mut args, "--session", &opts.session);
    push_opt(&mut args, "--session", &opts.session_id);
    if let Some(value) = &opts.resume {
        args.push("--resume".into());
        if let Some(value) = value {
            args.push(value.clone());
        }
    }
    if opts.dangerously_skip_permissions {
        return Err("pi has no direct --dangerously-skip-permissions/--yolo equivalent; use --target-arg for explicit pi flags".into());
    }
    Ok(TargetCommand {
        program: "pi".into(),
        args,
    })
}

fn cursor_command(opts: &Options) -> Result<TargetCommand, String> {
    reject_any("cursor-agent", "--system-prompt", &opts.system_prompt)?;
    reject_nonempty(
        "cursor-agent",
        "--append-system-prompt",
        &opts.append_system_prompts,
    )?;
    reject_nonempty("cursor-agent", "--allowed-tools", &opts.allowed_tools)?;
    reject_nonempty("cursor-agent", "--disallowed-tools", &opts.disallowed_tools)?;
    reject_nonempty("cursor-agent", "--tools", &opts.tools)?;
    reject_nonempty("cursor-agent", "--mcp-config", &opts.mcp_configs)?;
    reject_any("cursor-agent", "--settings", &opts.settings)?;
    reject_any("cursor-agent", "--permission-mode", &opts.permission_mode)?;
    reject_nonempty("cursor-agent", "--add-dir", &opts.add_dirs)?;
    reject_any("cursor-agent", "--session", &opts.session)?;

    let mut args = Vec::new();
    push_opt(&mut args, "--model", &opts.model);
    if let Some(value) = &opts.resume {
        args.push("--resume".into());
        if let Some(value) = value {
            args.push(value.clone());
        }
    }
    push_opt(&mut args, "--resume", &opts.session_id);
    if opts.continue_latest {
        args.push("resume".into());
    }
    if opts.dangerously_skip_permissions {
        args.push("--force".into());
    }
    Ok(TargetCommand {
        program: "cursor-agent".into(),
        args,
    })
}

fn push_opt(args: &mut Vec<String>, flag: &str, value: &Option<String>) {
    if let Some(value) = value {
        args.extend([flag.into(), value.clone()]);
    }
}

fn reject_any<T>(target: &str, flag: &str, value: &Option<T>) -> Result<(), String> {
    if value.is_some() {
        unsupported(target, flag, "")
    } else {
        Ok(())
    }
}

fn reject_nonempty<T>(target: &str, flag: &str, values: &[T]) -> Result<(), String> {
    if values.is_empty() {
        Ok(())
    } else {
        unsupported(target, flag, "")
    }
}

fn unsupported(target: &str, flag: &str, hint: &str) -> Result<(), String> {
    let suffix = if hint.is_empty() {
        String::new()
    } else {
        format!(" ({hint})")
    };
    Err(format!(
        "{target} does not support normalized flag {flag}{suffix}"
    ))
}

fn completion_prompt(prompt: &str, run_id: &str, answer_path: &Path) -> String {
    let exe = env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "tmuxlet".into());
    format!(
        "{prompt}\n\n\
TMUXLET COMPLETION CONTRACT:\n\
When you have the final response ready, write only that final response to this file:\n\
{answer}\n\n\
Then run this exact command:\n\
{exe} bridge complete --run {run_id} --file {answer}\n\n\
Do not wait for the user after the final response is written.",
        answer = answer_path.display(),
    )
}

fn start_tmux_session(name: &str, cwd: &Path) -> Result<(), String> {
    run_tmux([
        OsStr::new("new-session"),
        OsStr::new("-d"),
        OsStr::new("-s"),
        OsStr::new(name),
        OsStr::new("-x"),
        OsStr::new("220"),
        OsStr::new("-y"),
        OsStr::new("50"),
        OsStr::new("-c"),
        cwd.as_os_str(),
    ])
    .map(|_| ())
}

fn send_launch_command(tmux_session: &str, command: &TargetCommand) -> Result<(), String> {
    let mut parts = Vec::with_capacity(command.args.len() + 1);
    parts.push(shell_quote(&command.program));
    for arg in &command.args {
        parts.push(shell_quote(arg));
    }
    let line = parts.join(" ");
    run_tmux([
        OsStr::new("send-keys"),
        OsStr::new("-t"),
        OsStr::new(tmux_session),
        OsStr::new(&line),
        OsStr::new("Enter"),
    ])
    .map(|_| ())
}

fn paste_text(tmux_session: &str, text: &str) -> Result<(), String> {
    let tmp = env::temp_dir().join(format!("tmuxlet-paste-{}.txt", make_run_id()));
    fs::write(&tmp, text).map_err(|e| format!("failed to stage prompt: {e}"))?;
    let load = run_tmux([OsStr::new("load-buffer"), tmp.as_os_str()]);
    fs::remove_file(&tmp).ok();
    load?;
    run_tmux([
        OsStr::new("paste-buffer"),
        OsStr::new("-t"),
        OsStr::new(tmux_session),
    ])?;
    thread::sleep(Duration::from_millis(200));
    run_tmux([
        OsStr::new("send-keys"),
        OsStr::new("-t"),
        OsStr::new(tmux_session),
        OsStr::new("Enter"),
    ])
    .map(|_| ())
}

fn tmux_session_exists(name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn capture_pane(tmux_session: &str, lines: usize) -> Result<String, String> {
    let start = format!("-{lines}");
    run_tmux([
        OsStr::new("capture-pane"),
        OsStr::new("-t"),
        OsStr::new(tmux_session),
        OsStr::new("-p"),
        OsStr::new("-S"),
        OsStr::new(&start),
    ])
}

fn capture_pane_to_file(tmux_session: &str, path: &Path) -> Result<(), String> {
    let output = capture_pane(tmux_session, 500)?;
    fs::write(path, output).map_err(|e| format!("failed to write pane log: {e}"))
}

fn run_tmux<I, S>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute tmux: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn ensure_tmux() -> Result<(), String> {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map_err(|_| "tmux is required but was not found on PATH".to_string())
        .and_then(|output| {
            if output.status.success() {
                Ok(())
            } else {
                Err("tmux is required but did not run successfully".into())
            }
        })
}

fn handle_bridge(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) != Some("complete") {
        return Err("usage: tmuxlet bridge complete --run <id> --file <path>".into());
    }
    let mut run_id = None;
    let mut file = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--run" => {
                i += 1;
                run_id = args.get(i).cloned();
            }
            "--file" => {
                i += 1;
                file = args.get(i).map(PathBuf::from);
            }
            other => return Err(format!("unknown bridge option: {other}")),
        }
        i += 1;
    }
    let run_id = run_id.ok_or("--run is required")?;
    let file = file.ok_or("--file is required")?;
    let paths = run_paths(&run_id)?;
    let output =
        fs::read_to_string(&file).map_err(|e| format!("failed to read completion source: {e}"))?;
    fs::write(&paths.complete, output).map_err(|e| format!("failed to write completion: {e}"))?;
    Ok(())
}

fn handle_status() -> Result<(), String> {
    let root = state_dir().join("runs");
    if !root.exists() {
        println!("No runs.");
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| format!("failed to read runs: {e}"))? {
        let entry = entry.map_err(|e| format!("failed to read run: {e}"))?;
        let meta = entry.path().join("meta.json");
        if meta.exists() {
            println!("{}", fs::read_to_string(meta).unwrap_or_default().trim());
        }
    }
    Ok(())
}

fn handle_read(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("usage: tmuxlet read <id> [--lines N]")?;
    let mut lines = 80usize;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--lines" => {
                i += 1;
                lines = args
                    .get(i)
                    .ok_or("--lines requires a value")?
                    .parse()
                    .map_err(|_| "--lines must be an integer".to_string())?;
            }
            other => return Err(format!("unknown read option: {other}")),
        }
        i += 1;
    }
    print!("{}", capture_pane(&tmux_name_from_id(id), lines)?);
    Ok(())
}

fn handle_send(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("usage: tmuxlet send <id> <message>")?;
    let message = args.get(1..).unwrap_or_default().join(" ");
    if message.is_empty() {
        return Err("usage: tmuxlet send <id> <message>".into());
    }
    paste_text(&tmux_name_from_id(id), &message)
}

fn handle_attach(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("usage: tmuxlet attach <id>")?;
    let status = Command::new("tmux")
        .args(["attach-session", "-t", &tmux_name_from_id(id)])
        .status()
        .map_err(|e| format!("failed to attach tmux: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tmux attach exited with {status}"))
    }
}

fn handle_stop(args: &[String]) -> Result<(), String> {
    let id = args.first().ok_or("usage: tmuxlet stop <id>")?;
    let name = tmux_name_from_id(id);
    if tmux_session_exists(&name) {
        run_tmux([
            OsStr::new("send-keys"),
            OsStr::new("-t"),
            OsStr::new(&name),
            OsStr::new("/exit"),
            OsStr::new("Enter"),
        ])
        .ok();
        thread::sleep(Duration::from_secs(2));
        if tmux_session_exists(&name) {
            run_tmux([
                OsStr::new("kill-session"),
                OsStr::new("-t"),
                OsStr::new(&name),
            ])?;
        }
    }
    println!("Stopped {id}");
    Ok(())
}

fn tmux_name_from_id(id: &str) -> String {
    if id.starts_with("tmuxlet-") {
        id.to_string()
    } else {
        format!("tmuxlet-{id}")
    }
}

fn emit_result(result: &RunResult) -> Result<(), String> {
    match result.completion_source.as_str() {
        "explicit" => {}
        _ if result.status == "timeout" => {
            if result.completion_source == "fallback" && matches!(result.status.as_str(), "timeout")
            {
                // Still emit the captured output for integrations; the process exit communicates failure.
            }
        }
        _ => {}
    }

    match result_status_code(&result.status) {
        0 => {
            print_result(result);
            Ok(())
        }
        _ => {
            print_result(result);
            Err(format!(
                "run {} ended with status {}",
                result.id, result.status
            ))
        }
    }
}

fn print_result(result: &RunResult) {
    if result.output_format == OutputFormat::Json {
        println!("{}", result_json(result));
    } else {
        print!("{}", result.output);
        if !result.output.ends_with('\n') {
            println!();
        }
    }
}

fn result_status_code(status: &str) -> i32 {
    if status == "completed" { 0 } else { 1 }
}

fn write_meta(
    paths: &RunPaths,
    run_id: &str,
    tmux_session: &str,
    opts: &Options,
    command: &TargetCommand,
) -> Result<(), String> {
    let meta = format!(
        "{{\"id\":\"{}\",\"target\":\"{}\",\"tmuxSession\":\"{}\",\"cwd\":\"{}\",\"program\":\"{}\",\"args\":{},\"outputFormat\":\"{}\"}}\n",
        json_escape(run_id),
        json_escape(&opts.target),
        json_escape(tmux_session),
        json_escape(&opts.cwd.display().to_string()),
        json_escape(&command.program),
        json_array(&command.args),
        match opts.output_format {
            OutputFormat::Text => "text",
            OutputFormat::Json => "json",
        },
    );
    fs::write(&paths.meta, meta).map_err(|e| format!("failed to write metadata: {e}"))?;
    Ok(())
}

fn result_json(result: &RunResult) -> String {
    format!(
        "{{\"id\":\"{}\",\"target\":\"{}\",\"status\":\"{}\",\"output\":\"{}\",\"cwd\":\"{}\",\"tmuxSession\":\"{}\",\"completionSource\":\"{}\",\"elapsedMs\":{}}}",
        json_escape(&result.id),
        json_escape(&result.target),
        json_escape(&result.status),
        json_escape(&result.output),
        json_escape(&result.cwd.display().to_string()),
        json_escape(&result.tmux_session),
        json_escape(&result.completion_source),
        result.elapsed_ms,
    )
}

fn json_array(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|v| format!("\"{}\"", json_escape(v)))
        .collect::<Vec<_>>()
        .join(",");
    format!("[{items}]")
}

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn state_dir() -> PathBuf {
    env::var_os("TMUXLET_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".tmuxlet")))
        .unwrap_or_else(|| PathBuf::from(".tmuxlet"))
}

fn run_paths(run_id: &str) -> Result<RunPaths, String> {
    if run_id.contains('/') || run_id.contains('\\') || run_id.contains("..") {
        return Err("invalid run id".into());
    }
    let root = state_dir().join("runs").join(run_id);
    Ok(RunPaths {
        meta: root.join("meta.json"),
        prompt: root.join("prompt.txt"),
        answer: root.join("answer.txt"),
        complete: root.join("complete.txt"),
        pane_log: root.join("pane.log"),
        error: root.join("error.txt"),
        root,
    })
}

fn make_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{:x}", nanos, std::process::id())
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value))
    } else if let Some(rest) = value.strip_prefix("~/") {
        env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| PathBuf::from(value))
    } else {
        PathBuf::from(value)
    }
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        "''".into()
    } else if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@%+".contains(c))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn print_help() {
    println!(
        "tmuxlet {VERSION}\n\n\
Usage:\n  tmuxlet [options] [prompt]\n  tmuxlet -p [options] [prompt]\n  tmuxlet <status|read|send|attach|stop>\n\n\
Options:\n  -p, --print                         Blocking programmatic mode\n      --target <name>                  Target CLI: claude, gemini, codex, opencode, pi, cursor\n      --output-format <text|json>      Output format for tmuxlet result\n  -C, --cwd <dir>                      Working directory (--cd and --dir aliases)\n  -m, --model <model>                  Model pass-through where supported\n  -c, --continue                       Continue latest session where supported\n  -r, --resume [id]                    Resume session where supported\n      --session <id>                   Normalized session selector\n      --session-id <id>                Explicit normalized session id\n      --dangerously-skip-permissions   Normalized bypass flag (--yolo, --force aliases)\n      --target-arg <arg>               Raw target CLI escape hatch\n      --timeout <seconds>              Print-mode timeout (default: 1800)\n\n\
Examples:\n  tmuxlet -p \"say ready\"\n  tmuxlet --target codex -p --cwd /tmp \"say ready\"\n  printf 'say ready' | tmuxlet -p -"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> Options {
        let mut args = argv.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        parse_options(&mut args).unwrap()
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == flag && window[1] == value)
    }

    #[test]
    fn parses_print_target_and_prompt() {
        let opts = parse(&["--target", "codex", "-p", "hello"]);
        assert!(opts.print);
        assert_eq!(opts.target, "codex");
        assert_eq!(opts.prompt_parts, vec!["hello"]);
    }

    #[test]
    fn parses_cwd_aliases_and_bypass_alias() {
        let opts = parse(&["--dir", "/tmp", "--yolo", "-p", "hello"]);
        assert_eq!(opts.cwd, PathBuf::from("/tmp"));
        assert!(opts.dangerously_skip_permissions);
    }

    #[test]
    fn parses_session_controls() {
        let opts = parse(&["--continue", "-p", "x"]);
        assert!(opts.continue_latest);

        let opts = parse(&["--resume", "-p", "x"]);
        assert_eq!(opts.resume, Some(None));

        let opts = parse(&["--resume", "abc", "-p", "x"]);
        assert_eq!(opts.resume, Some(Some("abc".into())));

        let opts = parse(&["--session", "abc", "-p", "x"]);
        assert_eq!(opts.session, Some("abc".into()));

        let opts = parse(&["--session-id", "abc", "-p", "x"]);
        assert_eq!(opts.session_id, Some("abc".into()));
    }

    #[test]
    fn rejects_conflicting_session_controls() {
        let opts = parse(&["--continue", "--resume", "abc", "-p", "x"]);
        assert!(target_command(&opts).is_err());

        let opts = parse(&["--resume", "abc", "--session-id", "def", "-p", "x"]);
        assert!(target_command(&opts).is_err());

        let opts = parse(&["--session", "abc", "--session-id", "def", "-p", "x"]);
        assert!(target_command(&opts).is_err());
    }

    #[test]
    fn claude_maps_session_controls_to_native_flags() {
        let opts = parse(&["--continue", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(cmd.args.contains(&"--continue".to_string()));

        let opts = parse(&["--resume", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--resume", "abc"));

        let opts = parse(&["--session-id", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--session-id", "abc"));
    }

    #[test]
    fn gemini_maps_resume_latest() {
        let opts = parse(&["--target", "gemini", "--continue", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--resume", "latest"));

        let opts = parse(&["--target", "gemini", "--resume", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--resume", "latest"));

        let opts = parse(&["--target", "gemini", "--resume", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--resume", "abc"));
    }

    #[test]
    fn codex_maps_session_id_to_resume() {
        let opts = parse(&["--target", "codex", "--continue", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert_eq!(cmd.args[0], "resume");
        assert_eq!(cmd.args[1], "--last");

        let opts = parse(&["--target", "codex", "--resume", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert_eq!(cmd.args[0], "resume");
        assert_eq!(cmd.args[1], "abc");

        let opts = parse(&["--target", "codex", "--session-id", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert_eq!(cmd.args[0], "resume");
        assert_eq!(cmd.args[1], "abc");
    }

    #[test]
    fn opencode_maps_explicit_session_ids() {
        let opts = parse(&["--target", "opencode", "--continue", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(cmd.args.contains(&"--continue".to_string()));

        let opts = parse(&["--target", "opencode", "--resume", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--session", "abc"));

        let opts = parse(&["--target", "opencode", "--session-id", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--session", "abc"));
    }

    #[test]
    fn pi_maps_session_id_to_session() {
        let opts = parse(&["--target", "pi", "--session-id", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--session", "abc"));

        let opts = parse(&["--target", "pi", "--resume", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--resume", "abc"));
    }

    #[test]
    fn cursor_maps_session_controls() {
        let opts = parse(&["--target", "cursor", "--continue", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(cmd.args.contains(&"resume".to_string()));

        let opts = parse(&["--target", "cursor", "--session-id", "abc", "-p", "x"]);
        let cmd = target_command(&opts).unwrap();
        assert!(has_pair(&cmd.args, "--resume", "abc"));
    }

    #[test]
    fn claude_maps_bypass_to_claude_flag() {
        let opts = parse(&[
            "--dangerously-skip-permissions",
            "--model",
            "sonnet",
            "-p",
            "x",
        ]);
        let cmd = claude_command(&opts).unwrap();
        assert!(
            cmd.args
                .contains(&"--dangerously-skip-permissions".to_string())
        );
        assert!(cmd.args.contains(&"sonnet".to_string()));
    }

    #[test]
    fn gemini_maps_bypass_to_yolo() {
        let opts = parse(&["--target", "gemini", "--force", "-p", "x"]);
        let cmd = gemini_command(&opts).unwrap();
        assert!(cmd.args.contains(&"--yolo".to_string()));
    }

    #[test]
    fn codex_maps_bypass_to_codex_flag() {
        let opts = parse(&["--target", "codex", "--yolo", "-p", "x"]);
        let cmd = codex_command(&opts).unwrap();
        assert!(
            cmd.args
                .contains(&"--dangerously-bypass-approvals-and-sandbox".to_string())
        );
    }

    #[test]
    fn rejects_unsupported_flags() {
        let opts = parse(&["--target", "gemini", "--system-prompt", "sys", "-p", "x"]);
        assert!(gemini_command(&opts).is_err());
    }

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
