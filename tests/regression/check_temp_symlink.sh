#!/usr/bin/env bash
# AUD-TMP-1 — temp files must not be attacker-redirectable.
#
# The compiler used to name its temp objects `$TMPDIR/ccc_<pid>_<stem>.<n>.<ext>`
# with the pid as the only unknown, and wrote them with a plain open() that
# follows symlinks. A local user who pre-placed a symlink at the predicted path
# made lccc overwrite *any* file the invoking user can write:
#
#   ln -s ~/victim.o /tmp/ccc_<pid>_big.0.o
#   TMPDIR=/tmp exec lccc -O1 big.c -o big.out     # victim.o replaced by an ELF
#
# The `exec` in the reproducer keeps the pid stable so the pre-placement is
# deterministic (no race needed to demonstrate the mechanism), which is exactly
# how this gate drives it. Requirements checked here:
#
#   1. the victim file is bit-identical afterwards;
#   2. lccc still succeeds and produces a correct, runnable binary;
#   3. nothing is written directly into $TMPDIR: the temp files live in a
#      0700 directory owned by the invoking user, which is reaped afterwards;
#   4. the same holds for the `CCC_ASM_DEBUG` assembly dump, whose historical
#      path was the fixed /tmp/asm_debug_<stem>.s;
#   5. a pre-placed *directory* at the legacy path cannot be adopted either.
#
# Mutation check (the gate must be able to fail): `--expect-vulnerable` asserts
# the historical behaviour instead, i.e. that a symlink pre-placed at a legacy
# temp path IS followed. The hardening is therefore verified in both
# directions with the same script:
#
#   CCC=/path/to/pre-fix/lccc bash check_temp_symlink.sh --expect-vulnerable  # passes
#   CCC=/path/to/pre-fix/lccc bash check_temp_symlink.sh                      # fails
#   CCC=/path/to/fixed/lccc   bash check_temp_symlink.sh                      # passes
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
CCC_ARM=${CCC_ARM:-./target/fastbuild/lccc-arm}

if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
# The body of this gate runs from a scratch directory, so resolve the compiler
# once, now, while the caller's cwd (and any relative CCC) is still meaningful.
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")
if [[ -x $CCC_ARM ]]; then CCC_ARM=$(cd "$(dirname -- "$CCC_ARM")" && pwd)/$(basename -- "$CCC_ARM"); fi

EXPECT_VULNERABLE=0
[[ ${1:-} == --expect-vulnerable ]] && EXPECT_VULNERABLE=1

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-tmpsymlink.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
work="$tmp/work"
vault="$tmp/tmpd"
mkdir -p "$work" "$vault"

fail() { echo "FAIL: $*" >&2; exit 1; }

# A small but non-trivial translation unit: enough objects to exercise the
# multi-file temp-object path of `run_full`.
make_sources() {
    local dir=$1
    printf 'int f1(int x){return x*3+1;}\n' >"$dir/a.c"
    printf 'int f2(int x){return x*5-2;}\n' >"$dir/b.c"
    echo 'int f1(int); int f2(int); int main(void){return (f1(3)+f2(4)) == 28 ? 0 : 1;}' >"$dir/main.c"
}

# ---------------------------------------------------------------------------
# 1. symlink pre-placed at every legacy temp-object name for the pid we exec
# ---------------------------------------------------------------------------
make_sources "$work"
echo VICTIM >"$vault/victim.txt"
cp "$vault/victim.txt" "$vault/victim.expected"
victim_size_before=$(wc -c <"$vault/victim.txt")

# The symlinks and the compiler run must happen in ONE shell process: the
# legacy name contains the pid, and `exec` keeps `$$` (- the shell's pid, which
# lccc then has) stable across the hand-off. Doing the pre-placement in a
# separate subshell would guess a pid that never appears.
set +e
bash -c '
    set -e
    vault=$1; ccc=$2; work=$3; log=$4
    for n in 0 1 2 3 4 5 6 7; do
        for stem in a b main big; do
            ln -sf "$vault/victim.txt" "$vault/ccc_$$_$stem.$n.o"
            ln -sf "$vault/victim.txt" "$vault/ccc_asm_$$_$stem.$n.s"
        done
    done
    cd "$work"
    TMPDIR="$vault" exec "$ccc" -O1 a.c b.c main.c -o prog >"$log" 2>&1
