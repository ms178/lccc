#!/usr/bin/env bash
# check_ensure_gcc_torture.sh — contract tests for scripts/ensure_gcc_torture.sh.
#
# Runs the provisioning script against a synthetic release tarball served
# from a file:// mirror (no network), then damages the provisioned tree in
# every way a harness wipe or an interrupted run can, and checks that each
# damage is detected, reported and repaired — and that a stamp is never left
# behind for a tree that was not validated.
set -euo pipefail
export LC_ALL=C

here=$(cd "$(dirname "$0")" && pwd)
script="$here/../../scripts/ensure_gcc_torture.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/ensure-torture-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

ver=gcc-0.0.1-lccctest
fail=0
check() { # check NAME CONDITION...
    local name=$1
    shift
    if "$@"; then
        echo "PASS $name"
    else
        echo "FAIL $name" >&2
        fail=1
    fi
}

# --- synthetic release --------------------------------------------------
rel="$work/src/$ver/gcc/testsuite"
mkdir -p "$rel/gcc.c-torture/execute" "$rel/gcc.c-torture/compile" \
    "$rel/gcc.dg/torture" "$work/mirror/$ver"
printf 'int main(void){return 0;}\n' >"$rel/gcc.c-torture/execute/a.c"
printf 'int main(void){return 1-1;}\n' >"$rel/gcc.c-torture/execute/b.c"
printf 'void f(void){}\n' >"$rel/gcc.c-torture/compile/c.c"
printf '#define X 1\n' >"$rel/gcc.dg/torture/h.h"
printf '/* { dg-do run } */\n' >"$rel/gcc.dg/d.c"
tar -cJf "$work/mirror/$ver/$ver.tar.xz" -C "$work/src" "$ver"
good_pin=$(sha512sum <"$work/mirror/$ver/$ver.tar.xz" | cut -d' ' -f1)

dl="$work/dl"
dest="$work/suite"
tarball="$dl/$ver.tar.xz"
out="$work/out"
err="$work/err"

run() { # run [--force] ; env: PIN (default good), extra env via caller
    GCC_TORTURE_SHA512=${PIN:-$good_pin} GCC_TORTURE_URL="file://$work/mirror" \
        bash "$script" "$@" "$tarball" "$dest" >"$out" 2>"$err"
}
stamp_ok() { [[ -f $dest/.lccc-provisioned ]] && [[ $(head -n1 "$dest/.lccc-provisioned") == "$ver" ]]; }
no_stamp() { [[ ! -e $dest/.lccc-provisioned ]]; }
err_has() { grep -q -- "$1" "$err"; }
err_empty() { [[ ! -s $err ]]; }
tree_intact() {
    cmp -s "$dest/gcc.c-torture/execute/a.c" "$rel/gcc.c-torture/execute/a.c" &&
        cmp -s "$dest/gcc.c-torture/execute/b.c" "$rel/gcc.c-torture/execute/b.c" &&
        [[ -f $dest/gcc.dg/torture/h.h ]] && [[ ! -e $dest/gcc.c-torture/execute/zz.c ]]
}

# 1. fresh provision: download via the mirror, validate, stamp.
run
check fresh-provision stamp_ok
check fresh-reason err_has "no stamp"
check fresh-report grep -q "execute=2 compile=1" "$out"
check fresh-stamp-fields grep -q "^manifest-sha256=[0-9a-f]\{64\}$" "$dest/.lccc-provisioned"
check fresh-tree tree_intact

# 2. idempotent: a valid tree is verified and left alone.
inode=$(stat -c %i "$dest/gcc.c-torture")
run
check idempotent-quiet err_empty
check idempotent-untouched [ "$(stat -c %i "$dest/gcc.c-torture")" = "$inode" ]

# 3-8. damage -> detected with the right reason -> repaired.
damage_case() { # NAME REASON DAMAGE-COMMAND...
    local name=$1 reason=$2
    shift 2
    "$@"
    if run; then
        check "$name-reason" err_has "$reason"
        check "$name-repaired" tree_intact
        check "$name-stamped" stamp_ok
    else
        echo "FAIL $name: script exited non-zero: $(cat "$err")" >&2
        fail=1
    fi
}
damage_case missing-file "file set differs" rm "$dest/gcc.c-torture/execute/b.c"
damage_case extra-file "file set differs" touch "$dest/gcc.c-torture/execute/zz.c"
damage_case altered-file "file content differs" \
    sh -c "echo '/* x */' >>'$dest/gcc.c-torture/execute/a.c'"
damage_case missing-gcc-dg "required directory gcc.dg" rm -rf "$dest/gcc.dg"
damage_case missing-manifest "manifest missing" rm "$dest/.lccc-manifest"
damage_case tampered-manifest "manifest hash mismatch" \
    sh -c "echo 'x' >>'$dest/.lccc-manifest'"
damage_case legacy-stamp "different tarball" \
    sh -c "printf '%s\n' '$ver' >'$dest/.lccc-provisioned'"
damage_case foreign-version "stamp names" \
    sh -c "sed -i '1s/.*/gcc-9.9.9/' '$dest/.lccc-provisioned'"

# 9. corrupt cached tarball: discarded and fetched again.
damage_case corrupt-tarball "fails its sha512 pin" \
    sh -c "rm -f '$dest/.lccc-provisioned'; head -c 4096 /dev/urandom >'$tarball'"

# 10. a download that does not match the pin is refused, no stamp written.
rm -f "$tarball"
bad_pin=$(printf '%0128d' 0)
if PIN=$bad_pin run; then
    echo "FAIL bad-download: accepted a tarball that fails its pin" >&2
    fail=1
else
    check bad-download-reason err_has "does not match its sha512 pin"
    check bad-download-no-stamp no_stamp
fi

# 11. interrupted replacement: a failure in the directory swap must leave
#     no stamp (it is invalidated before the tree is touched), and the next
#     run must repair.
run
shim="$work/shim"
mkdir -p "$shim"
cat >"$shim/mv" <<'EOF'
#!/bin/sh
for a in "$@"; do
    case $a in */gcc.dg) echo "mv: simulated interruption" >&2; exit 1 ;; esac
done
exec /bin/mv "$@"
EOF
chmod +x "$shim/mv"
if PATH="$shim:$PATH" run --force; then
    echo "FAIL interrupted: the simulated interruption did not fail the run" >&2
    fail=1
else
    check interrupted-no-stamp no_stamp
fi
run
check interrupted-then-repaired stamp_ok
check interrupted-then-repaired-tree tree_intact

# 12. --force re-extracts a valid tree.
inode=$(stat -c %i "$dest/gcc.c-torture")
run --force
check force-reextracts [ "$(stat -c %i "$dest/gcc.c-torture")" != "$inode" ]
check force-stamped stamp_ok

# 13. an unpinned version without GCC_TORTURE_SHA512 is refused up front.
if env -u GCC_TORTURE_SHA512 bash "$script" "$dl/gcc-0.0.2-unpinned.tar.xz" "$work/other" \
    >"$out" 2>"$err"; then
    echo "FAIL unpinned: accepted a version without a pin" >&2
    fail=1
else
    check unpinned-reason err_has "no sha512 pin"
    check unpinned-untouched [ ! -e "$work/other" ]
fi

if [[ $fail -ne 0 ]]; then
    echo "check_ensure_gcc_torture: FAILED" >&2
    exit 1
fi
echo "check_ensure_gcc_torture: all checks passed"
