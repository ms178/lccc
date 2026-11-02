#!/usr/bin/env bash
# check_ensure_yarpgen.sh — contract tests for scripts/ensure_yarpgen.sh.
#
# The provisioner is the only thing standing between the differential
# harness and a generator that silently does not exist, so its trust model
# is a contract, not an implementation detail:
#
#   * a stamp is trusted only when it names the pinned ref AND the binary
#     exists AND a fresh generate/compile/run probe of that binary passes;
#   * a failed or interrupted re-provision leaves NO stamp behind;
#   * checkout drift is refused rather than built;
#   * --check reports the absence of a valid provision instead of
#     provisioning (CI uses it to decide SKIP versus run).
#
# Everything here runs offline: git and cmake are replaced by PATH shims, so
# the test exercises the script's decisions (stamp, probe, drift, re-provision)
# with no network, no compiler build, and no cmake dependency.
set -euo pipefail
export LC_ALL=C

here=$(cd "$(dirname "$0")" && pwd)
script="$here/../../scripts/ensure_yarpgen.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/ensure-yarpgen-test.XXXXXX")
trap 'rm -rf "$work"' EXIT

ref=1adb290f453505838f3aa33dc571f292b0c1810f
dest="$work/prefix"
shim="$work/shim"
out="$work/out"
err="$work/err"
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
stamp_ok() { [[ -f $dest/.lccc-provisioned ]] && grep -qx "ref=$ref" "$dest/.lccc-provisioned" \
    && grep -qx 'probe=ok' "$dest/.lccc-provisioned"; }
no_stamp() { [[ ! -e $dest/.lccc-provisioned ]]; }
err_has() { grep -q -- "$1" "$err"; }
out_has() { grep -q -- "$1" "$out"; }

# --- fakes --------------------------------------------------------------
# A working generator: writes the three files YARPGen emits, and its driver
# prints a decimal hash so the provisioner's probe can distinguish "ran" from
# "ran and produced something".
fake_good="$work/yarpgen.good"
cat >"$fake_good" <<'EOF'
#!/bin/sh
# fake yarpgen: -o DIR (DIR already exists)
out=
while [ $# -gt 0 ]; do
    case $1 in -o) out=$2; shift ;; esac
    shift
done
[ -n "$out" ] || exit 1
printf '%s\n' 'void test(void) { }' >"$out/func.c"
printf '%s\n' '/* nothing */' >"$out/init.h"
cat >"$out/driver.c" <<'EOC'
#include <stdio.h>
int main(void) { printf("%llu\n", 12345ULL); return 0; }
EOC
EOF
chmod +x "$fake_good"

# A generator that "succeeds" but produces nothing: the probe must reject it.
fake_empty="$work/yarpgen.empty"
printf '#!/bin/sh\nexit 0\n' >"$fake_empty"
chmod +x "$fake_empty"

for tool in git cmake; do mkdir -p "$shim"; done
cat >"$shim/git" <<EOF
#!/bin/sh
# Minimal git: only the three verbs the provisioner calls.
printf '%s\n' "\$*" >>"$work/git.log"
case "\$*" in
    *clone*) mkdir -p "\$4/.git"; exit 0 ;;
    *rev-parse*) printf '%s\n' "\${FAKE_GIT_HEAD:-$ref}"; exit 0 ;;
    *) exit 0 ;;
esac
EOF
chmod +x "$shim/git"
cat >"$shim/cmake" <<EOF
#!/bin/sh
# Minimal cmake: "build" installs \$FAKE_YARPGEN_SRC where the provisioner
# expects the binary.
case "\$*" in
    --build*)
        mkdir -p "\$FAKE_YARPGEN_OUT/build"
        cp "\$FAKE_YARPGEN_SRC" "\$FAKE_YARPGEN_OUT/build/yarpgen"
        chmod +x "\$FAKE_YARPGEN_OUT/build/yarpgen"
        exit 0 ;;
    *) exit 0 ;;
esac
EOF
chmod +x "$shim/cmake"

run() { # run [args...] ; YARPGEN_REF stays at the pin unless the caller overrides
    LCCC_YARPGEN_PREFIX="$dest" FAKE_YARPGEN_OUT="$dest" \
        FAKE_YARPGEN_SRC="${GEN:-$fake_good}" PATH="$shim:$PATH" \
        bash "$script" "$@" >"$out" 2>"$err"
}

# 1. --check with nothing provisioned: refuse, do not build.
if run --check; then
    echo "FAIL check-empty: --check succeeded with no provision" >&2
    fail=1
else
    check check-empty-reason err_has "no valid provision"
    check check-empty-no-stamp no_stamp
fi
unset GEN

# 2. unknown argument is a usage error, never a silent success.
if run --bogus-argument; then
    echo "FAIL unknown-arg: accepted an unknown argument" >&2
    fail=1
else
    check unknown-arg-exit [ "$?" -ne 0 ]
fi

# 3. fresh provision: build, probe, stamp; --print-bin reports the path.
rm -f "$work/git.log"
check fresh-provision run --print-bin
check fresh-stamped stamp_ok
check fresh-bin out_has "$dest/build/yarpgen"
check fresh-ref grep -qx "ref=$ref" "$dest/.lccc-provisioned"

# 4. idempotent: a valid provision is re-probed but not rebuilt.
clones_before=$(grep -c clone "$work/git.log" || true)
check idempotent-quiet run
check idempotent-no-clone [ "$(grep -c clone "$work/git.log" || true)" = "$clones_before" ]

# 5. a foreign stamp (different ref) is not trusted: re-provision.
printf 'ref=deadbeefdeadbeefdeadbeefdeadbeefdeadbeef\nprobe=ok\n' >"$dest/.lccc-provisioned"
check foreign-ref-reprovisions run
check foreign-ref-restamped stamp_ok

# 6. a stamp whose binary no longer probes is not trusted either. The binary
#    exists and the stamp names the pin, but the binary emits no digits.
cp "$dest/.lccc-provisioned" "$work/stamp.bak"
printf '#!/bin/sh\nexit 0\n' >"$dest/build/yarpgen"
chmod +x "$dest/build/yarpgen"
check broken-binary-reprovisions run
check broken-binary-restamped stamp_ok

# 7. --check on a damaged provision fails instead of repairing.
printf '#!/bin/sh\nexit 0\n' >"$dest/build/yarpgen"
chmod +x "$dest/build/yarpgen"
if run --check; then
    echo "FAIL check-damaged: --check accepted a binary that fails the probe" >&2
    fail=1
else
    check check-damaged-reason err_has "no valid provision"
fi

# 8. a probe failure during provisioning must leave NO stamp (a half-built
#    generator is not a provision).
GEN="$fake_empty"
if run --force; then
    echo "FAIL probe-failure: provisioning succeeded with a generator that emits nothing" >&2
    fail=1
else
    check probe-failure-reason err_has "failed the generate/compile/run probe"
    check probe-failure-no-stamp no_stamp
fi
unset GEN

# 9. checkout drift (the fetched ref is not the pin) is refused, not built.
export FAKE_GIT_HEAD=0000000000000000000000000000000000000000
if run --force; then
    echo "FAIL drift: built a checkout that does not match the pin" >&2
    fail=1
else
    check drift-reason err_has "checkout drift"
    check drift-no-stamp no_stamp
fi
unset FAKE_GIT_HEAD

# 10. recovery: after the damaged runs, one clean run re-provisions.
check recovered run
check recovered-stamped stamp_ok

if [[ $fail -ne 0 ]]; then
    echo "check_ensure_yarpgen: FAILED" >&2
    exit 1
fi
echo "check_ensure_yarpgen: all checks passed"