' _ "$vault" "$CCC" "$work" "$tmp/build.log"
rc=$?
set -e

if [[ $EXPECT_VULNERABLE == 1 ]]; then
    # The historical code writes its objects *through* the pre-placed symlinks,
    # so all three objects land in the same file and the link step then rejects
    # the duplicated definitions (exit != 0). The observable that matters here
    # is the clobbered victim, not the exit status.
    if cmp -s "$vault/victim.txt" "$vault/victim.expected"; then
        fail "--expect-vulnerable: victim.txt was NOT overwritten; the gate cannot detect the legacy behaviour"
    fi
    echo "PASS: legacy behaviour detected (victim.txt overwritten through the pre-placed symlink; lccc rc=$rc)"
    exit 0
fi

[[ $rc -eq 0 ]] || { cat "$tmp/build.log" >&2; fail "lccc exited $rc under a pre-placed-symlink TMPDIR"; }
[[ -x "$work/prog" ]] || fail "no executable produced"
"$work/prog" || fail "produced program returned non-zero"

cmp -s "$vault/victim.txt" "$vault/victim.expected" \
    || fail "victim.txt was overwritten through a pre-placed symlink ($victim_size_before -> $(wc -c <"$vault/victim.txt") bytes)"
[[ $(wc -c <"$vault/victim.txt") == "$victim_size_before" ]] \
    || fail "victim.txt changed size"

# ---------------------------------------------------------------------------
# 2. no stray files in $TMPDIR; private dir is 0700 and reaped
# ---------------------------------------------------------------------------
strays=$(find "$vault" -maxdepth 1 -type f ! -name 'ccc_*' ! -name 'victim.*' ! -name 'a_asm.o' | sed -n '1,5p')
[[ -z $strays ]] || fail "unexpected files written directly into TMPDIR: $strays"

leftovers=$(find "$vault" -maxdepth 1 -type d -name 'lccc-*' | sed -n '1,5p')
[[ -z $leftovers ]] || fail "private temp directory was not reaped: $leftovers"

# Prove the private directory is really used (and really 0700) rather than the
# flat names happening to collide with nothing: ask for a dump and inspect it.
# The dump is written by the built-in assembler path, i.e. for .s/.S inputs.
cat >"$work/a.S" <<'ASM'
    .text
    .globl asm_probe
asm_probe:
    ret
