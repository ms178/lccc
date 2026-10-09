#!/usr/bin/env bash
# ============================================================================
# arena_session_restore.sh — bring a fresh Arena sandbox back to working
# state after the between-turn workspace restore.
#
# The harness snapshot restores /home/user contents but: drops /opt (rustup),
# drops /swapfile, drops /tmp, drops target/ (excluded dir name), strips +x
# from worktree files, truncates the ~55k-file kernel tree (10k-file cap),
# and can remove .git ENTIRELY -- not just .git/config as the credential-path
# exclusion implies (observed 2026-09-07: no .git at all, so every git
# invocation died with "fatal: not a git repository"). Step 4 recovers that
# case. The persisted /home/user/.cargo and /home/user/.rustup trees are the
# canonical toolchain location, but they can come back incomplete, so step 2
# re-resolves the channel rather than trusting them. This script restores every
# piece idempotently so a new session is one command from productive work.
#
# Usage: scripts/arena_session_restore.sh [--with-kernel | --source-only]
# ============================================================================
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

log() { printf '[restore] %s\n' "$*"; }

if [[ ${1:-} == --source-only ]]; then
    if [[ ! -d .git ]]; then
        recover_args=(--repo "$repo_root" --artifacts "${LCCC_ARTIFACTS:-/home/user/artifacts}" --preserve-worktree)
        [[ ${LCCC_RECOVERY_OFFLINE:-0} == 1 ]] && recover_args+=(--offline)
        python3 "$repo_root/scripts/lccc_recover.py" "${recover_args[@]}" || exit 1
    fi
    log 'source-only recovery; no swap/toolchain/packages/compiler queries/builds'
    exit 0
fi


# ---- 1. swap (compiler/linker peaks exceed the 1.9 GB RAM) ------------------
if [[ $(awk 'NR>1 {n++} END {print n+0}' /proc/swaps) -eq 0 ]]; then
    log 'creating 8G /swapfile'
    sudo fallocate -l 8G /swapfile 2>/dev/null \
        || sudo dd if=/dev/zero of=/swapfile bs=1M count=8192 status=none
    sudo chmod 600 /swapfile
    sudo mkswap /swapfile >/dev/null
    sudo swapon /swapfile 2>/dev/null || true
fi
log "swap: $(awk '/SwapTotal/{print $2"kB"}' /proc/meminfo)"

# ---- 2. current Rust/Cargo toolchain in the persisted workspace ------------
# Do not reinstall under /opt: the harness wipes it and the old restore path
# silently downgraded the next session to an image-provided Cargo.  Resolve the
# channel from rust-toolchain.toml so a source upgrade changes every script's
# toolchain consistently.
export RUSTUP_HOME=${RUSTUP_HOME:-/home/user/.rustup}
export CARGO_HOME=${CARGO_HOME:-/home/user/.cargo}
export PATH="$CARGO_HOME/bin:$PATH"
# shellcheck source=rust_toolchain.sh
source "$repo_root/scripts/rust_toolchain.sh"
lccc_select_rust_toolchain "$repo_root"
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
# The minimal profile deliberately omits these, but the repository's strict
# formatting and Clippy gates require both after every harness restore.
if ! "$CARGO_HOME/bin/rustup" component add --toolchain "$RUSTUP_TOOLCHAIN" rustfmt clippy >/dev/null; then
    log "FATAL: unable to install rustfmt/clippy for $RUSTUP_TOOLCHAIN"
    exit 1
fi
# The snapshot persists /home/user/.cargo but can drop all but one of the
# hardlinked rustup shims (observed: only `rustup` survived, so cargo and
# rustc were MISSING despite an installed toolchain, and the build failed
# 10 minutes later with no hint of the cause). Recreate any missing shim
# as a hardlink to the multicall binary, then fail fast if cargo is still
# unusable rather than limping into a doomed build.
for shim in cargo rustc rustdoc rustfmt cargo-fmt cargo-clippy clippy-driver; do
    if [[ ! -x $CARGO_HOME/bin/$shim ]]; then
        if ln -f "$CARGO_HOME/bin/rustup" "$CARGO_HOME/bin/$shim" 2>/dev/null; then
            log "recreated missing $shim shim"
        else
            log "FATAL: unable to recreate $shim shim"
            exit 1
        fi
    fi
done
if ! cargo --version >/dev/null 2>&1; then
    log 'FATAL: cargo unusable after toolchain install'
    exit 1
fi
log "rustc: $(rustc --version 2>/dev/null || echo MISSING)"
log "cargo: $(cargo --version 2>/dev/null || echo MISSING)"

