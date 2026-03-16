mod diff;
mod overlay;
mod sandbox;

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(name = "slopwrap", about = "Sandbox AI tools with bubblewrap", trailing_var_arg = true)]
struct Cli {
    /// Disable network access inside the sandbox.
    #[arg(long)]
    no_net: bool,

    /// Use a specific directory for the overlay instead of a tempdir.
    #[arg(long)]
    overlay_dir: Option<PathBuf>,

    /// Directory where sandbox writes land (overlayfs upper). Overrides overlay-dir layout.
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Overlayfs workdir. Overrides overlay-dir layout.
    #[arg(long)]
    work_dir: Option<PathBuf>,

    /// Keep the overlay directory after exit (default on Ctrl-C).
    #[arg(long)]
    keep: bool,

    /// Bind-mount a path read-only into the sandbox (repeatable).
    #[arg(long = "ro-bind", value_name = "PATH")]
    ro_binds: Vec<PathBuf>,

    /// Bind-mount a path read-write into the sandbox (repeatable).
    #[arg(long = "bind", value_name = "PATH")]
    rw_binds: Vec<PathBuf>,

    /// Bind ~/.claude (rw) for Claude Code support.
    #[arg(long)]
    claude: bool,

    /// Command and arguments to run inside the sandbox.
    #[arg(required = true)]
    command: Vec<String>,
}

fn find_repo_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let path = String::from_utf8(o.stdout)
                .context("git output not utf8")?
                .trim()
                .to_string();
            Ok(PathBuf::from(path))
        }
        _ => {
            // Fallback to $PWD
            std::env::current_dir().context("getting current directory")
        }
    }
}

fn check_bwrap() -> Result<()> {
    let status = Command::new("bwrap").arg("--version").output();
    match status {
        Ok(o) if o.status.success() => Ok(()),
        _ => bail!("bwrap not found on PATH. Install bubblewrap first."),
    }
}

/// Prompt the user for what to do with the overlay.
fn prompt_action(keep_flag: bool) -> Action {
    if keep_flag {
        return Action::Keep;
    }

    eprint!("[a]pply / [d]iscard / [k]eep (default)? ");
    io::stderr().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return Action::Keep;
    }

    match input.trim().to_lowercase().as_str() {
        "a" | "apply" => Action::Apply,
        "d" | "discard" => Action::Discard,
        _ => Action::Keep,
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Apply,
    Discard,
    Keep,
}

/// Expand --claude into concrete bind paths.
/// Creates ~/.claude if missing (credential storage).
/// Warns if ~/.claude.json is absent (onboarding gate).
fn claude_binds(home: &str) -> Vec<PathBuf> {
    let mut binds = Vec::new();
    let config_claude = PathBuf::from(format!("{home}/.config/claude"));
    if config_claude.exists() {
        binds.push(config_claude);
    }

    // Always provide .claude dir — create if absent so bind mount has a target
    let dot_claude = PathBuf::from(format!("{home}/.claude"));
    if !dot_claude.exists() {
        std::fs::create_dir_all(&dot_claude).ok();
    }
    binds.push(dot_claude);

    // .claude.json holds the onboarding gate; without it Claude Code
    // forces interactive auth regardless of env vars or credentials.
    let claude_json = PathBuf::from(format!("{home}/.claude.json"));
    if !claude_json.exists() {
        eprintln!(
            "slopwrap: ~/.claude.json not found — run `claude` outside the sandbox first to complete onboarding."
        );
    }
    if claude_json.exists() {
        binds.push(claude_json);
    }

    binds
}

/// Claude-specific env vars to pass through when --claude is set.
const CLAUDE_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CONFIG_DIR",
];

