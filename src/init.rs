pub fn run_cli(args: &[String]) -> i32 {
    let flag = args.first().map(|s| s.as_str()).unwrap_or("--help");
    match flag {
        "--tmux" | "tmux" => {
            println!(
                "# Waymux Tmux Integration (Event-Driven AI Cockpit)\n\
set -g status-interval 0\n\
bind-key c run-shell -b \"waymux cockpit\"\n\
bind-key C run-shell -b \"waymux cockpit --fullscreen\"\n\
bind-key i run-shell -b \"waymux triage --popup\"\n\
bind-key I run-shell -b \"waymux triage --jump\"\n\
bind-key -n C-S-g run-shell -b \"waymux wt popup\""
            );
            0
        }
        "--systemd" | "systemd" => {
            println!(
                "[Unit]\n\
Description=Waymux: Terminal AI Cockpit & Agent Control Plane Daemon\n\
After=network-online.target\n\
Wants=network-online.target\n\n\
[Service]\n\
Type=notify\n\
ExecStart=%h/.local/bin/waymux daemon\n\
ExecReload=/bin/kill -HUP $MAINPID\n\
Restart=on-failure\n\
RestartSec=5s\n\
RuntimeDirectory=waymux acpd\n\n\
[Install]\n\
WantedBy=default.target"
            );
            0
        }
        "--hooks" | "hooks" => {
            println!(
                "# Waymux Zero-Latency Agent Hooks\n\
# Antigravity (~/.gemini/antigravity-cli/hooks.json):\n\
#   tmux-hook:    waymux hook tmux <Event>\n\
#   lazygit-hook: waymux hook lazygit <Event>"
            );
            0
        }
        _ => {
            println!(
                "waymux init — Portable Zero-Config Bootstrapper\n\n\
USAGE:\n    \
waymux init --tmux       Print Tmux keybindings and status bar integration\n    \
waymux init --systemd    Print user systemd unit file for acpd/waymux daemon\n    \
waymux init --hooks      Print AI agent hook integration snippets"
            );
            0
        }
    }
}
