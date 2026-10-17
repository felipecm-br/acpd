use crate::tmux_cmd::{tmux_out, tmux_status};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn slugify_prompt(prompt: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let conventional_prefixes = [
        "feat", "fix", "refactor", "docs", "chore", "test", "perf", "style", "ci", "build",
    ];

    let mut prefix = String::new();
    let mut body = String::new();

    for &cp in &conventional_prefixes {
        if let Some(after) = trimmed.strip_prefix(cp) {
            let rest = if let Some(after_scope) = after.strip_prefix('(') {
                if let Some(close_idx) = after_scope.find(')') {
                    &after_scope[close_idx + 1..]
                } else {
                    after
                }
            } else {
                after
            };
            if let Some(b) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('/')) {
                prefix = cp.to_string();
                body = b.trim().to_string();
                break;
            }
        }
    }

    if prefix.is_empty() {
        let mut words = trimmed.split_whitespace();
        let first_word = words.next().unwrap_or("").to_ascii_lowercase();
        let remainder: String = words.collect::<Vec<_>>().join(" ");
        match first_word.as_str() {
            "fix" | "bug" | "repair" | "resolve" | "patch" | "hotfix" => {
                prefix = "fix".to_string();
                body = remainder;
            }
            "refactor" | "clean" | "cleanup" | "restructure" | "reorganize" => {
                prefix = "refactor".to_string();
                body = remainder;
            }
            "doc" | "docs" | "document" | "documentation" => {
                prefix = "docs".to_string();
                body = remainder;
            }
            "test" | "tests" | "testing" => {
                prefix = "test".to_string();
                body = remainder;
            }
            "chore" | "bump" | "upgrade" | "deps" | "release" => {
                prefix = "chore".to_string();
                body = remainder;
            }
            "perf" | "optimize" | "performance" | "speedup" => {
                prefix = "perf".to_string();
                body = remainder;
            }
            "add" | "create" | "impl" | "implement" | "feat" | "feature" | "build" | "new" => {
                prefix = "feat".to_string();
                body = remainder;
            }
            _ => {
                prefix = "feat".to_string();
                body = trimmed.to_string();
            }
        }
    }

    if body.is_empty() {
        body = trimmed.to_string();
    }

    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in body.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.len() > 45 {
        slug.truncate(45);
        if let Some(last_dash) = slug.rfind('-')
            && last_dash > 0
        {
            slug.truncate(last_dash);
        }
    }

    if slug.is_empty() {
        slug = "worktree".to_string();
    }

    format!("{}/{}", prefix, slug)
}

fn find_waymaker_script(script_name: &str) -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        let p1 = PathBuf::from(&home)
            .join(".config/waymaker/scripts")
            .join(script_name);
        if p1.is_file() {
            return Some(p1);
        }
        let p2 = PathBuf::from(&home)
            .join(".config/matchmaker/scripts")
            .join(script_name);
        if p2.is_file() {
            return Some(p2);
        }
    }
    None
}

fn find_wm_bin() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let p = format!("{}/.local/bin/wm", home);
        if Path::new(&p).is_file() {
            return p;
        }
    }
    "wm".to_string()
}

fn spawn_session_and_send(sess: &str, target_dir: &str, cmd: &str) {
    let target_pane = format!("{}:0.0", sess);
    if tmux_status(&["has-session", "-t", sess]) {
        if !cmd.is_empty() && cmd != "__FRESH__" {
            let spaced = format!(" {}", cmd);
            let _ = tmux_status(&["send-keys", "-t", &target_pane, &spaced, "C-m"]);
        }
    } else if !cmd.is_empty() && cmd != "__FRESH__" {
        let pid = std::process::id();
        let chan = format!("awt_ready_{}", pid);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let init_cmd = format!("sh -c 'tmux wait-for -S {}; exec {} -l'", chan, shell);
        let _ = tmux_status(&["new-session", "-d", "-s", sess, "-c", target_dir, &init_cmd]);
        let _ = tmux_status(&["wait-for", &chan]);
        let spaced = format!(" {}", cmd);
        let _ = tmux_status(&["send-keys", "-t", &target_pane, &spaced, "C-m"]);
    } else {
        let _ = tmux_status(&["new-session", "-d", "-s", sess, "-c", target_dir]);
    }
}