fn run() -> Result<i32> {
    let cli = Cli::parse();

    // Preflight
    let repo_root = find_repo_root()?;
    check_bwrap()?;

    // Set up overlay dirs
    let overlay_base: PathBuf;
    let _tmpdir; // keep tempdir alive for the duration

    if let Some(ref dir) = cli.overlay_dir {
        overlay_base = dir.clone();
        _tmpdir = None;
    } else {
        let td = tempfile::tempdir().context("creating temp overlay dir")?;
        overlay_base = td.path().to_path_buf();
        _tmpdir = Some(td);
    }

    let dirs = match (&cli.output_dir, &cli.work_dir) {
        (Some(out), Some(work)) => overlay::setup_explicit(out, work)?,
        (Some(_), None) | (None, Some(_)) => {
            bail!("--output-dir and --work-dir must be used together");
        }
        (None, None) => overlay::setup(&overlay_base)?,
    };

    // Expand --claude into concrete binds and env passthrough
    let ro_binds = cli.ro_binds;
    let mut rw_binds = cli.rw_binds;
    let mut env_passthrough = Vec::new();
    if cli.claude {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        rw_binds.extend(claude_binds(&home));
        for var in CLAUDE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                env_passthrough.push((var.to_string(), val));
            }
        }
    }

    // Build and run bwrap
    let config = sandbox::SandboxConfig {
        repo_root: repo_root.clone(),
        upperdir: dirs.upperdir.clone(),
        workdir: dirs.workdir.clone(),
        no_net: cli.no_net,
        ro_binds,
        rw_binds,
        env_passthrough,
        command: cli.command,
    };

    let args = config.build_args();

    let mut child = Command::new("bwrap")
        .args(&args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .context("spawning bwrap")?;

    // Forward signals to child
    let child_id = child.id();
    ctrlc_forward(child_id);

    let status = child.wait().context("waiting for bwrap")?;
    let exit_code = status.code().unwrap_or(1);

    // Fix overlayfs d--------- permissions so cleanup can remove the tree
    overlay::fix_overlay_permissions(&overlay_base);

    // Post-session diff
    let summary = diff::summarize(&dirs.upperdir, &repo_root)?;

    if summary.is_empty() {
        eprintln!("No changes detected.");
        return Ok(exit_code);
    }

    eprintln!("\n--- Changes ---");
    diff::print_summary(&summary);
    eprintln!();
    diff::show_diff(&dirs.upperdir, &repo_root, &summary)?;

    match prompt_action(cli.keep) {
        Action::Apply => {
            overlay::apply(&dirs.upperdir, &repo_root)?;
            eprintln!("Changes applied.");
        }
        Action::Discard => {
            // If using a provided overlay-dir, just clean the contents
            overlay::discard(&overlay_base)?;
            eprintln!("Changes discarded.");
        }
        Action::Keep => {
            eprintln!("Overlay kept at: {}", overlay_base.display());
            // Prevent tempdir from being cleaned up
            if let Some(td) = _tmpdir {
                // Leak the tempdir so it persists
                let path = td.keep();
                eprintln!("(temp dir preserved: {})", path.display());
            }
        }
    }

    Ok(exit_code)
}

fn ctrlc_forward(_child_pid: u32) {
    // Ignore SIGINT in the parent — the child gets it directly from the terminal.
    // bwrap --die-with-parent ensures the child dies if we do.
    use nix::sys::signal::{SigHandler, Signal, signal};
    unsafe {
        signal(Signal::SIGINT, SigHandler::SigIgn).ok();
    }
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("slopwrap: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_binds_includes_dot_claude_when_exists() {
        let fake_home = tempfile::tempdir().unwrap();
        std::fs::create_dir(fake_home.path().join(".claude")).unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert!(binds.contains(&fake_home.path().join(".claude")));
    }

    #[test]
    fn claude_binds_creates_and_includes_dot_claude_when_missing() {
        let fake_home = tempfile::tempdir().unwrap();
        assert!(!fake_home.path().join(".claude").exists());
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert!(fake_home.path().join(".claude").exists());
        assert!(binds.contains(&fake_home.path().join(".claude")));
    }

    #[test]
    fn claude_binds_includes_config_claude_when_exists() {
        let fake_home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(fake_home.path().join(".config/claude")).unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert!(binds.contains(&fake_home.path().join(".config/claude")));
    }

    #[test]
    fn claude_binds_omits_config_claude_when_missing() {
        let fake_home = tempfile::tempdir().unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert!(!binds.contains(&fake_home.path().join(".config/claude")));
    }

    #[test]
    fn claude_binds_includes_claude_json_when_exists() {
        let fake_home = tempfile::tempdir().unwrap();
        std::fs::write(fake_home.path().join(".claude.json"), "{}").unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert!(binds.contains(&fake_home.path().join(".claude.json")));
    }

    #[test]
    fn claude_binds_omits_claude_json_when_missing() {
        let fake_home = tempfile::tempdir().unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert!(!binds.contains(&fake_home.path().join(".claude.json")));
    }

    #[test]
    fn claude_binds_includes_all_when_all_exist() {
        let fake_home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(fake_home.path().join(".config/claude")).unwrap();
        std::fs::create_dir(fake_home.path().join(".claude")).unwrap();
        std::fs::write(fake_home.path().join(".claude.json"), "{}").unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        assert_eq!(binds.len(), 3);
    }

    #[test]
    fn claude_binds_only_dot_claude_when_nothing_else_exists() {
        let fake_home = tempfile::tempdir().unwrap();
        let binds = claude_binds(fake_home.path().to_str().unwrap());
        // .claude is always created and included
        assert_eq!(binds.len(), 1);
        assert!(binds.contains(&fake_home.path().join(".claude")));
    }
}