# ---- 3. host packages (kernel tree + -m32 oracle) ----------------------------
if ! gcc -m32 -x c -o /dev/null - <<< 'int main(){return 0;}' 2>/dev/null \
    || ! printf '#include <stdexcept>\nint main(){return 0;}\n' | g++ -m32 -x c++ -o /dev/null - 2>/dev/null \
    || ! command -v zstd >/dev/null 2>&1 || ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
    log 'installing apt deps (gcc/g++-multilib, kernel tooling)'
    sudo apt-get update -qq >/dev/null 2>&1
    # The list is what scripts/prepare_kernel_tree.sh preflights, plus the 32-bit
    # C and C++ oracles used by compiler/linker suites: installing only gcc-multilib
    # lets i386 C++ DSO tests fail later with a missing bits/c++config.h.
    # Installing only part of the kernel tooling also lets --with-kernel fail
    # after the tarball download (observed: zstd missing for CONFIG_KERNEL_ZSTD=y).
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
        flex bison bc libelf-dev libssl-dev cpio gcc-multilib g++-multilib libc6-dev-i386 \
        zstd xz-utils lz4 lzop bzip2 kmod dwarves rsync qemu-system-x86 \
        >/dev/null 2>&1
fi
log "m32 C oracle: $(gcc -m32 -x c -o /dev/null - <<< 'int main(){return 0;}' 2>/dev/null && echo OK || echo FAIL)"
log "m32 C++ oracle: $(printf '#include <stdexcept>\nint main(){return 0;}\n' | g++ -m32 -x c++ -o /dev/null - 2>/dev/null && echo OK || echo FAIL)"

# ---- 4. git: remote + identity + executable bits ------------------------------
# The harness snapshot excludes sensitive credential paths, and in practice it can
# drop the ENTIRE .git directory rather than just .git/config. Everything below
# (and every snapshot) then dies with "fatal: not a git repository", so recover
# the repository first.
#
# Recovery keeps the restored worktree byte-for-byte and rebuilds only the index,
# so uncommitted work reappears as ordinary modifications.
#
# Recover through the compact-bundle/source-recipe consumer shared with
# bootstrap. --preserve-worktree installs .git only, never resets source bytes.
if [[ ! -d .git ]]; then
    recover_args=(--repo "$repo_root" --artifacts "${LCCC_ARTIFACTS:-/home/user/artifacts}" --preserve-worktree)
    [[ ${LCCC_RECOVERY_OFFLINE:-0} == 1 ]] && recover_args+=(--offline)
    python3 "$repo_root/scripts/lccc_recover.py" "${recover_args[@]}" || exit 1
fi
if ! git remote get-url origin >/dev/null 2>&1; then
    log 're-adding origin remote (snapshot drops .git/config)'
    git remote add origin https://github.com/ms178/lccc.git 2>/dev/null || true
fi
git config user.name  'LCCC Agent' 2>/dev/null || true
git config user.email 'agent@lccc.local' 2>/dev/null || true
# Missing paths are not guessed to be eviction from a deletion-count threshold.
# Deliberate deletions and dropped files are indistinguishable; require opt-in.
if [[ ${LCCC_RESTORE_MISSING:-0} == 1 ]]; then
    while IFS= read -r -d '' f; do git checkout -q -- "$f" || exit 1; done < <(git diff --name-only --diff-filter=D -z)
else
    log 'missing tracked files left unchanged (LCCC_RESTORE_MISSING=1 opts in)'
fi
# Restore +x on tracked files recorded as executable.  NEVER `git checkout -- .`:
# that would discard uncommitted content edits.
n_modes=0
while IFS= read -r f; do
    chmod +x "$f" 2>/dev/null && n_modes=$((n_modes+1))
done < <(git ls-files -s | awk '$1 == "100755" {print $4}')
log "exec bits restored: $n_modes"

# ---- 5. optional: kernel tree regeneration -----------------------------------
if [[ ${1:-} == --with-kernel ]]; then
    if [[ ! -f /home/user/target/kernel-work/linux-6.18.55/.lccc-prepared ]] \
       || [[ ! -f /home/user/target/kernel-work/linux-6.18.55/arch/x86/boot/setup.ld ]]; then
        log 'regenerating linux-cachymod-6.18.55 tree'
        ./scripts/prepare_kernel_tree.sh || log 'KERNEL TREE RESTORE FAILED'
    else
        log 'kernel tree present'
    fi
fi

# ---- 6. compiler binaries -----------------------------------------------------
if [[ ${LCCC_ALLOW_COMPILER_BUILD:-0} != 1 ]]; then
    log 'compiler build skipped; explicit LCCC_ALLOW_COMPILER_BUILD=1 required'
elif [[ ! -x target/fastbuild/lccc ]]; then
    log 'building lccc (fastbuild)'
    export PATH="$CARGO_HOME/bin:$PATH"
    ./scripts/build_lccc_fast.sh >/dev/null 2>&1 \
        && log "lccc built: $(./target/fastbuild/lccc --version 2>/dev/null | head -1)" \
        || log 'BUILD FAILED — run scripts/build_lccc_fast.sh manually'
else
    log "lccc present: $(./target/fastbuild/lccc --version 2>/dev/null | head -1)"
fi

log 'done.'
