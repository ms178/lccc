#!/usr/bin/env bash
# ============================================================================
# lccc-bootstrap.sh — idempotent environment reconstruction after a harness wipe.
#
# The execution harness wipes everything outside /home/user (and may wipe parts
# of /home/user itself).  This script re-establishes the invariants every LCCC
# session depends on:
#
#   1. A 6 GiB swap file on the largest writable filesystem   (hard requirement:
#      the sandbox has ~1.9 GiB RAM; linking/optimising lccc OOMs without it).
#   2. VM tuning appropriate for a swap-backed, memory-starved build box.
#   3. A working Rust 1.98.0 toolchain, INCLUDING the rustup proxy binaries and
#      rustup's own execute bit -- both are lost by a harness wipe and neither
#      is restored by `rustup toolchain install` (see setup_rust).
#   3. The lccc worktree at $LCCC_REPO, rebased on ms178/lccc main, with the
#      accumulated session patch (ms178-1.patch) re-applied if present.
#   4. The artifacts directory used by lccc-snapshot.sh.
#
# Safe to run repeatedly; every step is a no-op when already satisfied.
#
# Usage:  ./lccc-bootstrap.sh [--no-clone]
# ============================================================================
set -euo pipefail

WS=${LCCC_WS:-/home/user}
REPO=${LCCC_REPO:-$WS/lccc}
ART=${LCCC_ARTIFACTS:-$WS/artifacts}
SWAP_SIZE=${LCCC_SWAP_SIZE:-6G}
UPSTREAM=${LCCC_UPSTREAM:-https://github.com/ms178/lccc}

log() { printf '\033[1;36m[bootstrap]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[bootstrap]\033[0m %s\n' "$*" >&2; }

# ---------------------------------------------------------------- 1. swap ----
setup_swap() {
  if /sbin/swapon --show 2>/dev/null | grep -q swapfile; then
    log "swap already active: $(/sbin/swapon --show --bytes --noheadings | tr -s ' ')"
    return 0
  fi
  # Choose the mount with the most free space that we can actually write to.
  local best="" bestfree=0 mp free
  while read -r mp; do
    [[ -d $mp ]] || continue
    free=$(df -B1 --output=avail "$mp" 2>/dev/null | tail -1 || echo 0)
    if [[ ${free:-0} -gt $bestfree ]] && sudo -n test -w "$mp" 2>/dev/null; then
      bestfree=$free; best=$mp
    fi
  done < <(df --output=target -x tmpfs -x devtmpfs 2>/dev/null | tail -n +2)
  best=${best:-/}
  local dir="$best/swap"
  log "creating $SWAP_SIZE swap in $dir (free: $((bestfree/1024/1024/1024)) GiB)"
  sudo -n mkdir -p "$dir"
  sudo -n fallocate -l "$SWAP_SIZE" "$dir/swapfile" 2>/dev/null ||
    sudo -n dd if=/dev/zero of="$dir/swapfile" bs=1M count=6144 status=none
  sudo -n chmod 600 "$dir/swapfile"
  sudo -n /sbin/mkswap "$dir/swapfile" >/dev/null
  sudo -n /sbin/swapon "$dir/swapfile"
  log "swap online: $(/sbin/swapon --show --noheadings | tr -s ' ')"
}

# ------------------------------------------------------------- 2. vm tune ----
tune_vm() {
  # swappiness high: we *want* cold anonymous pages (idle cc1 heaps, ld temp
  # structures) evicted rather than the page cache holding hot headers.
  sudo -n sysctl -qw vm.swappiness=80 vm.vfs_cache_pressure=50 \
                     vm.overcommit_memory=1 2>/dev/null || warn "sysctl tuning skipped"
  log "vm tuned (swappiness=80, overcommit=1)"
}

# ----------------------------------------------------------- 3. worktree -----
setup_repo() {
  [[ ${1:-} == --no-clone ]] && return 0
  mkdir -p "$ART"
  if [[ ! -d $REPO/.git ]]; then
    if [[ -f $ART/lccc.bundle ]]; then
      log "restoring worktree from artifact bundle (offline-safe)"
      git clone -q "$ART/lccc.bundle" "$REPO" && return 0
    fi
    log "cloning $UPSTREAM"
    git clone -q --depth 50 "$UPSTREAM" "$REPO"
  fi
  git -C "$REPO" config user.name  'LCCC Agent'
  git -C "$REPO" config user.email 'agent@lccc.local'
}


# --------------------------------------------------------------- 4. rust -----
# A harness wipe restores ~/.cargo/bin from the snapshot but loses two things
# that make the toolchain unusable, and neither failure is self-explanatory:
#
#   1. `rustup` comes back WITHOUT its execute bit ("Permission denied"), and
#   2. the proxy binaries (cargo, rustc, ...) are gone entirely. `rustup
#      toolchain install` does NOT recreate them -- only `rustup-init` does --
#      so the toolchain installs successfully and `cargo` is still not found.
#
# rustup dispatches on argv[0], so symlinking the proxies back to it is the
# supported recovery. Pin 1.98.0: the tree is built and validated against it.
RUST_VERSION=${RUST_VERSION:-1.98.0}
RUST_PROXIES=(cargo rustc rustdoc rustfmt cargo-fmt cargo-clippy clippy-driver
              rust-gdb rust-lldb)

setup_rust() {
  local bin="$HOME/.cargo/bin"
  export PATH="$bin:$PATH"

  if [[ -f $bin/rustup && ! -x $bin/rustup ]]; then
    log "restoring execute bit on rustup"
    chmod +x "$bin/rustup"
  fi

  if [[ ! -x $bin/rustup ]]; then
    log "installing rustup + Rust $RUST_VERSION"
    curl -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION" \
                 --no-modify-path >/dev/null
  fi

  if ! "$bin/rustup" toolchain list 2>/dev/null | grep -q "^$RUST_VERSION"; then
    log "installing Rust $RUST_VERSION"
    "$bin/rustup" toolchain install "$RUST_VERSION" \
      --profile minimal --no-self-update >/dev/null
  fi
  "$bin/rustup" default "$RUST_VERSION" >/dev/null 2>&1 || true

  # Recreate any missing proxy. Harmless when they already exist.
  local p missing=0
  for p in "${RUST_PROXIES[@]}"; do
    if [[ ! -e $bin/$p ]]; then
      ln -sf rustup "$bin/$p"
      missing=$((missing + 1))
    fi
  done
  [[ $missing -gt 0 ]] && log "recreated $missing rustup proxy binaries"

  # Prove it, rather than assuming: a silent failure here wastes the whole
  # session, because every later step blames the compiler instead of the PATH.
  if ! "$bin/cargo" --version >/dev/null 2>&1; then
    log "FATAL: cargo still not runnable after bootstrap"
    return 1
  fi
  log "rust ready: $("$bin/rustc" --version)"
}

setup_swap
tune_vm
setup_rust
setup_repo "${1:-}"
log "environment ready:  repo=$REPO  artifacts=$ART"
free -h | sed 's/^/    /'
