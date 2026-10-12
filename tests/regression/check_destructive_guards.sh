#!/usr/bin/env bash
# Regression for the destructive-path guards in scripts/prepare_kernel_tree.sh,
# scripts/ci_ubuntu_chroot.sh and the shared scripts/mountinfo.sh helper.
#
# Every case stops before any download, debootstrap, mount or deletion of user
# data, so the test needs neither network nor privilege:
#   - mount tables are fixtures (LCCC_MOUNTINFO);
#   - `sudo` is a logging stub placed first on PATH, so every privileged command
#     the chroot script would run is recorded instead of executed;
#   - `debootstrap` is a stub so the bootstrap branch is reachable.
#
# Cases:
#   A. mountinfo.sh: escape decoding, prefix boundaries, "/" and fail-closed.
#   B. prepare_kernel_tree.sh refuses a foreign tree (a Makefile is not
#      ownership), a PKGDIR with '..', and a KDIR with an escaped mount below it.
#   C. ci_ubuntu_chroot.sh refuses unsafe LCCC_CI_CHROOT values before sudo runs.
#   D. ci_ubuntu_chroot.sh refuses to delete a root with a mount below it.
#   E. ci_ubuntu_chroot.sh binds home symlinks but never `/`, nor an ancestor of
#      the chroot.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-destructive-guards.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
fail=0
pass() { echo "PASS: $1"; }
bad() { echo "FAIL: $1" >&2; fail=1; }

# ---- A. mountinfo.sh -------------------------------------------------------
# shellcheck source=../../scripts/mountinfo.sh
source scripts/mountinfo.sh
mi="$tmp/mountinfo"
{
    printf '%s\n' "36 35 0:31 / $tmp/a\\040b/sub rw - tmpfs tmpfs rw"
    printf '%s\n' "37 35 0:32 / $tmp/c\\134040 rw - tmpfs tmpfs rw"
    printf '%s\n' "38 35 0:33 / $tmp/mnt/ab rw - tmpfs tmpfs rw"
    printf '%s\n' "39 35 0:34 / $tmp/t\\011x rw - tmpfs tmpfs rw"
} >"$mi"
export LCCC_MOUNTINFO="$mi"

if has_mount_under "$tmp/a b"; then pass "escaped space (\\040) is decoded"; else bad "\\040 not decoded"; fi
if has_mount_under "$tmp/a b/sub"; then pass "mount below an escaped path"; else bad "below escaped path"; fi
if has_mount_under "$tmp/c\\040"; then
    pass "escaped backslash (\\134) followed by 040 decodes once (literal \\040)"
else bad "\\134040 decoded wrongly"; fi
if has_mount_under "$tmp/c 040"; then bad "\\134040 decoded to a space (double decode)"; else pass "no double decode"; fi
if has_mount_under "$tmp/mnt/a"; then bad "prefix /mnt/a matched /mnt/ab"; else pass "prefix boundary is a path component"; fi
if has_mount_under "$tmp/t"$'\t'"x"; then pass "tab (\\011) is decoded"; else bad "\\011 not decoded"; fi
if has_mount_under "/"; then pass "\"/\" lists every mount"; else bad "/ found nothing"; fi
if has_mount_under "$tmp/unrelated"; then bad "unrelated path reported mounted"; else pass "unrelated path is clear"; fi
export LCCC_MOUNTINFO="$tmp/no-such-table"
if has_mount_under "$tmp/anything"; then pass "unreadable table fails closed"; else bad "unreadable table reported clear"; fi
if list_mounts_under "$tmp" >/dev/null; then bad "list_mounts_under succeeded on an unreadable table"; else pass "list reports the unreadable table"; fi
export LCCC_MOUNTINFO="$mi"