fn is_git_repo() -> bool {
    let wt = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if wt {
        return true;
    }
    Command::new("git")
        .args(["rev-parse", "--is-bare-repository"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn get_repo_root() -> String {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| ".".to_string())
}

fn compute_session_name(repo_root: &str, folder: &str) -> String {
    let parent = Path::new(repo_root)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    if parent == ".dotfiles" {
        format!("_dotfiles/{}", folder)
    } else {
        format!("{}/{}", parent, folder)
    }
}

fn show_help() {
    println!(
        "waymux wt (awt) - Agent Worktree Manager & Multi-Agent Git Orchestrator\n\n\
USAGE:\n    \
awt                                     Open interactive Waymaker TUI dashboard\n    \
awt -c | new | add [flags]              Launch interactive 5-step Conventional Commits wizard (awc)\n    \
awt -c -A <prompt> [base] [flags]       Auto-derive conventional branch name from prompt & provision\n    \
awt -c <branch> [base] [flags] [-- cmd] Create/provision worktree, run inline command & connect\n    \
awt <branch> | switch <branch>          Switch to existing worktree session (or auto-create from HEAD)\n    \
awt popup                               Open floating AWT modal in Tmux (85% × 75%) (awp)\n    \
awt pr [number]                         Interactive PR checkout or direct PR worktree provision\n    \
awt clone <repo> [dir]                  Clone repository into .bare container layout (awtc)\n    \
awt rm | remove <branch> [flags]        Delete worktree, Git branch, and kill Tmux session\n    \
awt rebase [base]                       Rebase active worktree onto base branch with auto-stash\n    \
awt ship [branch] [flags]               Merge into base, push to origin, and clean up worktree\n    \
awt sweep [flags] [base]                Prune merged worktrees and dead Tmux sessions in batch\n    \
awt reap [flags]                        Garbage-collect stale AI agents and orphan worktree sessions\n    \
awt help | -h | --help                  Show this help message"
    );
}

pub fn run_cli(args: &[String]) -> i32 {
    let first = args.first().map(|s| s.as_str()).unwrap_or("");
    match first {
        "-h" | "--help" | "help" => {
            show_help();
            return 0;
        }
        "popup" => {
            if let Ok(home) = std::env::var("HOME") {
                let popup = format!("{}/.config/tmux/awt-popup.sh", home);
                if Path::new(&popup).is_file() {
                    let cwd = std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| ".".to_string());
                    return Command::new(&popup)
                        .arg(&cwd)
                        .status()
                        .ok()
                        .and_then(|s| s.code())
                        .unwrap_or(0);
                }
            }
            return 0;
        }
        "clone" => {
            return Command::new("awtc")
                .args(&args[1..])
                .status()
                .ok()
                .and_then(|s| s.code())
                .unwrap_or(1);
        }
        "rm" | "remove" | "del" | "delete" => {
            let mut flags = Vec::new();
            let mut branch = String::new();
            for arg in &args[1..] {
                if matches!(
                    arg.as_str(),
                    "-f" | "--force"
                        | "--no-delete-branch"
                        | "--no-tmux"
                        | "--no-hooks"
                        | "--skip-pre-remove"
                        | "-H"
                ) {
                    flags.push(arg.clone());
                } else if branch.is_empty() {
                    branch = arg.clone();
                } else {
                    flags.push(arg.clone());
                }
            }
            if branch.is_empty() {
                eprintln!(
                    "Usage: awt rm <branch> [-f|--force] [--no-delete-branch] [--no-tmux] [--no-hooks]"
                );
                return 1;
            }
            let repo_root = get_repo_root();
            let branch_folder = branch.replace('/', "-");
            let target_dir = format!("{}/../{}", repo_root, branch_folder);
            let session_name = compute_session_name(&repo_root, &branch_folder);
            if let Some(del_script) = find_waymaker_script("awt-delete.sh") {
                let mut cmd = Command::new(del_script);
                cmd.args([&session_name, &target_dir, &branch]);
                cmd.args(&flags);
                return cmd.status().ok().and_then(|s| s.code()).unwrap_or(0);
            }
            return 0;
        }
        "rebase" => {
            let base = args.get(1).map(|s| s.as_str()).unwrap_or("");
            let repo_root = get_repo_root();
            let cur_branch = Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .stderr(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            if let Some(reb_script) = find_waymaker_script("awt-rebase.sh") {
                let mut cmd = Command::new(reb_script);
                cmd.args([&cur_branch, &repo_root, base]);
                cmd.args(&args[1..]);
                return cmd.status().ok().and_then(|s| s.code()).unwrap_or(0);
            }
            return 0;
        }
        "merge" | "ship" => {
            let is_ship = first == "ship";
            let mut flags = Vec::new();
            let mut target = String::new();
            let mut idx = 1usize;
            while idx < args.len() {
                let a = &args[idx];
                if matches!(
                    a.as_str(),
                    "--squash"
                        | "--no-squash"
                        | "--no-commit"
                        | "--no-remove"
                        | "--no-tmux"
                        | "--no-connect"
                        | "--no-rebase"
                        | "--rebase"
                        | "--push"
                        | "--ship"
                        | "--no-hooks"
                        | "--skip-pre-merge"
                        | "-H"
                ) || a.starts_with("--into=")
                {
                    flags.push(a.clone());
                } else if a == "--into" {
                    idx += 1;
                    if let Some(val) = args.get(idx) {
                        flags.push("--into".to_string());
                        flags.push(val.clone());
                    }
                } else if target.is_empty() {
                    target = a.clone();
                } else {
                    flags.push(a.clone());
                }
                idx += 1;
            }
            if is_ship {
                flags.push("--push".to_string());
            }
            let repo_root = get_repo_root();
            let cur_folder = Path::new(&repo_root)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let session_name = compute_session_name(&repo_root, &cur_folder);
            if let Some(m_script) = find_waymaker_script("awt-merge.sh") {
                let mut cmd = Command::new(m_script);
                cmd.args([&target, &repo_root, &session_name, ""]);
                cmd.args(&flags);
                return cmd.status().ok().and_then(|s| s.code()).unwrap_or(0);
            }
            return 0;
        }
        "pr" => {
            if let Some(script) = find_waymaker_script("awt-pr.sh") {
                return Command::new(script)
                    .args(&args[1..])
                    .status()
                    .ok()
                    .and_then(|s| s.code())
                    .unwrap_or(0);
            }
            eprintln!("awt: awt-pr.sh not found.");
            return 1;
        }
        "sweep" => {
            if let Some(script) = find_waymaker_script("awt-sweep.sh") {
                return Command::new(script)
                    .args(&args[1..])
                    .status()
                    .ok()
                    .and_then(|s| s.code())
                    .unwrap_or(0);
            }
            eprintln!("awt: awt-sweep.sh not found.");
            return 1;
        }
        "reap" => {
            if let Some(script) = find_waymaker_script("awt-reap.sh") {
                return Command::new(script)
                    .args(&args[1..])
                    .status()
                    .ok()
                    .and_then(|s| s.code())
                    .unwrap_or(0);
            }
            eprintln!("awt: awt-reap.sh not found.");
            return 1;
        }
        _ => {}
    }

    if !is_git_repo() {
        println!("awt: Not inside a Git repository.");
        return 1;
    }

    let wm_bin = find_wm_bin();

    // Interactive TUI Mode (0 arguments)
    if args.is_empty() {
        let out = Command::new(&wm_bin)
            .args(["-o", "awt"])
            .stdout(Stdio::piped())
            .spawn()
            .and_then(|c| c.wait_with_output());
        if let Ok(o) = out {
            let chosen = String::from_utf8_lossy(&o.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if chosen.is_empty() {
                return 0;
            }
            if let Some((session, target)) = chosen.split_once('\t') {
                let cur_session =
                    tmux_out(&["display-message", "-p", "#{session_name}"]).unwrap_or_default();
                if !cur_session.is_empty() && cur_session == session {
                    return 0;
                }
                let connect_arg = if tmux_status(&["has-session", "-t", session]) {
                    session
                } else {
                    target
                };
                let _ = Command::new(&wm_bin)
                    .args(["connect", connect_arg])
                    .status();
            }
        }
        return 0;
    }

    // Parse creation/switch arguments
    let mut create = false;
    let mut with_changes = false;
    let mut no_connect = false;
    let mut custom_ai_cmd = String::new();
    let mut branch = String::new();
    let mut base = "HEAD".to_string();
    let mut extra_flags: Vec<String> = Vec::new();

    let mut idx = 0usize;
    while idx < args.len() {
        let a = &args[idx];
        match a.as_str() {
            "new" | "create" | "add" | "-c" | "--create" => {
                create = true;
            }
            "switch" => {}
            "-W" | "--with-changes" => {
                with_changes = true;
                create = true;
            }
            "-A" | "--auto-name" => {
                idx += 1;
                if let Some(prompt) = args.get(idx) {
                    branch = slugify_prompt(prompt);
                    create = true;
                }
            }
            "--no-tmux" | "--no-connect" => {
                no_connect = true;
                extra_flags.push(a.clone());
            }
            "--continue" | "--ai-continue" | "--no-continue" | "--fresh" => {
                extra_flags.push(a.clone());
            }
            "--" => {
                custom_ai_cmd = args[idx + 1..].join(" ");
                extra_flags.push(format!("--ai={}", custom_ai_cmd));
                break;
            }
            _ => {
                if let Some(prompt) = a
                    .strip_prefix("--auto-name=")
                    .or_else(|| a.strip_prefix("-A="))
                {
                    branch = slugify_prompt(prompt);
                    create = true;
                } else if let Some(cmd) = a.strip_prefix("--ai=") {
                    custom_ai_cmd = cmd.to_string();
                    extra_flags.push(a.clone());
                } else if branch.is_empty() {
                    branch = a.clone();
                } else if base == "HEAD" {
                    base = a.clone();
                }
            }
        }
        idx += 1;
    }

    if create && branch.is_empty() {
        let cur_branch = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .stderr(Stdio::null())
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if let Some(new_script) = find_waymaker_script("awt-new.sh") {
            let mut cmd = Command::new(new_script);
            cmd.arg(&cur_branch);
            cmd.args(&extra_flags);
            return cmd.status().ok().and_then(|s| s.code()).unwrap_or(0);
        }
        return 0;
    }

    if branch.is_empty() {
        println!(
            "awt: branch name required. Usage: awt [-c <branch> [base] [--no-tmux] [--continue]]"
        );
        return 1;
    }

    let branch_clean = branch.replace('/', "-");
    let repo_root = get_repo_root();
    let raw_target = format!("{}/../{}", repo_root, branch_clean);
    let target_dir = fs::canonicalize(&raw_target)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(raw_target);
    let sess = compute_session_name(&repo_root, &branch_clean);

    if Path::new(&target_dir).is_dir() {
        if !custom_ai_cmd.is_empty() && custom_ai_cmd != "__FRESH__" {
            spawn_session_and_send(&sess, &target_dir, &custom_ai_cmd);
            if no_connect {
                println!("✓ Dispatched '{}' in session '{}'", custom_ai_cmd, sess);
                return 0;
            }
        }
        if !no_connect {
            let connect_arg = if tmux_status(&["has-session", "-t", &sess]) {
                &sess
            } else {
                &target_dir
            };
            let _ = Command::new(&wm_bin)
                .args(["connect", connect_arg])
                .status();
        }
        return 0;
    }

    // Create worktree via native Git
    let mut stashed = false;
    if with_changes {
        let dirty = Command::new("git")
            .args(["-C", &repo_root, "status", "--porcelain"])
            .stderr(Stdio::null())
            .output()
            .ok()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);
        if dirty {
            stashed = Command::new("git")
                .args([
                    "-C",
                    &repo_root,
                    "stash",
                    "push",
                    "--include-untracked",
                    "-m",
                    &format!("awt-with-changes: {}", branch),
                ])
                .stdout(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        }
    }

    let created = Command::new("git")
        .args(["worktree", "add", &target_dir, "-b", &branch, &base])
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        || Command::new("git")
            .args(["worktree", "add", &target_dir, &branch])
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if !created {
        println!("awt: Failed to create worktree at {}", target_dir);
        return 1;
    }

    let _ = Command::new("git")
        .args([
            "-C",
            &target_dir,
            "config",
            &format!("branch.{}.base", branch),
            &base,
        ])
        .stderr(Stdio::null())
        .status();

    if stashed {
        let popped = Command::new("git")
            .args(["-C", &target_dir, "stash", "pop"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !popped {
            eprintln!(
                "⚠ Stash pop had conflicts in {}; your changes remain safe in 'git stash list'.",
                target_dir
            );
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let post_hook = format!("{}/.config/waymaker/hooks/post-create.sh", home);
        if Path::new(&post_hook).is_file() {
            let _ = Command::new(&post_hook)
                .args([&target_dir, &branch, &base])
                .stderr(Stdio::null())
                .status();
        }
    }

    println!("✓ Worktree ready at {}", target_dir);

    if !custom_ai_cmd.is_empty() && custom_ai_cmd != "__FRESH__" {
        spawn_session_and_send(&sess, &target_dir, &custom_ai_cmd);
        if no_connect {
            println!(
                "✓ Dispatched '{}' in background session '{}'",
                custom_ai_cmd, sess
            );
            return 0;
        }
    }

    if !no_connect {
        let connect_arg = if tmux_status(&["has-session", "-t", &sess]) {
            &sess
        } else {
            &target_dir
        };
        let _ = Command::new(&wm_bin)
            .args(["connect", connect_arg])
            .status();
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_prompt() {
        assert_eq!(
            slugify_prompt("fix: race condition in token refresh"),
            "fix/race-condition-in-token-refresh"
        );
        assert_eq!(
            slugify_prompt("refactor(auth): simplify middleware"),
            "refactor/simplify-middleware"
        );
        assert_eq!(
            slugify_prompt("optimize database connection pool"),
            "perf/database-connection-pool"
        );
        assert_eq!(
            slugify_prompt("add oauth2 login flow"),
            "feat/oauth2-login-flow"
        );
    }

    #[test]
    fn test_slugify_truncation() {
        let long =
            "implement a very long feature description that exceeds forty five characters easily";
        let slug = slugify_prompt(long);
        assert!(slug.starts_with("feat/"));
        let body = slug.strip_prefix("feat/").unwrap();
        assert!(body.len() <= 45);
        assert!(!body.ends_with('-'));
    }
}
