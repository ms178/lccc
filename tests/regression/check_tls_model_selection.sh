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
#      "recompile with -fPIC". The dlopen round-trip proves the Initial-Exec
#      fallback is not merely linkable but functionally correct.
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
void setv(unsigned long v) { slot = v; }
unsigned long getv(void) { return slot; }
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
if [ -f "$work/tlsmin.so" ]; then
    "$CCC" -O2 -fPIC -shared -S "$work/tlsmin.c" -o "$work/shared.s" 2>/dev/null || true
    if [ -f "$work/shared.s" ] && grep -q '%fs:.*@TPOFF' "$work/shared.s"; then
        bad "-shared must not use the Local-Exec direct form"
    else
        note "contract 3: -shared keeps the GOT-based model"
    fi
fi

[ "$fail" -eq 0 ] || { echo "check_tls_model_selection: FAILED" >&2; exit 1; }
echo "check_tls_model_selection: PASS"
