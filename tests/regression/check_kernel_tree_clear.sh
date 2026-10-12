#!/usr/bin/env bash
# Regression for scripts/prepare_kernel_tree.sh deletion safety. The script must
# never remove user data when it is pointed at a directory that is not a kernel
# tree:
#
#   - KDIR must not be an existing non-kernel directory (a `linux-*` directory
#     holding user files is refused, and the files survive);
#   - KDIR must not be a symlink (the link target survives);
#   - KDIR must not be $HOME, even when its basename starts with `linux`;
#   - PKG_ROOT must not be cleared while a mount point lies below it
#     (`mounts_under`).
#
# Every case stops at the KDIR guard, before any download. The PKGBUILD is a
# minimal local stub, so the test needs no network and no kernel tarball.
set -euo pipefail
script=scripts/prepare_kernel_tree.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-kdir-guard.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail=0

pkg="$tmp/pkgroot/packages/linux-cachymod-6.18"
mkdir -p "$pkg"
cat >"$pkg/PKGBUILD" <<'PKGBUILD'
_major=6
_minor=18
pkgrel=1
sha256sums=('0000000000000000000000000000000000000000000000000000000000000000')
source=()
PKGBUILD

run_guard() {  # run_guard <KDIR> -> prints output, sets rc
    set +e
    out=$(HOME="$HOME_FOR_TEST" PKG_ROOT="$tmp/pkgroot" PKGDIR="$pkg" \
        KERNEL_WORK="$tmp/work" XDG_CACHE_HOME="$tmp/cache" \
        bash "$script" "$1" 2>&1)
    rc=$?
    set -e
}

expect_refused() {  # expect_refused <name> <KDIR>
    run_guard "$2"
    if [ "$rc" -ne 0 ] && grep -q 'refusing KDIR' <<<"$out"; then
        echo "PASS: $1 refused (rc=$rc)"
    else
        echo "FAIL: $1 was not refused (rc=$rc): $out" >&2
        fail=1
    fi
}

HOME_FOR_TEST="$tmp/home"
mkdir -p "$HOME_FOR_TEST"

# 1. A linux-* directory with user data: refused, and the data survives.
mkdir -p "$tmp/linux-data"
printf 'precious\n' >"$tmp/linux-data/notes.txt"
expect_refused "existing non-kernel linux-* directory" "$tmp/linux-data"
if [ -f "$tmp/linux-data/notes.txt" ]; then
    echo "PASS: user file in refused KDIR survives"
else
    echo "FAIL: user file deleted by a refused KDIR" >&2
    fail=1
fi

# 2. A symlink named linux-*: refused, and its target survives.
mkdir -p "$tmp/victim"
printf 'keep\n' >"$tmp/victim/data"
ln -s "$tmp/victim" "$tmp/linux-link"
expect_refused "symlink KDIR" "$tmp/linux-link"
if [ -f "$tmp/victim/data" ]; then
    echo "PASS: symlink target survives"
else
    echo "FAIL: symlink KDIR removed its target" >&2
    fail=1
fi

# 2b. The same symlink with a trailing slash. `-L` looks through a link when the
# path ends in `/`, so the guard must normalise the path first, or `rm -rf` would
# empty the link's target. The refusal must be the symlink guard's own message.
mkdir -p "$tmp/victim-slash"
printf 'keep\n' >"$tmp/victim-slash/data"
printf 'Makefile\n' >"$tmp/victim-slash/Makefile"
ln -s "$tmp/victim-slash" "$tmp/linux-link-slash"
run_guard "$tmp/linux-link-slash/"
if [ "$rc" -ne 0 ] && grep -q 'root, home or symlink' <<<"$out"; then
    echo "PASS: symlink KDIR with a trailing slash refused (rc=$rc)"
else
    echo "FAIL: symlink KDIR with a trailing slash was not refused by the symlink guard (rc=$rc): $out" >&2
    fail=1
fi
if [ -f "$tmp/victim-slash/data" ] && [ -f "$tmp/victim-slash/Makefile" ]; then
    echo "PASS: target of a trailing-slash symlink survives"
else
    echo "FAIL: trailing-slash symlink KDIR emptied its target" >&2
    fail=1
fi

# 2c. An existing kernel tree with a mount point below it: refused before any
# deletion. The mount table is a fixture (LCCC_MOUNTINFO), so no privilege is needed.
mkdir -p "$tmp/linux-mounted"
printf 'Makefile\n' >"$tmp/linux-mounted/Makefile"
printf 'precious\n' >"$tmp/linux-mounted/notes.txt"
# Only a tree this script prepared may be replaced, so the fixture is stamped.
printf 'stamp\n' >"$tmp/linux-mounted/.lccc-prepared"
printf '%s\n' "36 35 0:31 / $tmp/linux-mounted/sub rw - tmpfs tmpfs rw" >"$tmp/mountinfo"
set +e
out=$(LCCC_MOUNTINFO="$tmp/mountinfo" HOME="$HOME_FOR_TEST" PKG_ROOT="$tmp/pkgroot" PKGDIR="$pkg" \
    KERNEL_WORK="$tmp/work" XDG_CACHE_HOME="$tmp/cache" bash "$script" "$tmp/linux-mounted" 2>&1)
rc=$?
set -e
if [ "$rc" -ne 0 ] && grep -q 'a mount point lies below it' <<<"$out"; then
    echo "PASS: KDIR with a mount below it refused (rc=$rc)"
else
    echo "FAIL: KDIR with a mount below it was not refused (rc=$rc): $out" >&2
    fail=1
fi
if [ -f "$tmp/linux-mounted/notes.txt" ]; then
    echo "PASS: file in a KDIR with a mount below it survives"
else
    echo "FAIL: file deleted from a KDIR with a mount below it" >&2
    fail=1
fi

# 3. KDIR equal to $HOME, which has a linux-* basename: refused, contents kept.
HOME_FOR_TEST="$tmp/linux-home"
mkdir -p "$HOME_FOR_TEST"
printf 'home\n' >"$HOME_FOR_TEST/.profile"
expect_refused "KDIR equal to HOME" "$HOME_FOR_TEST"
if [ -f "$HOME_FOR_TEST/.profile" ]; then
    echo "PASS: HOME contents survive"
else
    echo "FAIL: HOME contents removed" >&2
    fail=1
fi
HOME_FOR_TEST="$tmp/home"

# 4. A basename that is not linux-*: refused.
expect_refused "non-linux basename" "$tmp/notlinux"

# 5. mounts_under: the root filesystem is mounted, an empty path is not.
# shellcheck source=/dev/null
source scripts/mountinfo.sh
mounts_under() { has_mount_under "$1"; }
if mounts_under / ; then
    echo "PASS: mounts_under / sees the root mount"
else
    echo "FAIL: mounts_under / reported no mount" >&2
    fail=1
fi
if mounts_under "$tmp/no-such-dir"; then
    echo "FAIL: mounts_under reported a mount for an unmounted path" >&2
    fail=1
else
    echo "PASS: mounts_under is clear for an unmounted path"
fi

if [ "$fail" -ne 0 ]; then
    echo "check_kernel_tree_clear: FAILED" >&2
    exit 1
fi
echo "check_kernel_tree_clear: PASSED"
