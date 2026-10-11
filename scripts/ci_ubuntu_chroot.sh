#!/usr/bin/env bash
# Run a command inside the userland GitHub's `ubuntu-latest` runner uses.
#
# WHY THIS EXISTS
# ---------------
# ci_local.sh mirrors the GATES of .github/workflows/ci.yml, but on the
# developer's host userland.  Many gates hand lccc output to the SYSTEM
# gcc / as / ld / objdump / glibc, and those differ between hosts: PR #638
# (S24) was green under ci_local.sh on Debian 13 (gcc 14, binutils 2.44,
# glibc 2.41) and red on the Ubuntu 24.04 runner (gcc 13, binutils 2.42,
# glibc 2.39).  A gate mirror without an environment mirror is half a mirror.
#
# This script builds (once, idempotently) a minimal Ubuntu chroot carrying the
# packages ci.yml installs on top of the runner image, bind-mounts the calling
# user's HOME at the SAME path (so the repo, rustup/cargo, ~/.cache and any
# pinned tool such as GNU as 2.47 resolve exactly as on the host), and runs
# the given command in the repo as the calling user.  It needs root (sudo)
# for debootstrap, mount and chroot.
#
# Usage:
#   scripts/ci_ubuntu_chroot.sh [--suite noble] -- CMD [ARGS...]
#   scripts/ci_ubuntu_chroot.sh --setup-only
#   scripts/ci_ubuntu_chroot.sh -- bash scripts/ci_local.sh --only i686
#   scripts/ci_ubuntu_chroot.sh -- bash scripts/ci_local.sh    # the full mirror
#
# target/ is shared with the host: binaries built inside link against the
# chroot's older glibc and run on the host too, but cargo's fingerprints see a
# different C toolchain, so switching between host and chroot builds rebuilds.
#
# Environment:
#   LCCC_CI_CHROOT   chroot directory (default /srv/lccc-ci-<suite>; keep it
#                    OUTSIDE the repo so it never lands in a snapshot)
#   LCCC_CI_MIRROR   Ubuntu archive (default http://archive.ubuntu.com/ubuntu)
#
# The runner migrates `ubuntu-latest` to Ubuntu 26.04 on 2026-10-19; pass
# `--suite` accordingly (and keep noble around for the transition window).
set -euo pipefail

SUITE=noble
SETUP_ONLY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --suite) SUITE=$2; shift 2 ;;
        --setup-only) SETUP_ONLY=1; shift ;;
        --) shift; break ;;
        *) echo "usage: $0 [--suite S] [--setup-only] -- CMD..." >&2; exit 2 ;;
    esac
