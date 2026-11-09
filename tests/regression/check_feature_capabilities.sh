#!/usr/bin/env bash
# Capability truthfulness gate: every `__has_feature` row that answers 1 must
# correspond to a capability that actually WORKS end to end, not merely parses.
#
# This is the evidence behind the answers in
# src/frontend/preprocessor/feature_queries.rs.  A row that claims a capability
# the back end does not honor is worse than a conservative 0: headers use the
# answer to select code paths.  The gate therefore compiles AND runs each
# claimed capability, and pins the deliberate zeros too.
#
# Deliberate zero pinned here: `c_atomic`.  LCCC parses `_Atomic` but lowers
# `a += 2` to a non-atomic load/add/store (GCC: `lock addl`) and a seq_cst store
# to a plain `movl` (GCC: `xchgl`), so C11 atomic semantics are not implemented.
# See engineering/FOLLOWUP-2026-11-08-rucc-redteam.md for the reproducers.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-feature-caps.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
status=0
fail() { echo "FAIL: $1" >&2; shift; for l in "$@"; do echo "    $l" >&2; done; status=1; }

# --- claimed capabilities, compiled and executed ----------------------------
cat >"$tmp/caps.c" <<'C'
#include <stdio.h>
#include <stdint.h>
#include <pthread.h>
_Static_assert(sizeof(int) == 4, "int must be 4 bytes on every in-scope target");
_Alignas(64) static char aligned_buf[8];
static _Thread_local int tls_a;
static __thread int tls_b;
static void *worker(void *arg) { (void)arg; tls_a = 77; tls_b = 88; return 0; }
int main(void) {
  int fails = 0;
  if ((uintptr_t)aligned_buf % 64) { puts("FAIL c_alignas"); fails++; }
  if (_Alignof(double) != 8) { puts("FAIL c_alignof"); fails++; }
  if (_Generic(1, int: 42, default: 0) != 42) { puts("FAIL c_generic_selections"); fails++; }
  if (0b1010 != 10 || 0B1111 != 15) { puts("FAIL cxx_binary_literals"); fails++; }
  /* True thread-local storage: the worker's writes must not be visible here. */
  tls_a = 1; tls_b = 2;
  pthread_t th;
  if (pthread_create(&th, 0, worker, 0) != 0) { puts("FAIL pthread_create"); return 1; }
  pthread_join(th, 0);
  if (tls_a != 1 || tls_b != 2) { puts("FAIL c_thread_local/tls isolation"); fails++; }
  if (fails) { printf("CAPS_FAIL %d\n", fails); return 1; }
  puts("CAPS_OK");
  return 0;
}
C
if "$CCC" -std=gnu23 -O2 "$tmp/caps.c" -o "$tmp/caps" -pthread >"$tmp/caps.build" 2>&1; then
    if ! "$tmp/caps" >"$tmp/caps.run" 2>&1; then
        fail "claimed capabilities must run correctly" "$(cat "$tmp/caps.run")"
    elif ! grep -q CAPS_OK "$tmp/caps.run"; then
        fail "capability program reported a failure" "$(cat "$tmp/caps.run")"
    fi
else
    fail "claimed capabilities must compile" "$(cat "$tmp/caps.build")"
fi

# --- asm goto (with and without outputs), executed on x86 hosts -------------
# The asm templates are x86; the sibling-driver checks below prove the same
# capability codegens for AArch64 and RISC-V.
case "$(uname -m)" in
x86_64|i?86)
    cat >"$tmp/ago_run.c" <<'C'