# ---- B. prepare_kernel_tree.sh ---------------------------------------------
prep=scripts/prepare_kernel_tree.sh
pkg="$tmp/pkgroot/packages/linux-cachymod-6.18"
mkdir -p "$pkg" "$tmp/home"
cat >"$pkg/PKGBUILD" <<'PKGBUILD'
_major=6
_minor=18
pkgrel=1
sha256sums=('0000000000000000000000000000000000000000000000000000000000000000')
source=()
PKGBUILD
prep_run() {  # prep_run <KDIR> [PKGDIR] [MOUNTINFO] -> sets out, rc
    set +e
    out=$(HOME="$tmp/home" PKG_ROOT="$tmp/pkgroot" PKGDIR="${2:-$pkg}" \
        LCCC_MOUNTINFO="${3:-$mi}" KERNEL_WORK="$tmp/work" XDG_CACHE_HOME="$tmp/cache" \
        timeout 60 bash "$prep" "$1" 2>&1)
    rc=$?
    set -e
}

# An empty, readable mount table: the ownership check must test the ownership rule alone. An
# unreadable table would fail closed and refuse for the wrong reason.
: >"$tmp/empty-mountinfo"
#  a real kernel checkout has a Makefile. It has no stamp, so it is not ours.
foreign="$tmp/linux-foreign"
mkdir -p "$foreign/kernel"
printf 'VERSION = 6\n' >"$foreign/Makefile"
printf 'precious\n' >"$foreign/kernel/my-work.c"
prep_run "$foreign" "" "$tmp/empty-mountinfo"
if [[ $rc -ne 0 ]] && grep -q 'refusing KDIR' <<<"$out" && [[ -f $foreign/kernel/my-work.c ]]; then
    pass "Makefile-only kernel checkout refused, user file survives"
else bad "foreign tree not protected (rc=$rc): $out"; fi

#  a package path that escapes PKG_ROOT with '..'.
prep_run "$tmp/linux-fresh" "$tmp/pkgroot/packages/../escape/linux-cachymod-6.18" "$tmp/empty-mountinfo"
if [[ $rc -ne 0 ]] && grep -q "'.', '..' or empty component" <<<"$out" && [[ ! -e $tmp/pkgroot/escape ]]; then
    pass "PKGDIR with '..' refused, nothing cloned"
else bad "PKGDIR escape accepted (rc=$rc): $out"; fi

#  an existing stamped tree with an escaped mount below it.
stamped="$tmp/linux sp"
mkdir -p "$stamped"
printf 'stamp\n' >"$stamped/.lccc-prepared"
printf 'precious\n' >"$stamped/notes.txt"
printf '%s\n' "40 35 0:35 / $tmp/linux\\040sp/sub rw - tmpfs tmpfs rw" >"$tmp/mi-sp"
prep_run "$stamped" "" "$tmp/mi-sp"
if [[ $rc -ne 0 ]] && grep -q 'a mount point lies below it' <<<"$out" && [[ -f $stamped/notes.txt ]]; then
    pass "escaped mount below KDIR refused, file survives"
else bad "escaped mount below KDIR not refused (rc=$rc): $out"; fi

