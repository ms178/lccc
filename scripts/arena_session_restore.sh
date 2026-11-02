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
# case. Step 2 consumes the hash-locked agent toolchains from the user's cache;
# it does not install packages or download toolchains from external mirrors.
# Restore the pinned cache first when it is missing.
#
# Usage: scripts/arena_session_restore.sh [--with-kernel | --source-only | --swap-only]
# ============================================================================
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

case ${1:-} in
    ''|--with-kernel|--source-only|--swap-only) ;;
    *) printf 'unknown restore mode: %s\n' "$1" >&2; exit 2 ;;
esac

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
if [[ ${1:-} == --swap-only ]]; then
    if [[ $(awk 'NR>1 {n++} END {print n+0}' /proc/swaps) -eq 0 ]]; then
        log 'FATAL: no active swap (swapon may be prohibited by the host)'
        exit 1
    fi
    exit 0
fi

# ---- 2. hash-locked toolchains in the user workspace ----------------------
# Never install from rustup, package mirrors, or a privileged package manager.
# The reviewed GitHub-only cache is restored separately into a private prefix.
cache_env=${LCCC_AGENT_CACHE_PREFIX:-$HOME/.cache/lccc/agent-toolchains}/env.sh
if [[ ! -r $cache_env ]]; then
    log "FATAL: pinned toolchain cache missing: $cache_env"
    log 'Run: python3 scripts/restore_agent_toolchain_cache.py --tag harness-cache-7 --only rust,gcc,clang,binutils'
    exit 1
fi
# shellcheck disable=SC1090
source "$cache_env"
for tool in cargo rustc rustfmt cargo-clippy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        log "FATAL: $tool is missing from the restored cache"
        exit 1
    fi
done
log "rustc: $(rustc --version)"
log "cargo: $(cargo --version)"

# ---- 3. host 32-bit (-m32) support: observe through the safe script --------
# Installing Debian packages into /, especially on usrmerged systems, can
# replace the host dynamic loader and destroy the sandbox. The restore path
# therefore *never* provisions: it asks scripts/ensure_i386_multilib_safe.sh
# for a --check-only verdict (compile and run a 32-bit probe; no apt, no dpkg).
# provisioning, when wanted, stays behind that script's audited --no-upgrade
# plan, which refuses any apt transaction that would upgrade or remove a live
# libc and never adds a foreign :i386 architecture.
if [[ -x ./scripts/ensure_i386_multilib_safe.sh ]]; then
    if bash ./scripts/ensure_i386_multilib_safe.sh --check-only >/dev/null 2>&1; then
        log 'm32 C/C++ oracle: OK'
    else
        log 'm32 C/C++ oracle: MISSING (run scripts/ensure_i386_multilib_safe.sh; it provisions with --no-upgrade or stages an isolated sysroot)'
    fi
else
    log 'm32 C/C++ oracle: UNKNOWN (scripts/ensure_i386_multilib_safe.sh is missing)'
fi

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
    # The tree path is resolved, never spelled: prepare_kernel_tree.sh derives
    # the version from the package PKGBUILD, so a hardcoded directory here would
    # regenerate (or silently skip) the wrong tree after a version bump.  Both
    # this script and prepare_kernel_tree.sh call the same resolver.
    kernel_tree=$(bash ./scripts/kernel_version.sh --default-dir)
    if [[ ! -f $kernel_tree/.lccc-prepared ]] \
       || [[ ! -f $kernel_tree/arch/x86/boot/setup.ld ]]; then
        log "regenerating $(basename -- "$kernel_tree") tree"
        ./scripts/prepare_kernel_tree.sh "$kernel_tree" || log 'KERNEL TREE RESTORE FAILED'
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