done
if [[ $SETUP_ONLY == 0 && $# -eq 0 ]]; then
    echo "usage: $0 [--suite S] [--setup-only] -- CMD..." >&2
    exit 2
fi

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ROOT=${LCCC_CI_CHROOT:-/srv/lccc-ci-$SUITE}
MIRROR=${LCCC_CI_MIRROR:-http://archive.ubuntu.com/ubuntu}
case $REPO in
    "$HOME"/*) ;;
    *) echo "the repo ($REPO) must live under \$HOME ($HOME), which is what is bind-mounted" >&2
       exit 2 ;;
esac
# What the runner image provides that the gates use (compiler, binutils, make,
# python3, git, file), plus what ci.yml's "Install i686 C runtime" step adds.
PACKAGES=build-essential,gcc-multilib,libc6-dev-i386,python3,python3-yaml,git,file,binutils,make,bc,xz-utils,ca-certificates,perl,curl
# The runner image also carries GCC 14 with its C++ runtime development files:
# the lccc driver picks the newest /usr/lib/gcc/<triple>/<ver> and links -lstdc++
# from it, so gcc-14 without libstdc++-14-dev fails the C++ linker tests.
PACKAGES=$PACKAGES,gcc-14,g++-14,libstdc++-14-dev

stamp=$ROOT/.lccc-ci-ready
if [[ ! -e $stamp ]]; then
    # /usr/sbin is on sudo's secure_path but often not on a user PATH.
    command -v debootstrap >/dev/null || [[ -x /usr/sbin/debootstrap ]] ||
        { echo "debootstrap missing: sudo apt-get install debootstrap ubuntu-keyring" >&2; exit 1; }
    echo "[ci-chroot] bootstrapping Ubuntu $SUITE into $ROOT (one-time, ~1 GB)"
    # debootstrap does not retry a failed download, and one transient miss
    # among ~300 packages aborts it (seen: libsemanage-common); a partial
    # root cannot be resumed reliably, so each attempt starts clean.  No
    # mount exists yet (the binds are made below, after the stamp).
    ok=0
    for attempt in 1 2 3; do
        sudo rm -rf --one-file-system "$ROOT"
        sudo mkdir -p "$ROOT"
        if sudo debootstrap --variant=minbase --components=main,universe \
            --include="$PACKAGES" "$SUITE" "$ROOT" "$MIRROR"; then
            ok=1
            break
        fi
        echo "[ci-chroot] debootstrap attempt $attempt failed; retrying" >&2
        sleep $((attempt * 10))
    done
    [[ $ok == 1 ]] || { echo "[ci-chroot] debootstrap failed 3 times" >&2; exit 1; }
    # The calling user, with the same uid/gid, so files written into the
    # bind-mounted repo keep their ownership.
    uid=$(id -u) gid=$(id -g) name=$(id -un)
    sudo chroot "$ROOT" sh -c "getent group $gid >/dev/null || groupadd -g $gid $name;
        getent passwd $uid >/dev/null || useradd -m -u $uid -g $gid -s /bin/bash $name"
    sudo touch "$stamp"
fi
# A chroot bootstrapped before a package was added to PACKAGES keeps the stamp,
# so any package it lacks is installed here instead of being silently absent.
missing=()
for pkg in ${PACKAGES//,/ }; do
    sudo chroot "$ROOT" dpkg -s "$pkg" >/dev/null 2>&1 || missing+=("$pkg")
done
if ((${#missing[@]})); then
    echo "[ci-chroot] installing into the existing chroot: ${missing[*]}"
    sudo chroot "$ROOT" sh -c "DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends ${missing[*]} >/dev/null"
fi
[[ $SETUP_ONLY == 1 ]] && { echo "[ci-chroot] ready: $ROOT"; exit 0; }

# Bind mounts are torn down on every exit path; a leaked /proc mount would
# keep the chroot busy.
mounts=()
cleanup() {
    local i
    for ((i = ${#mounts[@]} - 1; i >= 0; i--)); do
        sudo umount -l "${mounts[$i]}" 2>/dev/null || true
    done
}
trap cleanup EXIT
bind() { # bind SRC DST-inside-root
    sudo mkdir -p "$ROOT$2"
    sudo mount --bind "$1" "$ROOT$2"
    mounts+=("$ROOT$2")
}
bind /proc /proc
bind /dev /dev
bind /dev/pts /dev/pts
bind /sys /sys
bind /dev/shm /dev/shm
bind "$HOME" "$HOME"
# Symlinks directly under $HOME may point outside it: a workstation can keep the
# repository, cargo and rustup in a bulk store (`~/lccc -> /var/tmp/...`). The
# link resolves on the host but dangles inside the chroot, so every such target
# is bound at its own path. Bound paths are unmounted by the cleanup trap.
while IFS= read -r -d '' link; do
    target=$(readlink -f -- "$link")
    case $target in
        "$HOME" | "$HOME"/*) ;;
        *) bind "$target" "$target" ;;
    esac
done < <(find "$HOME" -mindepth 1 -maxdepth 1 -type l -print0)
sudo mkdir -p "$ROOT/tmp" && sudo chmod 1777 "$ROOT/tmp"

cmd=$(printf '%q ' "$@")
sudo chroot --userspec="$(id -u):$(id -g)" "$ROOT" /usr/bin/env -i \
    HOME="$HOME" USER="$(id -un)" LANG=C.UTF-8 TERM=dumb \
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    PATH="${CARGO_HOME:-$HOME/.cargo}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    CI=true GITHUB_ACTIONS=true \
    bash -c "cd $(printf '%q' "$REPO") && $cmd"
