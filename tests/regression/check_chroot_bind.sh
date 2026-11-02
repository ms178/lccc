#!/usr/bin/env bash
# Regression for scripts/ci_ubuntu_chroot.sh `bind()` and `bind_home_links()`:
#  - a bind must create a FILE target for a file source and a DIRECTORY target
#    for a directory source. The old helper always created a directory, so
#    binding a home symlink that points at a file failed with a file-type
#    mismatch.
#  - every external home-link target is bound once, and a dangling link is
#    skipped. mount --bind of a missing source fails, and set -e would stop the
#    whole setup.
#  - package installation into an existing chroot refreshes the APT indexes
#    first.
#
# The functions are extracted from the script and run in this shell (a subshell
# would hide `mounts`). `sudo` is stubbed, so nothing is mounted: the stub
# records the mount calls and runs everything else unprivileged inside a
# temporary root.
set -euo pipefail
script=scripts/ci_ubuntu_chroot.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-chroot-bind.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail=0

# The index refresh must come before the install in the existing-chroot path.
if ! grep -q 'apt-get update -qq' "$script" \
    || ! grep -q 'apt-get update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install' "$script"; then
    echo "FAIL: existing-chroot package install does not refresh APT indexes first" >&2
    fail=1
fi

# Extract bind() exactly as written in the script.
fn=$(sed -n '/^bind() {/,/^}/p' "$script")
if [ -z "$fn" ]; then
    echo "FAIL: bind() not found in $script" >&2
    exit 1
fi

sudo() {  # stub: record mounts, run everything else without privilege
    if [ "$1" = mount ]; then
        printf '%s\n' "mount $*" >>"$tmp/mounts.log"
        return 0
    fi
    "$@"
}
ROOT="$tmp/root"
mkdir -p "$ROOT"
mounts=()
eval "$fn"

src_dir="$tmp/src-dir"
src_file="$tmp/src-file"
mkdir -p "$src_dir"
printf 'x\n' >"$src_file"

bind "$src_dir" /d1
bind "$src_file" /f1

if [ -d "$ROOT/d1" ]; then echo "PASS: directory source -> directory target"; else echo "FAIL: directory target missing" >&2; fail=1; fi
if [ -f "$ROOT/f1" ]; then echo "PASS: file source -> file target"; else echo "FAIL: file source did not get a file target" >&2; fail=1; fi
if [ -d "$ROOT/f1" ]; then echo "FAIL: file source got a directory target" >&2; fail=1; fi
if [ "${#mounts[@]}" -ne 2 ] || [ "${mounts[0]}" != "$ROOT/d1" ] || [ "${mounts[1]}" != "$ROOT/f1" ]; then
    echo "FAIL: mounts[] does not record both targets for cleanup: ${mounts[*]:-}" >&2
    fail=1
else
    echo "PASS: both targets recorded for the cleanup trap"
fi
if [ "$(wc -l <"$tmp/mounts.log")" -ne 2 ]; then
    echo "FAIL: expected two mount calls" >&2
    fail=1
fi

# bind() of a target that is already bound must not mount it a second time.
bind "$src_file" /f1
if [ "$(wc -l <"$tmp/mounts.log")" -ne 2 ] || [ "${#mounts[@]}" -ne 2 ]; then
    echo "FAIL: a repeated bind of the same target mounted it again" >&2
    fail=1
else
    echo "PASS: a repeated bind of the same target is a no-op"
fi

# Home-level symlinks: each external target is bound once, and a dangling link
# is skipped. Two links point at the same file; one points at a directory; one
# dangles.
fn_links=$(sed -n '/^bind_home_links() {/,/^}/p' "$script")
if [ -z "$fn_links" ]; then
    echo "FAIL: bind_home_links() not found in $script" >&2
    fail=1
else
    home="$tmp/home"
    outside="$tmp/outside"
    mkdir -p "$home" "$outside/d"
    printf 'y\n' >"$outside/f"
    ln -s "$outside/f" "$home/link_file_a"
    ln -s "$outside/f" "$home/link_file_b"
    ln -s "$outside/d" "$home/link_dir"
    ln -s "$outside/missing" "$home/link_dangling"
    ROOT="$tmp/root2"
    mkdir -p "$ROOT"
    : >"$tmp/mounts.log"
    mounts=()
    HOME="$home"
    eval "$fn_links"
    bind_home_links
    if [ "${#mounts[@]}" -ne 2 ]; then
        echo "FAIL: home links recorded ${#mounts[@]} mounts, expected 2 (file once, directory once): ${mounts[*]:-}" >&2
        fail=1
    else
        echo "PASS: each external home-link target is bound once"
    fi
    if [ "$(wc -l <"$tmp/mounts.log")" -ne 2 ]; then
        echo "FAIL: expected two mount calls for home links, got $(wc -l <"$tmp/mounts.log")" >&2
        fail=1
    fi
    if grep -q missing "$tmp/mounts.log" || [ -e "$ROOT$outside/missing" ]; then
        echo "FAIL: dangling home link reached mount --bind" >&2
        fail=1
    else
        echo "PASS: dangling home link is skipped"
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "check_chroot_bind: FAILED" >&2
    exit 1
fi
echo "check_chroot_bind: PASSED"
