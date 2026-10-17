use crate::rpc_client;
use crate::theme::{BOLD, RESET, ThemePalette};
use crate::tmux_cmd::tmux_out;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const AGENT_NAMES: &[&str] = &[
    "agy",
    "agy-bin",
    "antigravity",
    "opencode",
    "claude",
    "codex",
    "muse",
];

pub fn is_agent_binary_name(name: &str) -> bool {
    AGENT_NAMES.contains(&name)
}

pub fn contains_agent_name(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    ["agy", "antigravity", "opencode", "claude", "codex", "muse"]
        .iter()
        .any(|a| lower.contains(a))
}

pub fn scan_running_agents() -> (HashMap<String, String>, HashMap<String, u64>) {
    let mut sess_agent = HashMap::new();
    let mut sess_etimes = HashMap::new();

    let uptime_secs: u64 = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0);

    let entries = match fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return (sess_agent, sess_etimes),
    };

    for entry in entries.flatten() {
        let fname = entry.file_name();
        let pid_str = match fname.to_str() {
            Some(s) if s.chars().all(|c| c.is_ascii_digit()) => s,
            _ => continue,
        };

        let comm_path = format!("/proc/{}/comm", pid_str);
        let comm = match fs::read_to_string(&comm_path) {
            Ok(c) => c.trim().to_string(),
            Err(_) => continue,
        };

        if !is_agent_binary_name(&comm) {
            continue;
        }

        let stat_path = format!("/proc/{}/stat", pid_str);
        if let Ok(stat_str) = fs::read_to_string(&stat_path)
            && let Some(rparen) = stat_str.rfind(')')
        {
            let rest: Vec<&str> = stat_str[rparen + 1..].split_whitespace().collect();
            // After ')', index 3 is session (field 6), index 19 is starttime (field 22)
            if let Some(&sess_id) = rest.get(3) {
                let clean_agent = comm.strip_suffix("-bin").unwrap_or(&comm).to_string();
                let etimes = rest
                    .get(19)
                    .and_then(|st| st.parse::<u64>().ok())
                    .map(|ticks| uptime_secs.saturating_sub(ticks / 100))
                    .unwrap_or(0);

                sess_agent.insert(sess_id.to_string(), clean_agent);
                sess_etimes.insert(sess_id.to_string(), etimes);
            }
        }
    }

    (sess_agent, sess_etimes)
}

pub fn format_age(diff_s: u64) -> String {
    if diff_s == 0 {
        String::new()
    } else if diff_s < 60 {
        format!("{}s", diff_s)
    } else if diff_s < 3600 {
        format!("{}m", diff_s / 60)
    } else if diff_s < 86400 {
        format!("{}h{}m", diff_s / 3600, (diff_s % 3600) / 60)
    } else {
        format!("{}d", diff_s / 86400)
    }
}

#[derive(Debug, Clone, Default)]
struct GitMetadata {
    is_repo: bool,
    branch: String,
    has_u: bool,
    ahead: u32,
    behind: u32,
    diffstat: String,
    pr_num: String,
    pr_ci: String,
}

fn parse_pr_cache_json(path: &Path) -> (String, String) {
    let content = match fs::read_to_string(path) {
        Ok(c) if !c.trim().is_empty() => c,
        _ => return (String::new(), String::new()),
    };
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return (String::new(), String::new()),
    };

    let pr_num = match val.get("number") {
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    };

    let commit_state = val
        .get("commit_state")
        .and_then(|s| s.as_str())
        .unwrap_or("none");

    let ci_state = match val.get("statusCheckRollup").and_then(|v| v.as_array()) {
        None => commit_state.to_string(),
        Some(arr) if arr.is_empty() => commit_state.to_string(),
        Some(arr) => {
            let has_fail = arr.iter().any(|item| {
                matches!(
                    item.get("conclusion").and_then(|c| c.as_str()),
                    Some("FAILURE" | "TIMED_OUT")
                )
            });
            let has_pending = arr.iter().any(|item| {
                matches!(
                    item.get("status").and_then(|s| s.as_str()),
                    Some("IN_PROGRESS" | "QUEUED")
                ) || item.get("conclusion").is_none_or(|c| c.is_null())
            });
            let has_success = arr.iter().any(|item| {
                matches!(
                    item.get("conclusion").and_then(|c| c.as_str()),
                    Some("SUCCESS")
                )
            });

            if has_fail {
                "failure".to_string()
            } else if has_pending {
                "pending".to_string()
            } else if has_success {
                "success".to_string()
            } else {
                commit_state.to_string()
            }
        }
    };

    (pr_num, ci_state)
}

