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
log "rustc: $(rustc --version 2>/dev/null || echo MISSING)"
log "cargo: $(cargo --version 2>/dev/null || echo MISSING)"

# ---- 3. host packages (kernel tree + -m32 oracle) ----------------------------
if ! gcc -m32 -x c -o /dev/null - <<< 'int main(){return 0;}' 2>/dev/null \
    || ! command -v zstd >/dev/null 2>&1 || ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
    log 'installing apt deps (gcc-multilib, kernel tooling)'
    sudo apt-get update -qq >/dev/null 2>&1
    # The list is what scripts/prepare_kernel_tree.sh preflights, plus the 32-bit
    # oracle: installing only part of it lets --with-kernel fail after the
    # tarball download, which is the expensive part (observed: zstd missing for
    # CONFIG_KERNEL_ZSTD=y on a restored sandbox).
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
        flex bison bc libelf-dev libssl-dev cpio gcc-multilib libc6-dev-i386 \
        zstd xz-utils lz4 lzop bzip2 kmod dwarves rsync qemu-system-x86 \
        >/dev/null 2>&1
fi
log "m32 oracle: $(gcc -m32 -x c -o /dev/null - <<< 'int main(){return 0;}' 2>/dev/null && echo OK || echo FAIL)"

# ---- 4. git: remote + identity + executable bits ------------------------------
# The harness snapshot excludes sensitive credential paths, and in practice it can
# drop the ENTIRE .git directory rather than just .git/config. Everything below
# (and every snapshot) then dies with "fatal: not a git repository", so recover
# the repository first.
#
# Recovery keeps the restored worktree byte-for-byte and rebuilds only the index,
# so uncommitted work reappears as ordinary modifications.
#
# First choice is the session's own artifacts/lccc.bundle: transplanting its
# .git preserves the work branch, the snapshot commits AND the base commit the
# snapshot script diffs against. (An older revision of this script avoided the
# bundle because `git fetch` from it can fail on thin packs; `git clone` from
# it does not -- verified 2026-09-14 after a full .git loss, where the bundle
# restored ms178-1-work with all commits and the base ref intact.)
#
# Fallback is a fresh upstream clone plus a MIXED reset (index rebuilt from
# HEAD, worktree untouched). NEVER `git checkout -- .` / `git reset --hard`
# here -- either one destroys the uncommitted work this script exists to
# protect. Note the fallback does NOT preserve the session branch or the
# snapshot base commit: the next snapshot must then be inspected by hand.
if [[ ! -d .git ]]; then
    log 'RECOVERY: .git is missing entirely (not just .git/config)'
    tmp_git="$(mktemp -d)/lccc"
    # Primary: the durable bundle the snapshot script publishes (path
    # agreement, audit F8).  Fallback: a bundle left in the bulk zone by an
    # older snapshot revision — better than nothing when it survived.
    if [[ ! -f /home/user/artifacts/lccc.bundle && -f /home/user/target/artifacts/lccc.bundle ]]; then
        log 'using bulk-zone bundle (legacy snapshot layout)'
        mkdir -p /home/user/artifacts
        cp /home/user/target/artifacts/lccc.bundle /home/user/artifacts/lccc.bundle
    fi
    if [[ -f /home/user/artifacts/lccc.bundle ]] \
        && git clone -q /home/user/artifacts/lccc.bundle "$tmp_git" 2>/dev/null; then
        mv "$tmp_git/.git" ./.git
        git remote set-url origin https://github.com/ms178/lccc.git 2>/dev/null || true
        log "recovered from bundle: branch=$(git branch --show-current 2>/dev/null || echo detached) HEAD=$(git rev-parse --short HEAD)"
        log "recovered: $(git status --porcelain | wc -l) worktree changes preserved as modifications"
    elif git clone --depth 200 -q https://github.com/ms178/lccc.git "$tmp_git"; then
        mv "$tmp_git/.git" ./.git
        # MIXED reset: rebuilds the index from HEAD and leaves the worktree
        # alone (see NEVER above).
        git reset -q
        log "recovered from upstream: HEAD=$(git rev-parse --short HEAD) ($(git rev-list --count HEAD ^origin/main 2>/dev/null || echo 0) local commits)"
        log "recovered: $(git status --porcelain | wc -l) worktree changes preserved as modifications"
    else
        log 'RECOVERY FAILED: bundle and upstream clone both failed; worktree is intact but git is unavailable'
    fi
    rm -rf "$(dirname "$tmp_git")" 2>/dev/null || true
fi
if ! git remote get-url origin >/dev/null 2>&1; then
    log 're-adding origin remote (snapshot drops .git/config)'
    git remote add origin https://github.com/ms178/lccc.git 2>/dev/null || true
fi
git config user.name  'LCCC Agent' 2>/dev/null || true
git config user.email 'agent@lccc.local' 2>/dev/null || true
# Restore worktree files the snapshot evicted (10k-file cap) WITHOUT touching
# modified files: only paths git reports as DELETED are checked out, so
# uncommitted content edits are never at risk (an absent file has no edits to
# lose -- observed 2026-09-14: 1334 tracked files missing after a restore,
# leaving the tree unbuildable until they were checked back out).
#
# A deliberately `rm`'d file looks identical to an evicted one, so the restore
# only fires in bulk: more than 50 deletions is never a deliberate uncommitted
# edit, it is snapshot eviction. At or below the threshold the deletions are
# listed for the agent to judge instead.
n_deleted=$(git diff --name-only --diff-filter=D -z 2>/dev/null | tr -cd '\0' | wc -c)
if [[ "$n_deleted" -gt 50 ]]; then
    n_restored=0
    while IFS= read -r -d '' f; do
        git checkout -q -- "$f" 2>/dev/null && n_restored=$((n_restored+1))
    done < <(git diff --name-only --diff-filter=D -z)
    log "evicted files restored: $n_restored (of $n_deleted deleted)"
else
    log "deleted-but-tracked files left alone: $n_deleted (below bulk threshold)"
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
    if [[ ! -f /home/user/target/kernel-work/linux-6.18.52/.lccc-prepared ]] \
       || [[ ! -f /home/user/target/kernel-work/linux-6.18.52/arch/x86/boot/setup.ld ]]; then
        log 'regenerating linux-cachymod-6.18.52 tree'
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
