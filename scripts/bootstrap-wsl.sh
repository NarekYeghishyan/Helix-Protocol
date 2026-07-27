#!/usr/bin/env bash
#
# Helix Protocol — development toolchain bootstrap for Ubuntu/WSL2.
#
# Installs: build deps, Rust, Solana CLI (Agave), Anchor (via avm), Node LTS, Surfpool.
# Idempotent: re-running skips anything already present.
#
# Usage:  bash scripts/bootstrap-wsl.sh
#
set -euo pipefail

ANCHOR_VERSION="${ANCHOR_VERSION:-1.1.2}"
NODE_VERSION="${NODE_VERSION:---lts}"

log()  { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
skip() { printf '\033[0;90m    (already installed: %s)\033[0m\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Append a line to ~/.bashrc only if it is not already there.
ensure_bashrc() {
  local line="$1"
  grep -qxF "$line" "$HOME/.bashrc" 2>/dev/null || echo "$line" >>"$HOME/.bashrc"
}

# ---------------------------------------------------------------- System deps
# Only this step needs root. Skipped entirely when the deps are already present,
# so re-runs never prompt for a password.
if have cc && have pkg-config && have protoc && [ -f /usr/include/openssl/ssl.h ]; then
  skip "system build dependencies"
else
  log "Installing system build dependencies (requires sudo, once)"
  sudo -v
  sudo apt-get update -qq
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    build-essential pkg-config libssl-dev libudev-dev \
    llvm libclang-dev protobuf-compiler \
    curl jq unzip ca-certificates
fi

# ---------------------------------------------------------------- Rust
if have rustc; then
  skip "$(rustc --version)"
else
  log "Installing Rust (stable)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable -c rustfmt -c clippy
fi
export PATH="$HOME/.cargo/bin:$PATH"
ensure_bashrc 'export PATH="$HOME/.cargo/bin:$PATH"'

# ---------------------------------------------------------------- Solana CLI
if have solana; then
  skip "$(solana --version)"
else
  log "Installing Solana CLI (Agave, stable channel)"
  sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"
fi
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
ensure_bashrc 'export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"'

# ---------------------------------------------------------------- Anchor
if have anchor && [ "$(anchor --version 2>/dev/null | awk '{print $2}')" = "$ANCHOR_VERSION" ]; then
  skip "anchor $ANCHOR_VERSION"
else
  log "Installing avm + Anchor $ANCHOR_VERSION (this takes a while — it builds from source)"
  have avm || cargo install --locked avm
  avm install "$ANCHOR_VERSION"
  avm use "$ANCHOR_VERSION"
fi

# ---------------------------------------------------------------- Node
if have node; then
  skip "node $(node --version)"
else
  log "Installing nvm + Node LTS"
  curl -sSfL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
  export NVM_DIR="$HOME/.nvm"
  # shellcheck disable=SC1091
  . "$NVM_DIR/nvm.sh"
  nvm install $NODE_VERSION
  nvm alias default node
fi

# ---------------------------------------------------------------- Surfpool
# Anchor >=1.0 uses Surfpool as the default local validator for `anchor test`.
if have surfpool; then
  skip "$(surfpool --version 2>/dev/null || echo surfpool)"
else
  log "Installing Surfpool (default validator for anchor test)"
  curl -sSL https://run.surfpool.run/ | bash || {
    echo "    Surfpool install failed — you can still run: anchor test --validator legacy"
  }
fi

# ---------------------------------------------------------------- Keypair
if [ ! -f "$HOME/.config/solana/id.json" ]; then
  log "Generating a local development keypair"
  solana-keygen new --no-bip39-passphrase --silent --outfile "$HOME/.config/solana/id.json"
fi

# ---------------------------------------------------------------- Summary
log "Installed versions"
for t in rustc cargo solana anchor node npm surfpool; do
  if have "$t"; then
    printf '  %-9s %s\n' "$t" "$("$t" --version 2>&1 | head -1)"
  else
    printf '  %-9s \033[0;33mnot on PATH (open a new shell)\033[0m\n' "$t"
  fi
done

log "Done. Open a new shell (or 'source ~/.bashrc') so PATH changes take effect."
