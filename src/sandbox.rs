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
    pub ro_binds: Vec<PathBuf>,
    pub rw_binds: Vec<PathBuf>,
    pub env_passthrough: Vec<(String, String)>,
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

        // Environment isolation: clear inherited env, set safe subset
        args.push("--clearenv".into());
        for (key, val) in [
            ("HOME", home.clone()),
            (
                "USER",
                std::env::var("USER").unwrap_or_else(|_| "nobody".into()),
            ),
            (
                "PATH",
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
            ),
            (
                "TERM",
                std::env::var("TERM").unwrap_or_else(|_| "xterm".into()),
            ),
            (
                "LANG",
                std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()),
            ),
            (
                "SHELL",
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            ),
        ] {
            args.extend(["--setenv".into(), key.into(), val]);
        }
        for (key, val) in &self.env_passthrough {
            args.extend(["--setenv".into(), key.clone(), val.clone()]);
        }

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
            "passwd",
            "group",
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

        // ro-bind-try for config/local, with tmpfs overlays hiding blocked children
        for dot in &[".config", ".local"] {
            let src = PathBuf::from(format!("{home}/{dot}"));
            if is_directly_blocked(&src, &blocked) {
                continue;
            }
            let src_str = src.to_string_lossy().to_string();
            args.extend(["--ro-bind-try".into(), src_str.clone(), src_str]);
            for child in blocked_children(&src, &blocked) {
                if child.exists() {
                    args.extend(["--tmpfs".into(), child.to_string_lossy().to_string()]);
                }
            }
        }

        // bind-try (writable) for cache
        {
            let src = PathBuf::from(format!("{home}/.cache"));
            if !is_directly_blocked(&src, &blocked) {
                let src_str = src.to_string_lossy().to_string();
                args.extend(["--bind-try".into(), src_str.clone(), src_str]);
                for child in blocked_children(&src, &blocked) {
                    if child.exists() {
                        args.extend(["--tmpfs".into(), child.to_string_lossy().to_string()]);
                    }
                }
            }
        }

        // ro-bind-try for shell/git config
        for dot in &[".bashrc", ".profile", ".gitconfig"] {
            let src = format!("{home}/{dot}");
            if !is_directly_blocked(&PathBuf::from(&src), &blocked) {
                args.extend(["--ro-bind-try".into(), src.clone(), src]);
            }
        }

        // Extra user-specified binds
        for p in &self.ro_binds {
            let s = p.to_string_lossy().to_string();
            args.extend(["--ro-bind".into(), s.clone(), s]);
        }
        for p in &self.rw_binds {
            let s = p.to_string_lossy().to_string();
            args.extend(["--bind".into(), s.clone(), s]);
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

/// True when `path` itself is blocked or is a descendant of a blocked dir.
fn is_directly_blocked(path: &Path, blocked: &[PathBuf]) -> bool {
    blocked.iter().any(|b| path == b || path.starts_with(b))
}

/// Returns blocked paths that are strict children of `path`.
fn blocked_children<'a>(path: &Path, blocked: &'a [PathBuf]) -> Vec<&'a PathBuf> {
    blocked
        .iter()
        .filter(|b| b.starts_with(path) && *b != path)
        .collect()
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
            ro_binds: vec![],
            rw_binds: vec![],
            env_passthrough: vec![],
            command: vec!["bash".into()],
        }
    }

    /// True when `path` follows a bind-mount flag in `args`.
    fn is_bind_mounted_in(path: &str, args: &[String]) -> bool {
        let bind_flags = ["--ro-bind", "--bind", "--ro-bind-try", "--bind-try"];
        args.windows(2)
            .any(|w| bind_flags.contains(&w[0].as_str()) && w[1] == path)
    }

    // --- existing tests (updated) ---

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
    fn blocked_paths_never_bind_mounted() {
        let args = make_config(false).build_args();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        for blocked in BLOCKED_PATHS {
            let full = format!("{home}/{blocked}");
            assert!(
                !is_bind_mounted_in(&full, &args),
                "blocked path {full} is bind-mounted"
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

    #[test]
    fn ro_bind_appears_in_args() {
        let mut cfg = make_config(false);
        cfg.ro_binds = vec![PathBuf::from("/tmp/myconfig")];
        let args = cfg.build_args();
        let idx = args.iter().position(|a| a == "/tmp/myconfig").unwrap();
        assert_eq!(args[idx - 1], "--ro-bind");
    }

    #[test]
    fn rw_bind_appears_in_args() {
        let mut cfg = make_config(false);
        cfg.rw_binds = vec![PathBuf::from("/tmp/mystate")];
        let args = cfg.build_args();
        let idx = args.iter().position(|a| a == "/tmp/mystate").unwrap();
        assert_eq!(args[idx - 1], "--bind");
    }

    #[test]
    fn multiple_binds_all_present() {
        let mut cfg = make_config(false);
        cfg.ro_binds = vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")];
        cfg.rw_binds = vec![PathBuf::from("/tmp/c")];
        let args = cfg.build_args();
        assert!(args.contains(&"/tmp/a".to_string()));
        assert!(args.contains(&"/tmp/b".to_string()));
        assert!(args.contains(&"/tmp/c".to_string()));
    }

    // --- is_directly_blocked / blocked_children ---

    #[test]
    fn blocked_child_does_not_block_parent() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        let config = PathBuf::from(format!("{home}/.config"));
        assert!(!is_directly_blocked(&config, &blocked));
    }

    #[test]
    fn blocked_child_is_still_blocked() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        let gcloud = PathBuf::from(format!("{home}/.config/gcloud"));
        assert!(is_directly_blocked(&gcloud, &blocked));
    }

    #[test]
    fn blocked_exact_match() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        assert!(is_directly_blocked(
            &PathBuf::from(format!("{home}/.ssh")),
            &blocked,
        ));
    }

    #[test]
    fn blocked_descendant() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        assert!(is_directly_blocked(
            &PathBuf::from(format!("{home}/.ssh/keys/id_rsa")),
            &blocked,
        ));
    }

    #[test]
    fn unrelated_path_not_blocked() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        assert!(!is_directly_blocked(
            &PathBuf::from(format!("{home}/.local")),
            &blocked,
        ));
    }

    #[test]
    fn config_has_gcloud_as_blocked_child() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        let config = PathBuf::from(format!("{home}/.config"));
        let children = blocked_children(&config, &blocked);
        assert_eq!(children.len(), 1);
        assert_eq!(
            children[0],
            &PathBuf::from(format!("{home}/.config/gcloud"))
        );
    }

    #[test]
    fn ssh_has_no_blocked_children() {
        let home = "/home/test";
        let blocked: Vec<PathBuf> = BLOCKED_PATHS
            .iter()
            .map(|p| PathBuf::from(home).join(p))
            .collect();
        // .ssh is directly blocked — not a parent with blocked children
        assert!(blocked_children(&PathBuf::from(format!("{home}/.ssh")), &blocked).is_empty());
    }

    // --- --clearenv / env isolation ---

    #[test]
    fn clearenv_present_in_args() {
        let args = make_config(false).build_args();
        assert!(args.contains(&"--clearenv".to_string()));
    }

    #[test]
    fn home_setenv_in_args() {
        let args = make_config(false).build_args();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        assert!(
            args.windows(3)
                .any(|w| w[0] == "--setenv" && w[1] == "HOME" && w[2] == home),
            "expected --setenv HOME {home}"
        );
    }

    #[test]
    fn path_setenv_in_args() {
        let args = make_config(false).build_args();
        assert!(
            args.windows(2).any(|w| w[0] == "--setenv" && w[1] == "PATH"),
            "expected --setenv PATH"
        );
    }

    #[test]
    fn term_setenv_in_args() {
        let args = make_config(false).build_args();
        assert!(
            args.windows(2).any(|w| w[0] == "--setenv" && w[1] == "TERM"),
            "expected --setenv TERM"
        );
    }

    #[test]
    fn dbus_not_in_setenv() {
        let args = make_config(false).build_args();
        assert!(
            !args
                .windows(2)
                .any(|w| w[0] == "--setenv" && w[1] == "DBUS_SESSION_BUS_ADDRESS"),
            "DBUS_SESSION_BUS_ADDRESS must not leak into sandbox"
        );
    }

    #[test]
    fn display_not_in_setenv() {
        let args = make_config(false).build_args();
        assert!(
            !args
                .windows(2)
                .any(|w| w[0] == "--setenv" && (w[1] == "DISPLAY" || w[1] == "WAYLAND_DISPLAY")),
            "DISPLAY/WAYLAND_DISPLAY must not leak into sandbox"
        );
    }

    #[test]
    fn env_passthrough_appears_in_args() {
        let mut cfg = make_config(false);
        cfg.env_passthrough = vec![("MY_VAR".into(), "my_val".into())];
        let args = cfg.build_args();
        assert!(args.windows(3).any(|w| w[0] == "--setenv"
            && w[1] == "MY_VAR"
            && w[2] == "my_val"));
    }

    // --- /etc/passwd and /etc/group ---

    #[test]
    fn passwd_in_etc_mounts() {
        if !Path::new("/etc/passwd").exists() {
            return;
        }
        let args = make_config(false).build_args();
        assert!(args.contains(&"/etc/passwd".to_string()));
    }

    #[test]
    fn group_in_etc_mounts() {
        if !Path::new("/etc/group").exists() {
            return;
        }
        let args = make_config(false).build_args();
        assert!(args.contains(&"/etc/group".to_string()));
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

    fn arb_paths(
        u: &mut arbtest::arbitrary::Unstructured,
    ) -> arbtest::arbitrary::Result<Vec<PathBuf>> {
        let len: usize = u.int_in_range(0..=3)?;
        (0..len).map(|_| arb_path(u)).collect()
    }

    fn arb_config(
        u: &mut arbtest::arbitrary::Unstructured,
    ) -> arbtest::arbitrary::Result<SandboxConfig> {
        Ok(SandboxConfig {
            repo_root: arb_path(u)?,
            upperdir: arb_path(u)?,
            workdir: arb_path(u)?,
            no_net: u.arbitrary()?,
            ro_binds: arb_paths(u)?,
            rw_binds: arb_paths(u)?,
            env_passthrough: vec![],
            command: vec!["test-cmd".into()],
        })
    }

    #[test]
    fn prop_blocked_paths_never_bind_mounted() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        arbtest::arbtest(|u| {
            let cfg = arb_config(u)?;
            let args = cfg.build_args();
            for blocked in BLOCKED_PATHS {
                let full = format!("{home}/{blocked}");
                assert!(
                    !is_bind_mounted_in(&full, &args),
                    "blocked path {full} is bind-mounted"
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

    #[test]
    fn prop_clearenv_always_present() {
        arbtest::arbtest(|u| {
            let cfg = arb_config(u)?;
            let args = cfg.build_args();
            assert!(args.contains(&"--clearenv".to_string()));
            Ok(())
        });
    }

    #[test]
    fn prop_no_env_leak() {
        let forbidden = [
            "DBUS_SESSION_BUS_ADDRESS",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
        ];
        arbtest::arbtest(|u| {
            let cfg = arb_config(u)?;
            let args = cfg.build_args();
            for var in &forbidden {
                assert!(
                    !args
                        .windows(2)
                        .any(|w| w[0] == "--setenv" && w[1] == *var),
                    "forbidden env var {var} leaked into --setenv"
                );
            }
            Ok(())
        });
    }
}