# ---- C. ci_ubuntu_chroot.sh root validation --------------------------------
# The script refuses a repository outside $HOME, so the copy under test lives in
# a fixture $HOME. $HOME is set to the fixture for every run.
fake_home="$tmp/home"
mkdir -p "$fake_home/lccc/scripts" "$tmp/bin"
cp scripts/ci_ubuntu_chroot.sh scripts/mountinfo.sh "$fake_home/lccc/scripts/"
chroot_script="$fake_home/lccc/scripts/ci_ubuntu_chroot.sh"
sudo_log="$tmp/sudo.log"
cat >"$tmp/bin/sudo" <<SUDO
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$sudo_log"
exit 0
SUDO
cat >"$tmp/bin/debootstrap" <<'DEBOOTSTRAP'
#!/usr/bin/env bash
exit 0
DEBOOTSTRAP
chmod +x "$tmp/bin/sudo" "$tmp/bin/debootstrap"
chroot_run() {  # chroot_run <ROOT> [MOUNTINFO] -> sets out, rc; sudo calls logged
    : >"$sudo_log"
    set +e
    out=$(HOME="$fake_home" PATH="$tmp/bin:$PATH" LCCC_CI_CHROOT="$1" \
        LCCC_MOUNTINFO="${2:-$mi}" timeout 60 bash "$chroot_script" --setup-only 2>&1)
    rc=$?
    set -e
}
chroot_run_full() {  # as chroot_run, but through the bind and command path
    : >"$sudo_log"
    set +e
    out=$(HOME="$fake_home" PATH="$tmp/bin:$PATH" LCCC_CI_CHROOT="$1" \
        LCCC_MOUNTINFO="${2:-$mi}" timeout 60 bash "$chroot_script" -- true 2>&1)
    rc=$?
    set -e
}
chroot_refused() {  # chroot_refused <name> <ROOT> <message>
    chroot_run "$2"
    if [[ $rc -eq 2 ]] && grep -q -- "$3" <<<"$out" && [[ ! -s $sudo_log ]]; then
        pass "$1"
    else bad "$1 (rc=$rc): $out sudo=$(cat "$sudo_log")"; fi
}
chroot_refused "LCCC_CI_CHROOT=HOME refused before sudo" "$fake_home" 'is or contains $HOME'
chroot_refused "LCCC_CI_CHROOT containing HOME refused" "$tmp" 'is or contains $HOME'
chroot_refused "shallow path /srv refused" "/srv" 'too shallow'
chroot_refused "relative path refused" "relative/root" 'must be absolute'
chroot_refused "'..' component refused" "$tmp/root/../root2" "'.', '..' or empty component"
mkdir -p "$tmp/real-root"
ln -s "$tmp/real-root" "$tmp/root-link"
chroot_refused "symlink root refused" "$tmp/root-link" 'a symlink'
chroot_refused "root equal to the repository refused" "$fake_home/lccc" 'contains the repository'

# ---- D. a leftover mount below the root blocks the bootstrap rm -------------
root="$tmp/root"
mkdir -p "$root/home/user"
printf 'sentinel\n' >"$root/sentinel"
printf '%s\n' "41 35 0:36 / $root/home/user rw - ext4 /dev/root rw" >"$tmp/mi-root"
chroot_run "$root" "$tmp/mi-root"
if [[ $rc -eq 1 ]] && grep -q 'mount points lie below it' <<<"$out" \
    && [[ -f $root/sentinel ]] && ! grep -q '^rm ' "$sudo_log"; then
    pass "leftover mount under the root blocks the rm; sentinel survives"
else bad "leftover mount did not block the bootstrap (rc=$rc): $out sudo=$(cat "$sudo_log")"; fi

# ---- E. bind targets: never '/', never an ancestor of the chroot -----------
root="$tmp/root-e"
mkdir -p "$root" "$tmp/outside"
touch "$root/.lccc-ci-ready"
ln -s "$tmp/outside" "$fake_home/to-outside"
ln -s / "$fake_home/to-host-root"
ln -s "$tmp" "$fake_home/up-to-tmp"
chroot_run_full "$root" "$tmp/no-such-table"
if [[ $rc -eq 0 ]] \
    && grep -qF -- "mount --bind $tmp/outside $root$tmp/outside" "$sudo_log" \
    && ! grep -qF -- "mount --bind / " "$sudo_log" \
    && ! grep -qF -- "mount --bind $tmp " "$sudo_log"; then
    pass "outside link bound at its own path; '/' and the chroot's ancestor skipped"
else bad "bind rules (rc=$rc): $out sudo=$(cat "$sudo_log")"; fi

if [[ $fail -ne 0 ]]; then
    echo "check_destructive_guards: FAILED" >&2
    exit 1
fi
echo "check_destructive_guards: PASSED"
