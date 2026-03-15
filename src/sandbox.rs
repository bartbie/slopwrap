use std::path::{Path, PathBuf};

/// Paths that must never be mounted into the sandbox.
const BLOCKED_PATHS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".config/gcloud",
];

pub struct SandboxConfig {
    pub repo_root: PathBuf,
    pub upperdir: PathBuf,
    pub workdir: PathBuf,
    pub no_net: bool,
    pub command: Vec<String>,
}

impl SandboxConfig {
    pub fn build_args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());

        // Namespace isolation
        args.extend(["--unshare-all".into()]);
        if !self.no_net {
            args.push("--share-net".into());
        }
        args.extend([
            "--die-with-parent".into(),
            "--hostname".into(),
            "slopwrap".into(),
        ]);

        // /dev, /proc, /tmp
        args.extend([
            "--dev".into(),
            "/dev".into(),
            "--proc".into(),
            "/proc".into(),
            "--tmpfs".into(),
            "/tmp".into(),
        ]);

        // System (read-only)
        args.extend(["--ro-bind".into(), "/usr".into(), "/usr".into()]);

        // NixOS: /nix store and /run/current-system (provides /run/current-system/sw/bin)
        if Path::new("/nix").exists() {
            args.extend(["--ro-bind".into(), "/nix".into(), "/nix".into()]);
        }
        if Path::new("/run/current-system").exists() {
            args.extend([
                "--ro-bind".into(),
                "/run/current-system".into(),
                "/run/current-system".into(),
            ]);
        }

        // FHS symlinks
        args.extend([
            "--symlink".into(),
            "usr/lib".into(),
            "/lib".into(),
            "--symlink".into(),
            "usr/bin".into(),
            "/bin".into(),
            "--symlink".into(),
            "usr/sbin".into(),
            "/sbin".into(),
        ]);
        if Path::new("/usr/lib64").exists() {
            args.extend(["--symlink".into(), "usr/lib64".into(), "/lib64".into()]);
        }

        // /etc — selective
        for etc_file in &[
            "resolv.conf",
            "ssl",
            "ld.so.cache",
            "ld.so.conf",
            "ld.so.conf.d",
            "hosts",
            "nsswitch.conf",
            "localtime",
        ] {
            let p = format!("/etc/{etc_file}");
            if Path::new(&p).exists() {
                args.extend(["--ro-bind".into(), p.clone(), p]);
            }
        }
        // Optional /etc paths
        for etc_file in &["alternatives", "profile.d", "profiles"] {
            let p = format!("/etc/{etc_file}");
            if Path::new(&p).exists() {
                args.extend(["--ro-bind".into(), p.clone(), p]);
            }
        }

        // /usr/share read-only paths
        for share in &["ca-certificates", "terminfo", "zoneinfo"] {
            let p = format!("/usr/share/{share}");
            if Path::new(&p).exists() {
                args.extend(["--ro-bind".into(), p.clone(), p]);
            }
        }

        // Home — tmpfs base with selective allowlist
        args.extend(["--tmpfs".into(), home.clone()]);

        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(&home).join(p))
            .collect();

        // ro-bind-try for config/local
        for dot in &[".config", ".local"] {
            let src = format!("{home}/{dot}");
            if !is_blocked(&PathBuf::from(&src), &blocked) {
                args.extend(["--ro-bind-try".into(), src.clone(), src]);
            }
        }

        // bind-try (writable) for cache
        {
            let src = format!("{home}/.cache");
            if !is_blocked(&PathBuf::from(&src), &blocked) {
                args.extend(["--bind-try".into(), src.clone(), src]);
            }
        }

        // ro-bind-try for shell/git config
        for dot in &[".bashrc", ".profile", ".gitconfig"] {
            let src = format!("{home}/{dot}");
            if !is_blocked(&PathBuf::from(&src), &blocked) {
                args.extend(["--ro-bind-try".into(), src.clone(), src]);
            }
        }

        // Repo overlay
        let repo = self.repo_root.to_string_lossy().to_string();
        let upper = self.upperdir.to_string_lossy().to_string();
        let work = self.workdir.to_string_lossy().to_string();
        args.extend([
            "--overlay-src".into(),
            repo.clone(),
            "--overlay".into(),
            upper,
            work,
            repo.clone(),
        ]);

        // Working directory
        args.extend(["--chdir".into(), repo]);

        // Separator and command
        args.push("--".into());
        args.extend(self.command.iter().cloned());

        args
    }
}

