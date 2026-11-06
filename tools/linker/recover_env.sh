#!/usr/bin/env bash
# Idempotent environment recovery for the LCCC workspace.
#
# The sandbox wipes ~/.rustup, /home/user/lccc/.git, target/, script exec bits
# and the swap file between turns, while /home/user files (sources, bundle,
# artifacts) persist.  This script restores everything needed to build and
# validate, and is safe to re-run.
set -uo pipefail

REPO=/home/user/lccc
BUNDLE=/home/user/artifacts/lccc.bundle
export PATH=/home/user/.cargo/bin:$PATH

step() { printf '\n=== %s ===\n' "$*"; }

step "1/6 32-bit toolchain (i686 compile/link/run, zero live-root writes)"
# This step used to run `sudo dpkg --add-architecture i386` followed by an
# UNAUDITED `apt-get install libc6-dev-i386 gcc-multilib lib32gcc-14-dev` with
# all output discarded.  On a usr-merged sandbox that class of command has
# destroyed /lib -> usr/lib and killed every shell in the session, repeatedly,
# across agent sessions -- and this recovery script is exactly what an agent
# runs first after a wipe.  It now goes through the audited, offline-first,
# user-namespace-jailed provisioner, which provides the SAME coverage (real
# 32-bit compile, link and execute, C and C++) without touching the live root.
if bash "$REPO/scripts/ensure_i386_multilib_safe.sh"; then
    echo "32-bit coverage ready (host multilib or isolated sysroot)"
else
    echo "WARNING: 32-bit coverage unavailable; i686 gates must be reported as"
    echo "         NOT RUN (never as passed).  See docs/I386-MULTILIB-SAFETY.md."
fi

step "2/6 rust toolchain"
if ! command -v cargo >/dev/null; then
    sh /home/user/rustup-init.sh -y --default-toolchain stable --profile minimal \
        -c rustfmt -c clippy >/home/user/rustup-reinstall.log 2>&1
fi
cargo --version || { echo "FAILED: cargo"; exit 1; }

step "3/6 git repository"
if [ ! -d "$REPO/.git" ]; then
    ( cd "$REPO" && git init -q && git remote add origin https://github.com/ms178/lccc.git 2>/dev/null )
    ( cd "$REPO" && git fetch -q "$BUNDLE" '+refs/heads/*:refs/heads/*' \
        '+refs/remotes/origin/*:refs/remotes/origin/*' )
    echo "restored from bundle"
else
    echo ".git present"
fi
( cd "$REPO" && git log --oneline -1 2>/dev/null || echo "no commits yet" )

step "4/6 swap file (4 GiB, audited helper)"
# Was: an open-coded 8 GiB /swapfile created through sudo.  The standing
# requirement is 4 GiB, and scripts/ci_enable_swap.sh already implements the
# idempotent, verified, non-destructive version.
bash "$REPO/scripts/ci_enable_swap.sh" || echo "(swap unavailable in this sandbox)"

step "5/6 script exec bits"
chmod +x "$REPO"/scripts/*.sh "$REPO"/scripts/*.py 2>/dev/null
echo "done"

step "6/6 upstream sync"
( cd "$REPO" && timeout 600 git fetch -q origin main 2>&1 | tail -2; \
  git log --oneline origin/main -1 2>/dev/null || echo "(fetch failed - offline?)" )

echo
echo "RECOVERY COMPLETE"
