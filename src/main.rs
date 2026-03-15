mod diff;
mod overlay;
mod sandbox;

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(name = "slopwrap", about = "Sandbox AI tools with bubblewrap")]
struct Cli {
    /// Disable network access inside the sandbox.
    #[arg(long)]
    no_net: bool,

    /// Use a specific directory for the overlay instead of a tempdir.
    #[arg(long)]
    overlay_dir: Option<PathBuf>,

    /// Keep the overlay directory after exit (default on Ctrl-C).
    #[arg(long)]
    keep: bool,

    /// Command and arguments to run inside the sandbox.
    #[arg(last = true, required = true)]
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

    let dirs = overlay::setup(&overlay_base)?;

    // Build and run bwrap
    let config = sandbox::SandboxConfig {
        repo_root: repo_root.clone(),
        upperdir: dirs.upperdir.clone(),
        workdir: dirs.workdir.clone(),
        no_net: cli.no_net,
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
