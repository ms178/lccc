#!/usr/bin/env bash
# __builtin_va_arg_pack_len() folding gate (inliner S17 sentinel rewrite).
#
# The inliner expands the two sentinels the lowering emits for glibc's
# _FORTIFY_SOURCE wrapper idiom (`__builtin_va_arg_pack()` / `..._len()`):
# the value sentinel is deleted and its slot spliced with the call site's
# forwarded arguments, and the length sentinel becomes an I32 copy of their
# count.  The length rewrite matched in the wrong value space — it added the
# clone's value offset to a destination the clone had already shifted — so it
# matched only when the caller's value space happened to be offset by zero.
# Everywhere else an inlined wrapper kept a live call to the undefined
# `__lccc_va_arg_pack_len`, which is a link error for the whole program:
# gzip 1.14's gnulib `open-safer.c` (glibc fortify `open` wrapper) is the
# measured instance.  GCC, Clang and ICX fold the check away.
#
# Checks:
#   1. codegen    — no `__lccc_va_arg_pack*` symbol survives in the emitted
#                   assembly or object at -O0, -O1, -O2 and -O3 (the offset
#                   depends on the caller's value ids, so one level proves
#                   nothing);
#   2. runtime    — the fixture links and runs, and its own checks pass: the
#                   one/two/zero-forwarded cases get counts 1/2/0, so the
#                   wrapper's `> 1` / `!= 0` branches decide exactly as the C
#                   semantics require (folding the sentinel to a constant is
#                   only correct if that constant is the call site's count);
#   3. differential — GCC builds and runs the same source to the same verdict,
#                   so the gate cannot be satisfied by a lccc-only reading.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
src=tests/regression/va_arg_pack_len_folds.c
tmp=${TMPDIR:-/tmp}/lccc-va-arg-pack-len-gate.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
fails=0

note() { printf '  %s\n' "$*"; }
bad() { printf 'FAIL: %s\n' "$*" >&2; fails=$((fails + 1)); }

[ -r "$src" ] || { printf 'FAIL: fixture %s missing\n' "$src" >&2; exit 1; }
[ -x "$CCC" ] || { printf 'FAIL: compiler %s not executable\n' "$CCC" >&2; exit 1; }

# ── 1 + 2: codegen and runtime, all four levels ─────────────────────────────
for opt in -O0 -O1 -O2 -O3; do
    if "$CCC" "$opt" -S -o "$tmp/rt$opt.s" "$src" 2>"$tmp/err"; then
        if grep -q '__lccc_va_arg_pack' "$tmp/rt$opt.s"; then
            bad "$opt: emitted assembly still calls a __lccc_va_arg_pack* sentinel"
        else
            note "codegen   $opt no sentinel call in the emitted assembly"
        fi
    else
        bad "$opt: compilation failed: $(head -c 200 "$tmp/err" | tr '\n' ' ')"
    fi
    if "$CCC" "$opt" -o "$tmp/rt$opt" "$src" 2>"$tmp/err"; then
        # The listing is captured before matching: `nm | grep -q` lets grep
        # exit on the first hit, the producer dies of SIGPIPE, and with
        # `set -o pipefail` the pipeline reports failure — the check would
        # silently pass exactly when the sentinel IS present.
        undef=$(nm -u "$tmp/rt$opt" 2>/dev/null || true)
        if grep -q '__lccc_va_arg_pack' <<<"$undef"; then
            bad "$opt: linked binary still references an undefined sentinel"
        fi
        out=$("$tmp/rt$opt") || { bad "$opt: fixture exited non-zero"; continue; }
        if [ "$out" = "OK" ]; then
            note "runtime   $opt one/two/zero forwarded args take the C paths"
        else
            bad "$opt: fixture verdict '$out' (want OK)"
        fi
    else
        bad "$opt: link failed: $(head -c 200 "$tmp/err" | tr '\n' ' ')"
    fi
done

# ── 3: reference-compiler differential ─────────────────────────────────────
if command -v gcc >/dev/null 2>&1; then
    if gcc -O2 -o "$tmp/rt-gcc" "$src" 2>"$tmp/err"; then
        gout=$("$tmp/rt-gcc") || { bad "gcc: fixture exited non-zero"; gout=""; }
        if [ "$gout" = "OK" ]; then
            note "diff      gcc -O2 folds the same counts (OK)"
        else
            bad "gcc verdict '$gout' differs from lccc's OK"
        fi
    else
        bad "gcc reference build failed: $(head -c 200 "$tmp/err" | tr '\n' ' ')"
    fi
else
    bad "gcc is required for the reference leg of this gate"
fi

if [ "$fails" -ne 0 ]; then
    printf 'va-arg-pack-len gate: %d failure(s)\n' "$fails" >&2
    exit 1
fi
printf 'va-arg-pack-len gate: all checks passed\n'
