#!/usr/bin/env bash
# ============================================================================
# check_tls_model_selection.sh — TLS model selection and the Local-Exec
# direct-access fold (PF-TLS-1).
#
# Three contracts, each pinned separately because each has failed before:
#
#   1. CORRECTNESS ACROSS EVERY MODEL. `tls_local_exec_direct.c` must agree
#      with the GCC oracle on stdout AND exit status at every optimization
#      level and for -fno-pic / -fPIE / -fPIC. The fold covers every width
#      the direct form can carry (8/16/32/64-bit, signed and unsigned) plus
#      constant-offset TLS array slots — the shapes whose addend used to be
#      parsed as a plain `symbol+offset` and degrade to an absolute
#      relocation (a wrong address, not a loud error).
#
#   2. THE SHARED-OBJECT MODEL. `-fPIC -shared` must LINK. Local-Exec is
#      illegal there: the linker rejects `R_X86_64_TPOFF32` for `ET_DYN`,
#      and lccc used to fail every `static __thread` shared build with
#      "recompile with -fPIC". The dlopen round-trip proves the dynamic descriptor
#      path is not merely linkable but functionally correct.
#
#   3. THE MECHANISM FIRES, AND ONLY WHERE IT MAY. `-O2` must contain the
#      one-instruction `%fs:sym@TPOFF` form (a fold that silently stops
#      firing is a FAIL) while `-shared` must NOT contain it.
#
# Contract 2 needs <dlfcn.h>; without it the leg is reported as SKIP, never
# as a pass.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_tls_model_selection: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_tls_model_selection: no gcc oracle" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$here/tls_local_exec_direct.c
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
"$GCC" -O2 "$src" -o "$work/ref"
timeout 30 "$work/ref" > "$work/expected" || bad "gcc oracle exited non-zero"
for opt in -O0 -O1 -O2 -O3 -Os; do
    for mode in "" "-fno-pic" "-fPIE" "-fPIC"; do
        if ! "$CCC" $opt $mode "$src" -o "$work/run" 2>"$work/err"; then
            bad "$opt $mode: compile failed: $(head -1 "$work/err")"
            continue
        fi
        if ! timeout 30 "$work/run" > "$work/actual"; then
            bad "$opt $mode: run failed"
            continue
        fi
        cmp -s "$work/expected" "$work/actual" \
            || bad "$opt $mode: output differs from gcc"
    done
done
note "contract 1: output parity over 5 levels x 4 code models"

# ---------------------------------------------------------------- contract 2
cat > "$work/tlsmin.c" <<'EOF'
static __thread unsigned long slot;
static __thread volatile unsigned long volatile_slot;
void setv(unsigned long v) { slot = v; }
unsigned long getv(void) { return slot; }
unsigned long sumv(void) { return volatile_slot + volatile_slot; }
EOF
cat > "$work/dlmain.c" <<'EOF'
#include <stdio.h>
#include <dlfcn.h>
int main(int argc, char **argv) {
    void *h = dlopen(argv[1], RTLD_NOW);
    if (!h) { printf("dlopen failed: %s\n", dlerror()); return 1; }
    void (*setv)(unsigned long) = dlsym(h, "setv");
    unsigned long (*getv)(void) = dlsym(h, "getv");
    if (!setv || !getv) { printf("dlsym failed\n"); return 1; }
    setv(0x4142434445464748UL);
    unsigned long v = getv();
    printf("%d\n", v == 0x4142434445464748UL);
    return !(v == 0x4142434445464748UL);
}
EOF
if printf '#include <dlfcn.h>\nint main(void){return 0;}\n' | "$GCC" -x c - -o "$work/dlprobe" >/dev/null 2>&1; then
    if "$CCC" -O2 -fPIC -shared "$work/tlsmin.c" -o "$work/tlsmin.so" 2>"$work/err"; then
        "$GCC" -O2 "$work/dlmain.c" -o "$work/dlmain" -ldl
        if timeout 30 "$work/dlmain" "$work/tlsmin.so" > "$work/dlout" && \
            [ "$(cat "$work/dlout")" = "1" ]; then
            note "contract 2: -fPIC -shared links and round-trips through dlopen"
        else
            bad "shared TLS object did not round-trip: $(cat "$work/dlout")"
        fi
    else
        bad "-fPIC -shared failed to link: $(head -1 "$work/err")"
    fi
else
    note "SKIP: contract 2 (no <dlfcn.h> on this host)"
fi

# ---------------------------------------------------------------- contract 3
"$CCC" -O2 -S "$src" -o "$work/o2.s"
if grep -q '%fs:.*@TPOFF' "$work/o2.s"; then
    note "contract 3: -O2 emits the direct Local-Exec form"
else
    bad "-O2 emitted no `%fs:sym@TPOFF` access: the fold stopped firing"
fi
cat >"$work/public-definition.c" <<'C'
__thread unsigned public_tls;
unsigned get_public(void) { return public_tls; }
C
for mode in '' '-fPIE'; do
    "$CCC" -O2 $mode -S "$work/public-definition.c" -o "$work/public${mode:-default}.s"
    if ! grep -q '%fs:public_tls@TPOFF' "$work/public${mode:-default}.s"; then
        bad "executable $mode externally-visible TLS definition missed Local-Exec folding"
    fi
