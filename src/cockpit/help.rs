use crate::theme::{BOLD, DIM, RESET, ThemePalette};
use std::path::Path;
use std::process::Command;

pub fn run_cli() -> i32 {
    let theme = ThemePalette::load();
    let c_cyan = &theme.c_cyan;
    let c_yellow = &theme.c_busy;
    let c_red = &theme.c_error;
    let c_magenta = &theme.c_question;
    let c_blue = &theme.c_branch;

    if let Ok(home) = std::env::var("HOME") {
        let telemetry_py = format!("{}/.config/tmux/flow-telemetry.py", home);
        if Path::new(&telemetry_py).is_file() {
            let _ = Command::new("python3")
                .args([&telemetry_py, "--hud"])
                .status();
        }
    }

    println!(
        "  {}{}󱂬 Cockpit HUD — Keybindings & Shortcuts{}\n",
        c_cyan, BOLD, RESET
    );

    println!("  {}{}󰊢 Worktree Lifecycle{}", c_yellow, BOLD, RESET);
    println!(
        "    {}{}[w / c]{}  New Worktree      Launch 5-step AWT wizard",
        c_yellow, BOLD, RESET
    );
    println!(
        "    {}{}[S]{}      Ship Worktree     Merge, push & clean worktree",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[W]{}      Sweep Worktrees   Batch-prune merged worktrees\n",
        c_yellow, BOLD, RESET
    );

    println!("  {}{}󱚥 AI Fleet & Agent Triage{}", c_magenta, BOLD, RESET);
    println!(
        "    {}{}[y / n]{}  Approve / Deny    Guarded prompt reply (permission/question only)",
        c_yellow, BOLD, RESET
    );
    println!(
        "    {}{}[r]{}      Review Notes      Dispatch Lazygitrs review notes to agent",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[i]{}      Live Passthrough  Stream keystrokes to agent ([Esc] exit)",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[I]{}      Prompt Agent      Send sanitized inline prompt to agent",
        c_magenta, BOLD, RESET
    );
    println!(
        "    {}{}[a]{}      Toggle Fleet      Fleet View (all windows) ↔ Agent Triage",
        c_yellow, BOLD, RESET
    );
    println!(
        "    {}{}[Bksp]{}   Last Agent        Jump back to previously focused agent",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[R]{}      Reap Agents       Garbage-collect stale agents\n",
        c_red, BOLD, RESET
    );

    println!("  {}{}󰓩 Git & Pull Requests{}", c_blue, BOLD, RESET);
    println!(
        "    {}{}[o]{}      Open PR           Open GitHub pull request in browser",
        c_blue, BOLD, RESET
    );
    println!(
        "    {}{}[O]{}      Open PR Checks    Open PR CI status checks in browser",
        c_blue, BOLD, RESET
    );
    println!(
        "    {}{}[v]{}      Diff View         Toggle side-by-side Git Diff",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[g]{}      Git Cockpit       Toggle Lazygitrs (Dual-Diff, g returns)",
        c_magenta, BOLD, RESET
    );
    println!(
        "    {}{}[p]{}      Patch Mode        Turbo Hunk Staging in Lazygitrs\n",
        c_magenta, BOLD, RESET
    );

    println!("  {}{}󰌌 Navigation & Windows{}", c_cyan, BOLD, RESET);
    println!(
        "    {}{}[Enter]{}  Jump / Connect    Focus selected window/session",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[Tab]{}    AWT Worktrees     Switch to AWT Worktree popup",
        c_yellow, BOLD, RESET
    );
    println!(
        "    {}{}[s]{}      Sessions          Open Session Picker (Sesh)",
        c_yellow, BOLD, RESET
    );
    println!(
        "    {}{}[t]{}      New Window        Create window in current path",
        c_cyan, BOLD, RESET
    );
    println!(
        "    {}{}[d / C-d]{} Kill / Delete     Kill window or delete session",
        c_red, BOLD, RESET
    );
    println!(
        "    {}{}[J / K]{}  Scroll Preview    Scroll preview pane up/down",
        c_blue, BOLD, RESET
    );
    println!(
        "    {}{}[+ / -]{}  Resize Preview    Expand/shrink preview split",
        c_blue, BOLD, RESET
    );
    println!(
        "    {}{}[/]{}      Filter            Enter fuzzy query filter mode",
        c_blue, BOLD, RESET
    );
    println!(
        "    {}{}[Esc]{}    Exit              Dismiss Cockpit modal\n",
        c_blue, BOLD, RESET
    );

    println!(
        "  {}────────────────────────────────────────────────────────────{}",
        DIM, RESET
    );
    println!(
        "  {}{}?{} {}HUD help{}   {}│{}   {}{}v{} {}Diff{}   {}│{}   {}{}g{} {}Git{}   {}│{}   {}{}p{} {}Patch{}",
        c_cyan,
        BOLD,
        RESET,
        DIM,
        RESET,
        DIM,
        RESET,
        c_cyan,
        BOLD,
        RESET,
        DIM,
        RESET,
        DIM,
        RESET,
        c_cyan,
        BOLD,
        RESET,
        DIM,
        RESET,
        DIM,
        RESET,
        c_cyan,
        BOLD,
        RESET,
        DIM,
        RESET
    );

    0
}
