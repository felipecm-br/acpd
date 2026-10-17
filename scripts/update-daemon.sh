#!/usr/bin/env bash
set -euo pipefail

# Rebuild the release binary, install atomically to ~/.local/bin, and restart the user systemd daemon.

echo "Building waymux in release mode..."
cargo build --release

mkdir -p "$HOME/.local/bin"
install -m 755 target/release/waymux "$HOME/.local/bin/waymux"
ln -sf "$HOME/.local/bin/waymux" "$HOME/.local/bin/acpd"

echo "Reloading and restarting waymux.service..."
systemctl --user daemon-reload
systemctl --user restart waymux.service

echo "Done! waymux.service is running the latest build:"
systemctl --user status waymux.service --no-pager | head -n 14