done
note "contract 3: executable TLS definitions use direct Local-Exec"
cat >"$work/external-ie.c" <<'C'
extern __thread unsigned long external_tls;
unsigned long get_external(void) { return external_tls; }
C
"$CCC" -O2 -fno-pic -S "$work/external-ie.c" -o "$work/external-ie.s"
sed -n '/^get_external:/,/^\.size get_external/p' "$work/external-ie.s" \
    >"$work/external-ie-fn.s"
grep -q 'external_tls@GOTTPOFF' "$work/external-ie-fn.s"
grep -q 'movq %fs:(%rax), %rax' "$work/external-ie-fn.s"
if grep -q 'addq %fs:0' "$work/external-ie-fn.s"; then
    bad "Initial-Exec load materialized the thread pointer instead of folding it"
else
    note "contract 3: Initial-Exec load matches the two-instruction oracle"
fi
# A volatile access remains observable when its load is carried directly by
# an adjacent ALU instruction. This is the hot-loop shape where a separate
# `%fs:` load consumed one extra instruction and one temporary GPR per trip.
hot_src="$here/../benchmark/programs/tls_hot_loop.c"
"$CCC" -O3 -fno-pic -S "$hot_src" -o "$work/hot-folded.s"
CCC_NO_SEG_OPERAND_FUSE=1 "$CCC" -O3 -fno-pic -S "$hot_src" -o "$work/hot-control.s"
sed -n '/^tls_hot_loop:/,/^\.size tls_hot_loop/p' "$work/hot-folded.s" >"$work/hot-folded-fn.s"
sed -n '/^tls_hot_loop:/,/^\.size tls_hot_loop/p' "$work/hot-control.s" >"$work/hot-control-fn.s"
if grep -q 'addq %fs:tls_hot_value@TPOFF, %' "$work/hot-folded-fn.s" && \
    ! grep -q 'movq %fs:tls_hot_value@TPOFF' "$work/hot-folded-fn.s" && \
    grep -q 'movq %fs:tls_hot_value@TPOFF' "$work/hot-control-fn.s"; then
    note "contract 3: adjacent volatile TLS load folds into its ALU consumer"
else
    bad "volatile TLS load/ALU fold or its mutation control did not fire"
fi
"$CCC" -O3 -fno-pic -no-pie "$hot_src" -o "$work/hot-run"
"$GCC" -O3 -fno-pic -no-pie "$hot_src" -o "$work/hot-ref"
if [ "$("$work/hot-run" 100003)" = "$("$work/hot-ref" 100003)" ]; then
    note "contract 3: folded volatile TLS hot loop preserves runtime result"
else
    bad "folded volatile TLS hot loop differs from GCC"
fi

if [ -f "$work/tlsmin.so" ]; then
    "$CCC" -O2 -fPIC -shared -S "$work/tlsmin.c" -o "$work/shared.s" 2>/dev/null || true
    if [ -f "$work/shared.s" ] && grep -q '%fs:.*@TPOFF' "$work/shared.s"; then
        bad "-shared must not use the Local-Exec direct form"
    else
        note "contract 3: -shared keeps a dynamic TLS model"
    fi
    if [ -f "$work/shared.s" ]; then
        sed -n '/^getv:/,/^\.size getv/p' "$work/shared.s" >"$work/shared-getv.s"
        if grep -q '@TLSDESC' "$work/shared-getv.s" && \
            grep -q 'movq %fs:(%rax), %rax' "$work/shared-getv.s" && \
            ! grep -q 'addq %fs:0' "$work/shared-getv.s" && \
            ! grep -q 'movq (%rax)' "$work/shared-getv.s"; then
            note "contract 3: dynamic TLS load consumes the descriptor offset directly"
        else
            bad "dynamic TLS load missed the canonical segmented form"
        fi
        sed -n '/^sumv:/,/^\.size sumv/p' "$work/shared.s" >"$work/shared-sumv.s"
        calls=$(grep -c '@TLSCALL' "$work/shared-sumv.s" || true)
        accesses=$(grep -Ec '(movq|addq) %fs:\(%rax\)' "$work/shared-sumv.s" || true)
        if [ "$calls" -eq 2 ] && [ "$accesses" -eq 2 ]; then
            note "contract 3: nested descriptor guards preserve both volatile accesses"
        else
            bad "dynamic volatile loads were coalesced (calls=$calls accesses=$accesses)"
        fi
    fi
fi
# `-shared` itself determines legality even if the caller omitted -fPIC. This
# mutation-sensitive arm catches a direct-fold check that looks only at PIC.
"$CCC" -O2 -shared -S "$work/tlsmin.c" -o "$work/shared-no-pic.s"
if grep -q '%fs:.*@TPOFF' "$work/shared-no-pic.s" || \
    ! grep -q '@TLSDESC' "$work/shared-no-pic.s"; then
    bad "-shared without -fPIC selected or folded to Local-Exec"
else
    note "contract 3: -shared alone selects dynamic TLS"
fi

[ "$fail" -eq 0 ] || { echo "check_tls_model_selection: FAILED" >&2; exit 1; }
echo "check_tls_model_selection: PASS"
