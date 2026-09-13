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

step "1/6 32-bit toolchain (i686 native execution)"
if gcc -m32 -x c -o /tmp/.m32probe - <<'EOF' 2>/dev/null
int main(void){return 0;}
EOF
then
    echo "gcc -m32 works"
else
    sudo dpkg --add-architecture i386 >/dev/null 2>&1
    sudo apt-get update -qq >/dev/null 2>&1
    sudo apt-get install -y -qq libc6-dev-i386 gcc-multilib lib32gcc-14-dev >/dev/null 2>&1
    gcc -m32 -x c -o /tmp/.m32probe - <<'EOF' 2>/dev/null && echo "gcc -m32 now works" || echo "FAILED: gcc -m32"
int main(void){return 0;}
EOF
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

step "4/6 swap file"
if [ -e /swapfile ]; then
    sudo swapon /swapfile 2>/dev/null
fi
if ! sudo swapon --show 2>/dev/null | grep -q swap; then
    sudo fallocate -l 8G /swapfile 2>/dev/null || sudo dd if=/dev/zero of=/swapfile bs=1M count=8192 status=none
    sudo chmod 600 /swapfile
    sudo mkswap /swapfile >/dev/null 2>&1
    sudo swapon /swapfile 2>/dev/null
fi
sudo swapon --show 2>/dev/null || echo "(swapon unavailable)"

step "5/6 script exec bits"
chmod +x "$REPO"/scripts/*.sh "$REPO"/scripts/*.py 2>/dev/null
echo "done"

step "6/6 upstream sync"
( cd "$REPO" && timeout 600 git fetch -q origin main 2>&1 | tail -2; \
  git log --oneline origin/main -1 2>/dev/null || echo "(fetch failed - offline?)" )

echo
echo "RECOVERY COMPLETE"
