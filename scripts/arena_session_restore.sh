#!/usr/bin/env bash
# ============================================================================
# arena_session_restore.sh — bring a fresh Arena sandbox back to working
# state after the between-turn workspace restore.
#
# The harness snapshot restores /home/user contents but: drops /opt (rustup),
# drops /swapfile, drops /tmp, drops target/ (excluded dir name), strips +x
# from worktree files, truncates the ~55k-file kernel tree (10k-file cap),
# and removes .git/config (credential-path exclusion).  This script restores
# every piece idempotently so a new session is one command from productive
# work.
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

# ---- 2. rust toolchain to /opt (kept out of the snapshot) --------------------
export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
if [[ ! -x /opt/cargo/bin/rustc ]]; then
    log 'installing rust (minimal profile) to /opt'
    sudo mkdir -p /opt/rustup /opt/cargo
    sudo chown -R "$(id -u):$(id -g)" /opt/rustup /opt/cargo
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null 2>&1
fi
log "rustc: $(/opt/cargo/bin/rustc --version 2>/dev/null || echo MISSING)"

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
    if [[ ! -f /home/user/kernel-work/linux-6.18.44/.lccc-prepared ]] \
       || [[ ! -f /home/user/kernel-work/linux-6.18.44/arch/x86/boot/setup.ld ]]; then
        log 'regenerating linux-cachymod-6.18.44 tree'
        ./scripts/prepare_kernel_tree.sh || log 'KERNEL TREE RESTORE FAILED'
    else
        log 'kernel tree present'
    fi
fi

# ---- 6. compiler binaries -----------------------------------------------------
if [[ ! -x target/fastbuild/lccc ]]; then
    log 'building lccc (fastbuild)'
    export PATH=/opt/cargo/bin:$PATH
    ./scripts/build_lccc_fast.sh >/dev/null 2>&1 \
        && log "lccc built: $(./target/fastbuild/lccc --version 2>/dev/null | head -1)" \
        || log 'BUILD FAILED — run scripts/build_lccc_fast.sh manually'
else
    log "lccc present: $(./target/fastbuild/lccc --version 2>/dev/null | head -1)"
fi

log 'done.'
