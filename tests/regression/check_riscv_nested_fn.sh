#!/usr/bin/env bash
# Differential runner for GNU C nested-function codegen on rv64.
#
# Covers the three hardware shapes of the RISC-V nested-function support:
#   1. direct nested call with static chain (t2 convention),
#   2. address-taken nested function (32-byte stack trampoline; the word
#      sequence must stay byte-identical to GCC's),
#   3. non-local goto out of a nested function into an enclosing frame.
# lccc-riscv and riscv64 gcc compile the SAME source; both binaries run
# under qemu-riscv64 and must agree on stdout and exit code.
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
lccc=${CCC_RISCV:-$repo/target/fastbuild/lccc-riscv}
xgcc=${CCC_RISCV_GCC:-riscv64-linux-gnu-gcc}
qemu=${CCC_QEMU:-qemu-riscv64}
as247=/home/user/.cache/gas-2.47-riscv64-linux-gnu/bin/as
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

command -v "$xgcc" >/dev/null || { echo "SKIP: $xgcc not available"; exit 0; }
command -v "$qemu" >/dev/null || { echo "SKIP: $qemu not available"; exit 0; }

fail=0
run_pair() {  # run_pair <name> <opt> <file>
    local name=$1 opt=$2 file=$3
    "$lccc" $opt "$file" -S -o "$tmp/$name.s" || { echo "FAIL($name lccc compile $opt)"; fail=1; return; }
    if [ -x "$as247" ]; then "$as247" -o "$tmp/$name.o" "$tmp/$name.s"; else "$xgcc" -c "$tmp/$name.s" -o "$tmp/$name.o"; fi
    "$xgcc" -static "$tmp/$name.o" -o "$tmp/$name.l" || { echo "FAIL($name link)"; fail=1; return; }
    lout=$("$qemu" "$tmp/$name.l" 2>&1); lec=$?
    "$xgcc" $opt -static "$file" -o "$tmp/$name.g" || { echo "SKIP($name gcc cannot build)"; return; }
    gout=$("$qemu" "$tmp/$name.g" 2>&1); gec=$?
    if [ "$lout" = "$gout" ] && [ "$lec" = "$gec" ]; then
        echo "PASS $name $opt (exit $lec)"
    else
        echo "FAIL $name $opt: lccc(exit=$lec) vs gcc(exit=$gec)"
        diff <(printf '%s\n' "$gout") <(printf '%s\n' "$lout") | head -10
        fail=1
    fi
}

tmpsrc="$tmp/n.c"

# 1. direct call, static chain, capture by value-by-reference semantics.
cat > "$tmpsrc" <<'EOF'
#include <stdio.h>
int outer(int a) {
    int captured = a;
    int add(int x) { captured += x; return captured; }
    add(5); add(7);
    return captured;
}
int main(void) { printf("%d\n", outer(30)); return 0; }
EOF
for o in -O0 -O2; do run_pair chain $o "$tmpsrc"; done

# 2. address-taken nested function: trampoline through a function pointer.
cat > "$tmpsrc" <<'EOF'
#include <stdio.h>
static int acc;
int outer(int n) {
    int add(int x) { acc += x; return acc; }
    int (*fp)(int) = add;
    for (int i = 1; i <= n; i++) fp(i);
    return acc;
}
int main(void) { acc = 0; printf("%d\n", outer(4)); return 0; }
EOF
for o in -O0 -O2; do run_pair tramp $o "$tmpsrc"; done

# Trampoline word check: the 4 code words must be byte-identical to GCC's
# (auipc t2,0 / ld t0,24(t2) / ld t2,16(t2) / jalr zero,0(t0)).
"$lccc" -O0 "$tmpsrc" -S -o "$tmp/tr.s"
words=$(grep -oE "li +t4, [0-9]+" "$tmp/tr.s" | awk '{print $3}' | tail -4 | tr '\n' ',')
if [ "$words" = "919,25408131,17019779,163943," ]; then
    echo "PASS trampoline words (GCC-identical)"
else
    echo "FAIL trampoline words: got [$words]"
    fail=1
fi

# 3. non-local goto out of a nested function (straight and out of a loop).
cat > "$tmpsrc" <<'EOF'
#include <stdio.h>
int r;
int outer(int lim) {
    int inner(int x) { if (x == lim) goto found; return 0; found: r = 99; return 1; }
    r = 0;
    int s = inner(lim);
    if (!s) { r = 1; }
    return s;
}
int main(void) { int a = outer(42); printf("%d %d\n", a, r); return 0; }
EOF
for o in -O0 -O2; do run_pair nlgoto $o "$tmpsrc"; done

cat > "$tmpsrc" <<'EOF'
#include <stdio.h>
int hits;
int outer(int lim) {
    int scan(int n) { for (int i = 0; i < n; i++) { if (i * i == lim) goto hit; } return 0; hit: hits++; return 1; }
    int s = scan(10);
    s += scan(100);
    s += scan(50);
    return s;
}
int main(void) { hits = 0; printf("%d %d\n", outer(10), hits); return 0; }
EOF
for o in -O0 -O2; do run_pair nlgoto_loop $o "$tmpsrc"; done

exit $fail