fn is_blocked(path: &Path, blocked: &[PathBuf]) -> bool {
    blocked
        .iter()
        .any(|b| path == b || path.starts_with(b) || b.starts_with(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(no_net: bool) -> SandboxConfig {
        SandboxConfig {
            repo_root: PathBuf::from("/tmp/repo"),
            upperdir: PathBuf::from("/tmp/overlay/upper"),
            workdir: PathBuf::from("/tmp/overlay/work"),
            no_net,
            command: vec!["bash".into()],
        }
    }

    #[test]
    fn default_args_contain_unshare_all() {
        let args = make_config(false).build_args();
        assert!(args.contains(&"--unshare-all".to_string()));
    }

    #[test]
    fn default_args_contain_share_net() {
        let args = make_config(false).build_args();
        assert!(args.contains(&"--share-net".to_string()));
    }

    #[test]
    fn no_net_omits_share_net() {
        let args = make_config(true).build_args();
        assert!(!args.contains(&"--share-net".to_string()));
        assert!(args.contains(&"--unshare-all".to_string()));
    }

    #[test]
    fn blocked_paths_never_in_args() {
        let args = make_config(false).build_args();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        for blocked in BLOCKED_PATHS {
            let full = format!("{home}/{blocked}");
            assert!(
                !args.contains(&full),
                "blocked path {full} found in args"
            );
        }
    }

    #[test]
    fn overlay_flags_reference_correct_paths() {
        let cfg = make_config(false);
        let args = cfg.build_args();
        let joined = args.join(" ");
        assert!(joined.contains("--overlay-src /tmp/repo"));
        assert!(joined.contains("--overlay /tmp/overlay/upper /tmp/overlay/work /tmp/repo"));
    }

    #[test]
    fn command_appears_after_separator() {
        let args = make_config(false).build_args();
        let sep_pos = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[sep_pos + 1], "bash");
    }

    #[test]
    fn hostname_set_to_slopwrap() {
        let args = make_config(false).build_args();
        let idx = args.iter().position(|a| a == "--hostname").unwrap();
        assert_eq!(args[idx + 1], "slopwrap");
    }

    #[test]
    fn die_with_parent_present() {
        let args = make_config(false).build_args();
        assert!(args.contains(&"--die-with-parent".to_string()));
    }

    // --- Property-based tests ---

    fn arb_path(u: &mut arbtest::arbitrary::Unstructured) -> arbtest::arbitrary::Result<PathBuf> {
        let len: usize = u.int_in_range(1..=8)?;
        let mut s = String::from("/tmp/");
        for _ in 0..len {
            let c = u.int_in_range(b'a'..=b'z')?;
            s.push(c as char);
        }
        Ok(PathBuf::from(s))
    }

    fn arb_config(u: &mut arbtest::arbitrary::Unstructured) -> arbtest::arbitrary::Result<SandboxConfig> {
        Ok(SandboxConfig {
            repo_root: arb_path(u)?,
            upperdir: arb_path(u)?,
            workdir: arb_path(u)?,
            no_net: u.arbitrary()?,
            command: vec!["test-cmd".into()],
        })
    }

    #[test]
    fn prop_blocked_paths_never_in_args() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        arbtest::arbtest(|u| {
            let cfg = arb_config(u)?;
            let args = cfg.build_args();
            for blocked in BLOCKED_PATHS {
                let full = format!("{home}/{blocked}");
                assert!(
                    !args.contains(&full),
                    "blocked path {full} found in args"
                );
            }
            Ok(())
        });
    }

    #[test]
    fn prop_unshare_all_always_present() {
        arbtest::arbtest(|u| {
            let cfg = arb_config(u)?;
            let args = cfg.build_args();
            assert!(args.contains(&"--unshare-all".to_string()));
            Ok(())
        });
    }

    #[test]
    fn prop_share_net_iff_not_no_net() {
        arbtest::arbtest(|u| {
            let cfg = arb_config(u)?;
            let no_net = cfg.no_net;
            let args = cfg.build_args();
            if no_net {
                assert!(!args.contains(&"--share-net".to_string()));
            } else {
                assert!(args.contains(&"--share-net".to_string()));
            }
            Ok(())
        });
    }
}