#include <stdio.h>
int main(void) {
  int out = 0;
  asm goto ("stc\n\tjc %l[carry]" : : : "memory" : carry);
  out = 1;
 carry:
  if (out != 0) { puts("FAIL gnu_asm_goto"); return 1; }
  int val = 0;
  asm goto ("movl $5, %0\n\tjmp %l[done]" : "=r"(val) : : "memory" : done);
  val = -1;
 done:
  if (val != 5) { puts("FAIL gnu_asm_goto_with_outputs"); return 1; }
  puts("AGO_OK");
  return 0;
}
C
    if "$CCC" -std=gnu23 -O2 "$tmp/ago_run.c" -o "$tmp/ago_run" >"$tmp/ago.build" 2>&1; then
        if ! "$tmp/ago_run" >"$tmp/ago.run" 2>&1 || ! grep -q AGO_OK "$tmp/ago.run"; then
            fail "asm goto with outputs must execute correctly" "$(cat "$tmp/ago.run")"
        fi
    else
        fail "asm goto with outputs must compile" "$(cat "$tmp/ago.build")"
    fi
    ;;
*) echo "note: asm goto runtime proof skipped on $(uname -m) (x86 templates)" ;;
esac

# --- the query answers must agree with the demonstrated reality -------------
cat >"$tmp/answers.c" <<'C'
int alignas = __has_feature(c_alignas);
int alignof = __has_feature(c_alignof);
int generic = __has_feature(c_generic_selections);
int sassert = __has_feature(c_static_assert);
int threadlocal = __has_feature(c_thread_local);
int tls = __has_feature(tls);
int binlit = __has_feature(cxx_binary_literals);
int asmgoto_out = __has_extension(gnu_asm_goto_with_outputs);
int atomic = __has_feature(c_atomic);
int atomic_ext = __has_extension(c_atomic);
C
got=$("$CCC" -std=gnu23 -E "$tmp/answers.c" 2>/dev/null | grep -v '^#' | tr -d ' \n\t')
for kv in alignas=1 alignof=1 generic=1 sassert=1 threadlocal=1 tls=1 binlit=1 asmgoto_out=1 \
          atomic=0 atomic_ext=0; do
    key=${kv%%=*}; val=${kv#*=}
    grep -qE "(^|;)int${key}=${val};" <<<"$got" \
        || fail "query answer [$key]" "got: $got" "want int${key}=${val};"
done

# --- _Static_assert must REJECT a false claim, not just accept a true one ---
printf '_Static_assert(sizeof(int) == 99, "boom");\nint main(void){return 0;}\n' >"$tmp/sa.c"
if "$CCC" -std=gnu23 -c "$tmp/sa.c" -o "$tmp/sa.o" >"$tmp/sa.log" 2>&1; then
    fail "a false _Static_assert must fail the translation unit" "exit 0"
elif ! grep -q "static assertion failed" "$tmp/sa.log"; then
    fail "false _Static_assert diagnostic" "$(cat "$tmp/sa.log")"
fi

# --- UBSan must actually trap, not merely accept the flag -------------------
cat >"$tmp/ub.c" <<'C'
#include <limits.h>
#include <stdio.h>
int main(int argc, char **argv) {
  (void)argv;
  int x = INT_MAX;
  x += argc;            /* signed overflow: must trap under -fsanitize=undefined */
  printf("%d\n", x);
  return 0;
}
C
if "$CCC" -std=gnu23 -O2 -fsanitize=undefined -fsanitize-trap=all "$tmp/ub.c" -o "$tmp/ub" \
        >"$tmp/ub.build" 2>&1; then
    # Run under Python so the expected SIGILL is observed as a signal, not
    # announced by the shell as an "Illegal instruction" job message: the trap
    # is the PASS condition here, and the log must not look like a crash.
    # A normal exit (even 0) means the check was never instrumented.
    if ! python3 -c '
import subprocess, sys
p = subprocess.run([sys.argv[1]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
sys.exit(0 if p.returncode == -4 else 1)
' "$tmp/ub"; then
        fail "ubsan must trap on signed overflow" "expected SIGILL from the instrumented binary"
    fi
else
    fail "ubsan trap mode must compile" "$(cat "$tmp/ub.build")"
fi
# Without a trap mode LCCC has no libubsan runtime and must say so, not pretend.
if "$CCC" -std=gnu23 -fsanitize=undefined -E "$tmp/ub.c" >/dev/null 2>"$tmp/ub.err"; then
    fail "-fsanitize=undefined without trap mode must be rejected" "exit 0"
elif ! grep -q "requires trap mode" "$tmp/ub.err"; then
    fail "ubsan runtime-unavailable diagnostic" "$(cat "$tmp/ub.err")"
fi

# --- every in-scope target must at least codegen the claimed capabilities ---
# Runtime proof above is the host's; the sibling drivers prove the claims are
# not x86-only.  Execution needs a cross sysroot, so this is compile-level.
cat >"$tmp/cross.c" <<'C'
static _Thread_local int tls_v;
_Alignas(32) static char buf[4];
int probe(void) {
  _Static_assert(sizeof(int) == 4, "i32");
  tls_v = 3;
  int g = _Generic(tls_v, int: 1, default: 0);
  return tls_v + g + (0b1010 == 10) + (int)(_Alignof(double) == 8) + buf[0];
}
C
cat >"$tmp/ago_x86.c" <<'C'
int f(void){int v=0;asm goto("movl $5, %0\n\tjmp %l[d]":"=r"(v)::"memory":d);v=-1;d:return v;}
C
cat >"$tmp/ago_arm.c" <<'C'
int f(void){int v=0;asm goto("mov %w0, #5\n\tb %l[d]":"=r"(v)::"memory":d);v=-1;d:return v;}
C
cat >"$tmp/ago_rv.c" <<'C'
int f(void){int v=0;asm goto("li %0, 5\n\tj %l[d]":"=r"(v)::"memory":d);v=-1;d:return v;}
C
check_sibling() {  # <driver> <asm-goto source>
    local driver=$1 ago=$2
    [ -x "$driver" ] || return 0
    if ! "$driver" -std=gnu23 -O2 -S "$tmp/cross.c" -o "$tmp/$(basename "$driver").s" \
            >"$tmp/sib.log" 2>&1; then
        fail "$(basename "$driver"): claimed capabilities must codegen" "$(cat "$tmp/sib.log")"
    fi
    if ! "$driver" -std=gnu23 -O2 -S "$ago" -o "$tmp/$(basename "$driver").ago.s" \
            >>"$tmp/sib.log" 2>&1; then
        fail "$(basename "$driver"): asm goto with outputs must codegen" "$(cat "$tmp/sib.log")"
    fi
    # The capability rows are target-independent claims: every sibling driver
    # must answer exactly as the host does (no -E sysroot is needed here).
    local sib_got kv key val
    sib_got=$("$driver" -std=gnu23 -E "$tmp/answers.c" 2>/dev/null | grep -v '^#' | tr -d ' \n\t')
    for kv in alignas=1 alignof=1 generic=1 sassert=1 threadlocal=1 tls=1 binlit=1 asmgoto_out=1 \
              atomic=0 atomic_ext=0; do
        key=${kv%%=*}; val=${kv#*=}
        grep -qE "(^|;)int${key}=${val};" <<<"$sib_got" \
            || fail "$(basename "$driver"): query answer [$key]" "got: $sib_got" "want int${key}=${val};"
    done
}
here=$(cd "$(dirname "$0")/../.." && pwd)
check_sibling "${CCC_I686:-$here/target/fastbuild/lccc-i686}" "$tmp/ago_x86.c"
check_sibling "${CCC_ARM:-$here/target/fastbuild/lccc-arm}" "$tmp/ago_arm.c"
check_sibling "${CCC_RISCV:-$here/target/fastbuild/lccc-riscv}" "$tmp/ago_rv.c"

if [ "$status" -eq 0 ]; then
    echo "feature-capabilities: ok (claimed rows executed; c_atomic pinned truthful 0)"
fi
exit "$status"