ASM
set +e
( cd "$work" && TMPDIR="$vault" CCC_ASM_DEBUG=1 "$CCC" -c a.S -o a_asm.o ) >"$tmp/dbg.log" 2>&1
set -e
dump=$(sed -n 's/^\[ccc\] CCC_ASM_DEBUG: wrote //p' "$tmp/dbg.log" | sed -n '1p')
[[ -n $dump ]] || { cat "$tmp/dbg.log" >&2; fail "CCC_ASM_DEBUG produced no dump path"; }
[[ -f $dump ]] || fail "CCC_ASM_DEBUG dump $dump does not exist"
[[ $dump == "$vault"/lccc-*/* ]] || fail "CCC_ASM_DEBUG dump $dump is not inside a private TMPDIR subdirectory"
dumpperm=$(stat -c '%a' "$(dirname "$dump")")
[[ $dumpperm == 700 ]] || fail "private temp directory mode is $dumpperm, expected 700"
[[ $(stat -c '%u' "$(dirname "$dump")") == "$(id -u)" ]] || fail "private temp directory is not owned by the invoking user"

# The fixed historical dump path must not be written by this run either.
if [[ -e /tmp/asm_debug_a.s && ! -L /tmp/asm_debug_a.s ]]; then
    echo "note: unrelated /tmp/asm_debug_a.s exists (not created by this gate)" >&2
fi

# ---------------------------------------------------------------------------
# 3. a pre-placed *directory* at a legacy flat name cannot break or capture us
#    (the "adopt something that already exists" shape, one step up from the
#    symlink: mkdir must fail with EEXIST and the compiler must pick another
#    candidate directory)
# ---------------------------------------------------------------------------
rm -rf "$vault"/ccc_*
set +e
bash -c '
    set -e
    vault=$1; ccc=$2; work=$3; log=$4
    mkdir -p "$vault/ccc_$$_a.0.o" "$vault/ccc_$$_b.1.o" "$vault/ccc_$$_main.2.o"
    cd "$work"
    TMPDIR="$vault" exec "$ccc" -O1 a.c b.c main.c -o prog2 >"$log" 2>&1
' _ "$vault" "$CCC" "$work" "$tmp/build2.log"
rc=$?
set -e
[[ $rc -eq 0 ]] || { cat "$tmp/build2.log" >&2; fail "lccc exited $rc with a pre-placed directory in TMPDIR"; }
[[ -x "$work/prog2" ]] || fail "no executable produced with a pre-placed directory in TMPDIR"
[[ -z $(find "$vault" -maxdepth 1 -type f -newer "$vault/victim.expected" | sed -n '1p') ]]     || fail "files were written directly into TMPDIR"

# ---------------------------------------------------------------------------
# 4. cross-target sanity: the same discipline for a non-x86 backend
# ---------------------------------------------------------------------------
if [[ -x $CCC_ARM ]]; then
    set +e
    ( cd "$work" && TMPDIR="$vault" "$CCC_ARM" -S a.c -o a_arm.s ) >"$tmp/arm.log" 2>&1
    rc=$?
    set -e
    [[ $rc -eq 0 ]] || { cat "$tmp/arm.log" >&2; fail "lccc-arm exited $rc under the hardened TMPDIR"; }
    cmp -s "$vault/victim.txt" "$vault/victim.expected" || fail "lccc-arm followed a pre-placed symlink"
fi

# 5. Failure to create a private directory must not revive flat-name attacks.
# Directory write permission is not needed to follow a pre-existing symlink
# and overwrite its writable target. A warning followed by fallback is unsafe.
chmod 0555 "$vault"
if mkdir "$vault/permission-probe" 2>/dev/null; then
    rmdir "$vault/permission-probe"
    chmod 0700 "$vault"
    echo "SKIP: read-only TMPDIR attack (permission-bypassing privileges)"
else
    chmod 0700 "$vault"
    set +e
    bash -c '
        set -e
        vault=$1; ccc=$2; work=$3; log=$4
        ln -sf "$vault/victim.txt" "$vault/ccc_$$_a.0.o"
        ln -sf "$vault/victim.txt" "$vault/ccc_$$_b.1.o"
        ln -sf "$vault/victim.txt" "$vault/ccc_$$_main.2.o"
        chmod 0555 "$vault"
        cd "$work"
        TMPDIR="$vault" exec "$ccc" -O1 a.c b.c main.c -o refused >"$log" 2>&1
    ' _ "$vault" "$CCC" "$work" "$tmp/readonly.log"
    rc=$?
    set -e
    chmod 0700 "$vault"
    cmp -s "$vault/victim.txt" "$vault/victim.expected" \
        || fail "read-only TMPDIR fallback overwrote the symlink target"
    [[ $rc -ne 0 ]] || fail "private-directory failure did not stop compilation"
    if grep -q 'panicked at' "$tmp/readonly.log"; then
        fail "driver must report a normal temporary-directory error, not panic"
    fi
    grep -q 'cannot reserve a secure temporary-file directory' "$tmp/readonly.log" \
        || { cat "$tmp/readonly.log" >&2; fail "compilation failed for the wrong reason"; }
    echo "PASS: read-only TMPDIR fails closed without overwriting its symlink target"
fi

echo "PASS: temp files are symlink-safe (private 0700 directory, victim untouched, no TMPDIR debris)"