fn resolve_git_metadata(ppath: &str, now_s: u64) -> GitMetadata {
    let mut meta = GitMetadata::default();
    let out = match Command::new("git")
        .args([
            "-C",
            ppath,
            "-c",
            "submodule.recurse=false",
            "--no-optional-locks",
            "status",
            "--porcelain",
            "-b",
        ])
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return meta,
    };

    if out.trim().is_empty() {
        return meta;
    }

    meta.is_repo = true;
    let mut lines = out.lines();
    if let Some(first_line) = lines.next() {
        let br_part = first_line.strip_prefix("## ").unwrap_or(first_line);
        if let Some((br, rest)) = br_part.split_once("...") {
            meta.branch = br.trim().to_string();
            meta.has_u = true;
            if let Some(idx) = rest.find("ahead ") {
                let after = &rest[idx + 6..];
                let num_s: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                meta.ahead = num_s.parse().unwrap_or(0);
            }
            if let Some(idx) = rest.find("behind ") {
                let after = &rest[idx + 7..];
                let num_s: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                meta.behind = num_s.parse().unwrap_or(0);
            }
        } else {
            meta.branch = br_part.split_whitespace().next().unwrap_or("").to_string();
        }
    }

    let dirty_files = lines
        .filter(|l| !l.is_empty() && !l.starts_with("## "))
        .count();
    if dirty_files > 0 {
        let mut ds = String::new();
        if let Ok(dout) = Command::new("git")
            .args([
                "-C",
                ppath,
                "-c",
                "submodule.recurse=false",
                "--no-optional-locks",
                "diff",
                "--no-ext-diff",
                "--shortstat",
                "HEAD",
            ])
            .stderr(Stdio::null())
            .output()
            && dout.status.success()
        {
            let dstr = String::from_utf8_lossy(&dout.stdout);
            for part in dstr.split(',') {
                let p = part.trim();
                if p.contains("insertion") {
                    if let Some(n) = p.split_whitespace().next() {
                        ds.push_str(&format!("+{}", n));
                    }
                } else if p.contains("deletion")
                    && let Some(n) = p.split_whitespace().next()
                {
                    if !ds.is_empty() {
                        ds.push(' ');
                    }
                    ds.push_str(&format!("-{}", n));
                }
            }
        }
        if ds.is_empty() {
            ds = format!("*{}", dirty_files);
        }
        meta.diffstat = ds;
    }

    let br = &meta.branch;
    if !br.is_empty() && !matches!(br.as_str(), "main" | "master" | "HEAD" | "detached") {
        let hash_key = format!("{}_{}", ppath.replace('/', "_"), br.replace('/', "_"));
        let cache_file = format!("/tmp/gh-pr-cache-{}.json", hash_key);
        let cache_path = Path::new(&cache_file);

        let cache_age = fs::metadata(cache_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| now_s.saturating_sub(d.as_secs()))
            .unwrap_or(9999);

        if cache_age > 60 {
            let _ = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(cache_path);
            let ppath_owned = ppath.to_string();
            let cache_owned = cache_file.clone();
            let bg_script = format!(
                "cd '{}' 2>/dev/null || exit 0; \
                 if gh pr view --json number,title,state,statusCheckRollup > '{}.tmp' 2>/dev/null && [ -s '{}.tmp' ]; then \
                   mv '{}.tmp' '{}'; \
                 else \
                   c_sha=$(git rev-parse HEAD 2>/dev/null || echo ''); \
                   if [ -n \"$c_sha\" ]; then \
                     c_state=$(gh api \"repos/:owner/:repo/commits/$c_sha/status\" --jq .state 2>/dev/null || echo 'none'); \
                     if [ -n \"$c_state\" ] && [ \"$c_state\" != 'none' ]; then \
                       echo \"{{\\\"commit_state\\\":\\\"$c_state\\\"}}\" > '{}'; \
                     else \
                       echo '{{}}' > '{}'; \
                     fi; \
                   else \
                     echo '{{}}' > '{}'; \
                   fi; \
                   rm -f '{}.tmp'; \
                 fi",
                ppath_owned,
                cache_owned,
                cache_owned,
                cache_owned,
                cache_owned,
                cache_owned,
                cache_owned,
                cache_owned,
                cache_owned
            );
            let _ = Command::new("sh")
                .args(["-c", &bg_script])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }

        let (pr_n, pr_c) = parse_pr_cache_json(cache_path);
        meta.pr_num = pr_n;
        meta.pr_ci = pr_c;
    }

    meta
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if char_len(s) <= max_chars {
        return s.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    let prefix: String = s.chars().take(max_chars - 1).collect();
    format!("{}…", prefix)
}

