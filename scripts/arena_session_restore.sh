#!/usr/bin/env bash
# ============================================================================
# arena_session_restore.sh — bring a fresh Arena sandbox back to working
# state after the between-turn workspace restore.
#
# The harness snapshot restores /home/user contents but: drops /opt (rustup),
# drops /swapfile, drops /tmp, drops target/ (excluded dir name), strips +x
# from worktree files, truncates the ~55k-file kernel tree (10k-file cap),
# and removes .git/config (credential-path exclusion). The persisted
# /home/user/.cargo and /home/user/.rustup trees are the canonical toolchain
# location. This script restores every piece idempotently so a new session is
# one command from productive work.
#
# Usage: scripts/arena_session_restore.sh [--with-kernel]
# ============================================================================
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

log() { printf '[restore] %s\n' "$*"; }

# ---- 1. swap (compiler/linker peaks exceed the 1.9 GB RAM) ------------------
if ! grep -q '^/swapfile' /proc/swaps 2>/dev/null; then
    log 'creating 8G /swapfile'
    sudo fallocate -l 8G /swapfile 2>/dev/null \
        || sudo dd if=/dev/zero of=/swapfile bs=1M count=8192 status=none
    sudo chmod 600 /swapfile
    sudo mkswap /swapfile >/dev/null
    sudo swapon /swapfile 2>/dev/null || true
fi
log "swap: $(awk '/SwapTotal/{print $2"kB"}' /proc/meminfo)"

# ---- 2. exact Rust/Cargo toolchain in the persisted workspace --------------
# Do not reinstall under /opt: the harness wipes it and the old restore path
# silently downgraded the next session to an image-provided Cargo.  The
# repository's rust-toolchain.toml and this explicit channel must agree.
export RUSTUP_HOME=${RUSTUP_HOME:-/home/user/.rustup}
export CARGO_HOME=${CARGO_HOME:-/home/user/.cargo}
export RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-1.98.0}
export PATH="$CARGO_HOME/bin:$PATH"
if [[ ! -x "$CARGO_HOME/bin/rustup" ]]; then
    log 'installing rustup into persisted /home/user/.cargo'
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
        sh -s -- -y --profile minimal --default-toolchain none --no-modify-path \
        >/dev/null 2>&1
fi
if ! "$CARGO_HOME/bin/rustup" toolchain list 2>/dev/null | grep -q "^${RUSTUP_TOOLCHAIN}"; then
    log "installing Rust/Cargo $RUSTUP_TOOLCHAIN"
    "$CARGO_HOME/bin/rustup" toolchain install "$RUSTUP_TOOLCHAIN" \
        --profile minimal --no-self-update >/dev/null
fi
"$CARGO_HOME/bin/rustup" default "$RUSTUP_TOOLCHAIN" >/dev/null 2>&1 || true
log "rustc: $(rustc --version 2>/dev/null || echo MISSING)"
log "cargo: $(cargo --version 2>/dev/null || echo MISSING)"

# ---- 3. host packages (kernel tree + -m32 oracle) ----------------------------
if ! gcc -m32 -x c -o /dev/null - <<< 'int main(){return 0;}' 2>/dev/null; then
    log 'installing apt deps (gcc-multilib, kernel tooling)'
    sudo apt-get update -qq >/dev/null 2>&1
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
        flex bison bc libelf-dev libssl-dev cpio gcc-multilib libc6-dev-i386 \
        >/dev/null 2>&1
fi
log "m32 oracle: $(gcc -m32 -x c -o /dev/null - <<< 'int main(){return 0;}' 2>/dev/null && echo OK || echo FAIL)"

# ---- 4. git: remote + identity + executable bits ------------------------------
if ! git remote get-url origin >/dev/null 2>&1; then
    log 're-adding origin remote (snapshot drops .git/config)'
    git remote add origin https://github.com/ms178/lccc.git
fi
git config user.name  'LCCC Agent' 2>/dev/null || true
git config user.email 'agent@lccc.local' 2>/dev/null || true
# Restore +x on tracked files recorded as executable.  NEVER `git checkout -- .`:
# that would discard uncommitted content edits.
n_modes=0
while IFS= read -r f; do
    chmod +x "$f" 2>/dev/null && n_modes=$((n_modes+1))
done < <(git ls-files -s | awk '$1 == "100755" {print $4}')
log "exec bits restored: $n_modes"

# ---- 5. optional: kernel tree regeneration -----------------------------------
if [[ ${1:-} == --with-kernel ]]; then
    if [[ ! -f /home/user/kernel-work/linux-6.18.46/.lccc-prepared ]] \
       || [[ ! -f /home/user/kernel-work/linux-6.18.46/arch/x86/boot/setup.ld ]]; then
        log 'regenerating linux-cachymod-6.18.46 tree'
        ./scripts/prepare_kernel_tree.sh || log 'KERNEL TREE RESTORE FAILED'
    else
        log 'kernel tree present'
    fi
fi

# ---- 6. compiler binaries -----------------------------------------------------
if [[ ! -x target/fastbuild/lccc ]]; then
    log 'building lccc (fastbuild)'
    export PATH="$CARGO_HOME/bin:$PATH"
    ./scripts/build_lccc_fast.sh >/dev/null 2>&1 \
        && log "lccc built: $(./target/fastbuild/lccc --version 2>/dev/null | head -1)" \
        || log 'BUILD FAILED — run scripts/build_lccc_fast.sh manually'
else
    log "lccc present: $(./target/fastbuild/lccc --version 2>/dev/null | head -1)"
fi

log 'done.'
