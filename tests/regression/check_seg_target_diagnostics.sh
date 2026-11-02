#!/usr/bin/env bash
# x86 named address spaces must fail as diagnostics, never backend panics, on
# unsupported architectures. Test declaration-only use too (no load to trap).
set -euo pipefail
export LC_ALL=C
ccc=${CCC:-target/fastbuild/lccc}
bindir=$(dirname -- "$ccc")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail=0
for space in gs fs; do
    printf 'long f(long __seg_%s *p) { return *p; }\n' "$space" > "$tmp/load.c"
    printf 'void f(long __seg_%s *p) { *p = 7; }\n' "$space" > "$tmp/store.c"
    printf 'extern long __seg_%s object;\n' "$space" > "$tmp/decl.c"
    for src in "$tmp"/*.c; do
        for compiler in "$ccc" "$bindir/lccc-i686"; do
            "$compiler" -O2 -S "$src" -o "$tmp/positive.s"
        done
        for target in arm riscv; do
            compiler="$bindir/lccc-$target"
            for mode in -S -fsyntax-only; do
                if "$compiler" "$mode" "$src" -o "$tmp/reject.s" > "$tmp/err" 2>&1; then
                    echo "FAIL: $target accepted __seg_$space in ${src##*/} ($mode)" >&2
                    fail=1
                else
                    status=$?
                    if [[ $status -ne 1 ]] || ! grep -q 'named address spaces require an x86 target' "$tmp/err" \
                        || grep -qE 'panicked|internal error' "$tmp/err"; then
                        echo "FAIL: $target did not give the target diagnostic ($space, ${src##*/}, $mode, status=$status)" >&2
                        cat "$tmp/err" >&2
                        fail=1
                    fi
                fi
            done
        done
    done
done
# Generic pointers and an inactive named qualifier must remain legal everywhere.
cat > "$tmp/control.c" <<'C'
#if 0
extern long __seg_gs unused;
#endif
long f(long *p) { return *p; }
C
for compiler in "$ccc" "$bindir/lccc-i686" "$bindir/lccc-arm" "$bindir/lccc-riscv"; do
    "$compiler" -S "$tmp/control.c" -o "$tmp/control.s"
done
[[ $fail -eq 0 ]] || exit 1
echo 'segment target diagnostics: PASS (x86/i686 positive, ARM/RISC-V negative, generic/inactive controls)'
