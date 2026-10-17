#!/usr/bin/env bash
# ==============================================================================
# Waymux Universal Installer
# ==============================================================================
# Installs the pre-built static `waymux` binary (and `acpd` compatibility alias),
# configures the user-level systemd service (`waymux.service`), and outputs
# ready-to-use Tmux and AI agent hook snippets.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/felipecm-br/acpd/main/install.sh | bash
#   ./install.sh [--from-source] [--prefix ~/.local/bin] [--no-systemd]
# ==============================================================================

set -euo pipefail

REPO="felipecm-br/acpd"
PREFIX="${PREFIX:-$HOME/.local/bin}"
FROM_SOURCE=false
SETUP_SYSTEMD=true

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from-source)
      FROM_SOURCE=true
      shift
      ;;
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --no-systemd)
      SETUP_SYSTEMD=false
      shift
      ;;
    -h|--help)
      echo "Usage: install.sh [--from-source] [--prefix <dir>] [--no-systemd]"
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

mkdir -p "$PREFIX"

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *)
      echo "Unsupported architecture: $arch" >&2
      return 1
      ;;
  esac

  case "$os" in
    linux) echo "${arch}-unknown-linux-musl" ;;
    darwin) echo "${arch}-apple-darwin" ;;
    *)
      echo "Unsupported OS: $os" >&2
      return 1
      ;;
  esac
}

install_from_release() {
  local target tag url tmp_dir archive
  target="$(detect_target)" || return 1

  tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep -m1 '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)"
  if [[ -z "$tag" ]]; then
    return 1
  fi

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' RETURN

  for bin_name in "waymux" "acpd"; do
    archive="${bin_name}-${tag}-${target}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${tag}/${archive}"
    if curl -fsSL "$url" -o "${tmp_dir}/${archive}" 2>/dev/null; then
      if curl -fsSL "${url}.sha256" -o "${tmp_dir}/${archive}.sha256" 2>/dev/null; then
        (cd "$tmp_dir" && sha256sum -c "${archive}.sha256" >/dev/null 2>&1) || {
          echo "Checksum verification failed for ${archive}" >&2
          return 1
        }
      fi
      tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir"
      if [[ -f "${tmp_dir}/waymux" ]]; then
        install -m 755 "${tmp_dir}/waymux" "${PREFIX}/waymux"
      elif [[ -f "${tmp_dir}/acpd" ]]; then
        install -m 755 "${tmp_dir}/acpd" "${PREFIX}/waymux"
      fi
      ln -sf "${PREFIX}/waymux" "${PREFIX}/acpd"
      echo "✓ Installed pre-built waymux (${tag}, ${target}) to ${PREFIX}/waymux"
      return 0
    fi
  done

  return 1
}

install_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: cargo is required for --from-source installation." >&2
    exit 1
  fi
  echo "Building waymux from source (release profile)..."
  if [[ -f "Cargo.toml" ]] && grep -q 'name = "waymux"' Cargo.toml 2>/dev/null; then
    cargo build --release --bin waymux
    install -m 755 target/release/waymux "${PREFIX}/waymux"
  else
    cargo install --git "https://github.com/${REPO}.git" --bin waymux --root "${PREFIX%/bin}" --force
  fi
  ln -sf "${PREFIX}/waymux" "${PREFIX}/acpd"
  echo "✓ Installed waymux to ${PREFIX}/waymux (and symlinked ${PREFIX}/acpd)"
}

if [[ "$FROM_SOURCE" == "true" ]]; then
  install_from_source
else
  if ! install_from_release; then
    echo "Pre-built binary not found for current platform; falling back to source build..."
    install_from_source
  fi
fi

if [[ "$SETUP_SYSTEMD" == "true" ]] && command -v systemctl >/dev/null 2>&1; then
  mkdir -p "$HOME/.config/systemd/user"
  "${PREFIX}/waymux" init --systemd > "$HOME/.config/systemd/user/waymux.service"
  systemctl --user daemon-reload 2>/dev/null || true
  systemctl --user enable --now waymux.service 2>/dev/null || true
  echo "✓ Enabled and started user systemd service: waymux.service"
fi

echo ""
echo "━━━ Waymux Installation Complete ━━━"
echo "Binary:   ${PREFIX}/waymux"
echo "Health:   $( "${PREFIX}/waymux" health 2>/dev/null || echo 'run: systemctl --user start waymux.service' )"
echo ""
echo "Next steps:"
echo "  1. Add Tmux bindings:  waymux init --tmux >> ~/.tmux.conf"
echo "  2. Configure AI hooks: waymux init --hooks"