#[derive(Debug, Clone)]
struct RawPaneRow {
    prio: u8,
    ts: u64,
    sess: String,
    idx: String,
    idx_num: u32,
    wname: String,
    pid: String,
    ppath: String,
    ico: String,
    agent_label: String,
    age_str: String,
    branch: String,
    is_cur: bool,
    st: String,
    is_agent: bool,
    has_u: bool,
    u_ahead: u32,
    u_behind: u32,
    diffstat_str: String,
    pr_num: String,
    pr_ci: String,
}

pub fn generate_items(filter_agents: bool, orig_sess: &str, orig_win: &str) -> String {
    let theme = ThemePalette::load();
    let mut cur_session = orig_sess.to_string();
    let mut cur_window = orig_win.to_string();

    if cur_session.is_empty() || cur_window.is_empty() {
        if let Some(env_s) = std::env::var("TMUX_ORIGIN_SESSION")
            .ok()
            .filter(|s| !s.is_empty())
            && cur_session.is_empty()
        {
            cur_session = env_s;
        }
        if let Some(env_w) = std::env::var("TMUX_ORIGIN_WINDOW")
            .ok()
            .filter(|s| !s.is_empty())
            && cur_window.is_empty()
        {
            cur_window = env_w;
        }
        if (cur_session.is_empty() || cur_window.is_empty())
            && let Some(out) =
                tmux_out(&["display-message", "-p", "#{session_name}\t#{window_index}"])
            && let Some((ts, tw)) = out.split_once('\t')
        {
            if cur_session.is_empty() {
                cur_session = ts.to_string();
            }
            if cur_window.is_empty() {
                cur_window = tw.to_string();
            }
        }
    }

    let acpd_map = rpc_client::fetch_agent_states();
    let (sess_agent, sess_etimes) = scan_running_agents();

    let now_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now_ms = now_s * 1000;

    let fmt = "#{session_name}\x1f#{window_index}\x1f#{window_name}\x1f#{pane_id}\x1f#{pane_pid}\x1f#{pane_current_path}\x1f#{pane_current_command}\x1f#{pane_title}\x1f#{?@ai_agent_state_raw,#{@ai_agent_state_raw},#{@w:@ai_agent_state_raw}}\x1f#{?@ai_agent_title,#{@ai_agent_title},#{@w:@ai_agent_title}}";
    let list_out = tmux_out(&["list-panes", "-a", "-F", fmt]).unwrap_or_default();

    let mut git_cache: HashMap<String, GitMetadata> = HashMap::new();
    let mut raw_rows: Vec<RawPaneRow> = Vec::new();

    for line in list_out.lines() {
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\x1f').collect();
        if fields.len() < 10 {
            continue;
        }
        let sess = fields[0];
        if sess.starts_with("_lazygitrs") || sess.starts_with("_popups") || sess.starts_with('.') {
            continue;
        }
        let idx = fields[1];
        let wname = fields[2];
        let pid = fields[3];
        let pane_os_pid = fields[4];
        let mut ppath = fields[5].to_string();
        let pcmd = fields[6];
        let ptitle = fields[7];
        let raw_state = fields[8];
        let ai_title = fields[9];

        let detected_agent = sess_agent.get(pane_os_pid).cloned().unwrap_or_default();

        let (mut st, ts) = match acpd_map.get(pid) {
            Some(entry) => (entry.state.clone(), entry.last_timestamp),
            None => (String::new(), 0),
        };

        if st.is_empty() {
            st = raw_state.to_string();
        }
        if st.is_empty() {
            if !detected_agent.is_empty() {
                st = "idle".to_string();
            } else {
                st = "normal".to_string();
            }
        }

        let (prio, ico) = match st.as_str() {
            "permission" => (1, "󱅭"),
            "question" | "awaiting_input" => (2, "󱜻"),
            "stalled" | "hanging" => (2, "󱥁"),
            "error" => (3, "󰨄"),
            "busy" | "working" => (4, "@SPIN@"),
            "idle" => (5, "󱥂"),
            _ => {
                if !detected_agent.is_empty()
                    || is_agent_binary_name(pcmd)
                    || contains_agent_name(ptitle)
                {
                    (5, "󰑮")
                } else {
                    (6, "·")
                }
            }
        };

        let diff_s = if ts > 0 {
            if ts < 10_000_000_000 {
                now_s.saturating_sub(ts)
            } else {
                now_ms.saturating_sub(ts) / 1000
            }
        } else {
            sess_etimes.get(pane_os_pid).copied().unwrap_or(0)
        };
        let age_str = format_age(diff_s);

        let mut git_meta = GitMetadata::default();
        if !ppath.is_empty() && Path::new(&ppath).is_dir() {
            let p_obj = Path::new(&ppath);
            if !p_obj.join(".git").exists()
                && p_obj.join(".bare").is_dir()
                && p_obj.join("main").is_dir()
            {
                ppath = format!("{}/main", ppath);
            }
            git_meta = git_cache
                .entry(ppath.clone())
                .or_insert_with(|| resolve_git_metadata(&ppath, now_s))
                .clone();
        }

        let is_agent = !detected_agent.is_empty()
            || is_agent_binary_name(pcmd)
            || !ai_title.is_empty()
            || (!st.is_empty() && st != "normal");

        let raw_label = if !ai_title.is_empty() {
            ai_title
        } else if !detected_agent.is_empty() {
            &detected_agent
        } else if is_agent_binary_name(pcmd) {
            pcmd
        } else {
            ""
        };

        let clean_label = raw_label
            .replace(['\t', '\r', '\n'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let short_label = truncate_chars(&clean_label, 24);

        let is_cur = sess == cur_session && idx == cur_window;
        let idx_num = idx.parse::<u32>().unwrap_or(0);

        raw_rows.push(RawPaneRow {
            prio,
            ts,
            sess: sess.to_string(),
            idx: idx.to_string(),
            idx_num,
            wname: wname.to_string(),
            pid: pid.to_string(),
            ppath,
            ico: ico.to_string(),
            agent_label: short_label,
            age_str,
            branch: git_meta.branch,
            is_cur,
            st,
            is_agent,
            has_u: git_meta.has_u,
            u_ahead: git_meta.ahead,
            u_behind: git_meta.behind,
            diffstat_str: git_meta.diffstat,
            pr_num: git_meta.pr_num,
            pr_ci: git_meta.pr_ci,
        });
    }

    raw_rows.sort_by(|a, b| a.prio.cmp(&b.prio).then_with(|| a.idx_num.cmp(&b.idx_num)));

    let mut sess_prio: HashMap<String, u8> = HashMap::new();
    let mut sess_ts: HashMap<String, u64> = HashMap::new();
    let mut session_items: HashMap<String, String> = HashMap::new();
    let mut session_agent_items: HashMap<String, String> = HashMap::new();
    let mut seen_windows: HashSet<String> = HashSet::new();
    let mut agent_count = 0usize;

    for row in raw_rows {
        if row.is_agent {
            agent_count += 1;
        }
        let cur_p = sess_prio.get(&row.sess).copied().unwrap_or(99);
        if row.prio < cur_p {
            sess_prio.insert(row.sess.clone(), row.prio);
            sess_ts.insert(row.sess.clone(), row.ts);
        }

        let win_key = format!("{}:{}", row.sess, row.idx);
        if !seen_windows.insert(win_key) {
            continue;
        }

        let c_ico = match row.st.as_str() {
            "permission" => &theme.c_perm,
            "question" | "awaiting_input" => &theme.c_question,
            "stalled" | "hanging" => &theme.c_perm,
            "error" => &theme.c_error,
            "busy" | "working" => &theme.c_busy,
            "idle" => &theme.c_idle,
            _ => &theme.c_normal,
        };

        let ai_ico = if row.is_agent {
            format!("{}{}{} ", c_ico, row.ico, RESET)
        } else {
            format!("{}·{} ", theme.c_muted, RESET)
        };

        let (cur_dot, c_wname) = if row.is_cur {
            (format!("{}•{}", theme.c_cur, RESET), &theme.c_cur)
        } else {
            (format!("{}·{}", theme.c_muted, RESET), &theme.c_name)
        };

        let idx_col = if char_len(&row.idx) == 1 {
            format!("{}{}{}{} ", theme.c_muted, row.idx, RESET, cur_dot)
        } else {
            format!("{}{}{}{}", theme.c_muted, row.idx, RESET, cur_dot)
        };

        // Col 3: Fixed-width Window + Agent Title Column (24 visual chars)
        let w_len = char_len(&row.wname);
        let title_col = if !row.agent_label.is_empty() && row.agent_label != row.wname {
            let max_lbl = 24usize.saturating_sub(w_len + 1).max(4);
            let lbl_trunc = truncate_chars(&row.agent_label, max_lbl);
            let vis_len = w_len + 1 + char_len(&lbl_trunc);
            let pad_t = 24usize.saturating_sub(vis_len);
            let spaces = " ".repeat(pad_t);
            format!(
                "{}{}{}{} {}{}{}{}",
                c_wname, row.wname, RESET, "", c_ico, lbl_trunc, RESET, spaces
            )
        } else {
            let w_trunc = truncate_chars(&row.wname, 24);
            let vis_len = char_len(&w_trunc);
            let pad_t = 24usize.saturating_sub(vis_len);
            let spaces = " ".repeat(pad_t);
            format!("{}{}{}{}", c_wname, w_trunc, RESET, spaces)
        };

        // Col 4: Fixed-width Branch + Remote Sync Column (14 visual chars)
        let mut sync_search = String::new();
        let branch_col = if !row.branch.is_empty() {
            let br_trunc = truncate_chars(&row.branch, 8);
            let mut sync_glyph = String::new();
            let mut sync_col_c = &theme.c_cyan;
            if row.has_u {
                if row.u_ahead == 0 && row.u_behind == 0 {
                    sync_glyph = " 󰄬".to_string();
                    sync_search = "synced".to_string();
                } else {
                    if row.u_ahead > 0 {
                        sync_glyph.push_str(&format!(" ↑{}", row.u_ahead));
                        sync_search.push_str(" ahead");
                    }
                    if row.u_behind > 0 {
                        sync_glyph.push_str(&format!(" ↓{}", row.u_behind));
                        sync_col_c = &theme.c_error;
                        sync_search.push_str(" behind");
                    }
                    sync_search = sync_search.trim().to_string();
                }
            }
            let vis_br_len = 2 + char_len(&br_trunc) + char_len(&sync_glyph);
            let pad_b = 14usize.saturating_sub(vis_br_len);
            let br_spaces = " ".repeat(pad_b);
            format!(
                "  {}⎇ {}{}{}{}{}{}",
                theme.c_branch, br_trunc, RESET, sync_col_c, sync_glyph, RESET, br_spaces
            )
        } else {
            format!("  {}-             {}", theme.c_muted, RESET)
        };

        // Col 5: Fixed-width Git Diffstat / PR Badge Column (11 visual chars)
        let mut diff_badge = String::new();
        if !row.diffstat_str.is_empty() {
            for part in row.diffstat_str.split_whitespace() {
                if !diff_badge.is_empty() {
                    diff_badge.push(' ');
                }
                if part.starts_with('+') {
                    diff_badge.push_str(&format!("{}{}{}", theme.c_diff_ins, part, RESET));
                } else if part.starts_with('-') {
                    diff_badge.push_str(&format!("{}{}{}", theme.c_diff_del, part, RESET));
                } else {
                    diff_badge.push_str(&format!("{}{}{}", theme.c_diff_dirty, part, RESET));
                }
            }
        }

        let pr_search = if !row.pr_num.is_empty() {
            format!("PR:#{} ci:{}", row.pr_num, row.pr_ci)
        } else if !row.pr_ci.is_empty() && row.pr_ci != "none" {
            format!("ci:{}", row.pr_ci)
        } else {
            String::new()
        };

        let diff_col = if !row.diffstat_str.is_empty() {
            let vis_ds_len = char_len(&row.diffstat_str);
            let pad_d = 11usize.saturating_sub(vis_ds_len);
            format!(" {}{}", diff_badge, " ".repeat(pad_d))
        } else if !row.pr_num.is_empty() {
            let vis_pr = format!("󰏫#{}", row.pr_num);
            let pad_d = 11usize.saturating_sub(char_len(&vis_pr));
            format!(" {}{}{}{}", theme.c_pr, vis_pr, RESET, " ".repeat(pad_d))
        } else {
            format!(" {}-          {}", theme.c_muted, RESET)
        };

        // Col 6: Right-aligned Elapsed Age Column (6 visual chars)
        let age_col = if !row.age_str.is_empty() {
            let pad_a = 6usize.saturating_sub(char_len(&row.age_str));
            let age_spaces = " ".repeat(pad_a);
            if row.prio <= 3 {
                format!(" {}{}{}{}{}", age_spaces, c_ico, BOLD, row.age_str, RESET)
            } else {
                format!(" {}{}{}{}", age_spaces, theme.c_muted, row.age_str, RESET)
            }
        } else {
            format!(" {}     -{}", theme.c_muted, RESET)
        };

        let display_line = format!(
            " {}{} {}{}{}{}  ",
            ai_ico, idx_col, title_col, branch_col, diff_col, age_col
        );

        let search_title = format!(
            "{} {} {} {} {} {} {} {} {} {}",
            row.sess,
            row.idx,
            row.wname,
            if row.agent_label.is_empty() {
                "-"
            } else {
                &row.agent_label
            },
            if row.branch.is_empty() {
                "-"
            } else {
                &row.branch
            },
            row.st,
            if row.age_str.is_empty() {
                "-"
            } else {
                &row.age_str
            },
            sync_search,
            pr_search,
            if row.diffstat_str.is_empty() {
                "-"
            } else {
                &row.diffstat_str
            }
        );

        let formatted_row = format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            search_title,
            row.idx,
            row.wname,
            row.sess,
            row.pid,
            row.ppath,
            row.st,
            if row.agent_label.is_empty() {
                "-"
            } else {
                &row.agent_label
            },
            if row.age_str.is_empty() {
                "-"
            } else {
                &row.age_str
            },
            if row.branch.is_empty() {
                "-"
            } else {
                &row.branch
            },
            if sync_search.is_empty() {
                "-"
            } else {
                &sync_search
            },
            display_line
        );

        session_items
            .entry(row.sess.clone())
            .or_default()
            .push_str(&formatted_row);
        if row.is_agent {
            session_agent_items
                .entry(row.sess)
                .or_default()
                .push_str(&formatted_row);
        }
    }

    let mut out = String::new();
    if filter_agents && agent_count == 0 {
        out.push_str(&format!(
            "· (no active AI agents)\t0\t-\t{}\t-\t-\t-\t-\t-\t-\t-\t \x1b[2m(no AI agents running — press a for Fleet view)\x1b[0m\n",
            cur_session
        ));
    } else {
        let mut sessions_order: Vec<(u8, u64, String)> = sess_prio
            .iter()
            .map(|(s, &p)| (p, sess_ts.get(s).copied().unwrap_or(0), s.clone()))
            .collect();
        sessions_order.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        for (_, _, sess) in sessions_order {
            let items = if filter_agents {
                session_agent_items.get(&sess)
            } else {
                session_items.get(&sess)
            };
            if let Some(rows_str) = items
                && !rows_str.is_empty()
            {
                out.push_str(&format!("#  {}\n{}", sess, rows_str));
            }
        }
    }

    out
}

pub fn run_cli(args: &[String]) -> i32 {
    let mut filter_agents = false;
    let mut cur_session = String::new();
    let mut cur_window = String::new();

    for arg in args {
        match arg.as_str() {
            "--agents" | "agents" | "-a" => filter_agents = true,
            "--fleet" | "fleet" => filter_agents = false,
            _ => {
                if cur_session.is_empty() {
                    cur_session = arg.clone();
                } else if cur_window.is_empty() {
                    cur_window = arg.clone();
                }
            }
        }
    }

    let output = generate_items(filter_agents, &cur_session, &cur_window);
    print!("{}", output);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_age() {
        assert_eq!(format_age(0), "");
        assert_eq!(format_age(45), "45s");
        assert_eq!(format_age(125), "2m");
        assert_eq!(format_age(3720), "1h2m");
        assert_eq!(format_age(90000), "1d");
    }

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("hello world", 6), "hello…");
    }
}
