default:
	@just --list

build:
	cargo build

install:
	cargo build --release
	mkdir -p "$HOME/.local/bin"
	install -m 755 target/release/waymux "$HOME/.local/bin/waymux"
	ln -sf "$HOME/.local/bin/waymux" "$HOME/.local/bin/acpd"

# Build static x86_64 binaries for Linux (musl)
build-x86:
	cargo zigbuild --release --target x86_64-unknown-linux-musl

# Build static ARM64 binaries for Linux (musl)
build-arm:
	cargo zigbuild --release --target aarch64-unknown-linux-musl

# Package release archives for all architectures into dist/
dist version:
	mkdir -p dist
	@echo "Building Linux x86_64 (musl)..."
	cargo zigbuild --release --target x86_64-unknown-linux-musl
	tar -czf dist/waymux-{{version}}-x86_64-unknown-linux-musl.tar.gz -C target/x86_64-unknown-linux-musl/release waymux acpd
	@echo "Building Linux ARM64 (musl)..."
	cargo zigbuild --release --target aarch64-unknown-linux-musl
	tar -czf dist/waymux-{{version}}-aarch64-unknown-linux-musl.tar.gz -C target/aarch64-unknown-linux-musl/release waymux acpd
	@echo "Generating SHA256 checksums..."
	cd dist && sha256sum waymux-{{version}}-* > SHA256SUMS.txt
	@echo "Artifacts generated in dist/:"
	@ls -lh dist/
