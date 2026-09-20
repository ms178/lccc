#!/usr/bin/env bash
# Emission + semantics gate for constant-array promotion (constarr).
#
#   promote-shape   a 16-byte local array with affine constant init and a
#                   read-only callee — the .LCA_ global must exist, the
#                   16 scalar stores must be gone, and the callee argument
#                   must reference the global (rip-relative leaq).
#   promote-i32     the SAME shape through an i32 reader whose loop is
#                   strength-reduced to pointer induction (a pointer phi)
#                   — the callee-phi proof must fire, not reject.
#   keep-shape      a runtime-written array — no .LCA_ global may appear
#                   for it and the stores must remain.
#   hole-shape      a runtime store at a variable index with a PHI-FREE
#                   read-only callee — the store itself must reject the
#                   promotion (found live: the const-offset-only
#                   classifier was blind to it and the store landed in
#                   .rodata; SIGSEGV).
#   leak-shape      `return a;` — must compile without an ICE and must
#                   not promote (the rewrite would dangle the terminator's
#                   reference to the deleted alloca).
#   align-shapes    int[2] promotes at .align 4 (natural alignment
#                   preserved, not 1); _Alignas(64) at .align 64.
#   escape-shape    address stored to memory — no promotion.
#   write-shape     callee stores through the parameter — no promotion.
#   gap-shape       partial initialization — no promotion (uninit bytes).
#
# Every configuration is also EXECUTED and checked: a gate that only
# verified emission would pass while silently corrupting the object.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
ccc=${CCC:-$repo/target/fastbuild/lccc}
battery=$repo/tests/regression/const_array_promote.c
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fail=0
check_eq() { # <desc> <actual> <expected>
    if [[ "$2" != "$3" ]]; then
        echo "FAIL: $1 -- got '$2', want '$3'" >&2
        fail=1
    fi
}
check_absent() { # <desc> <file> <egrep-pattern>
    if grep -qE "$3" "$2"; then
        echo "FAIL: $1 -- forbidden pattern '$3' matched:" >&2
        grep -nE "$3" "$2" | head -3 >&2
        fail=1
    fi
}
check_present() { # <desc> <file> <egrep-pattern>
    if ! grep -qE "$3" "$2"; then
        echo "FAIL: $1 -- required pattern '$3' not found" >&2
        fail=1
    fi
}

# --- 1. the promote shape -----------------------------------------------------
cat > "$tmp/promote.c" <<'EOF'
int sum_u8(const unsigned char *p, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
int probe(void) {
    unsigned char a[16];
    for (int i = 0; i < 16; i++) a[i] = (unsigned char)(i * 17 + 3);
    return sum_u8(a, 16);
}
int main(void) { return probe() == 1832 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/promote.c" -o "$tmp/promote.s"
check_present "promote: .LCA_ global emitted" "$tmp/promote.s" '^\.LCA_[0-9]+:'
check_present "promote: rodata section" "$tmp/promote.s" '\.section\s+\.rodata'
# The 16 scalar byte stores must be gone from probe.
sed -n '/^probe:/,/^\.cfi_endproc/p' "$tmp/promote.s" > "$tmp/probe-body.s"
n_stores=$(grep -cE 'movb\s+\$[0-9]+,' "$tmp/probe-body.s" || true)
check_eq "promote: byte stores eliminated" "$n_stores" "0"
# The callee argument must be the global's address.
check_present "promote: global address materialized" "$tmp/probe-body.s" 'lea[a-z]*\s+\.LCA_[0-9]+\(%rip\)'
"$ccc" -O2 "$tmp/promote.c" -o "$tmp/promote.bin" && "$tmp/promote.bin"
check_eq "promote: execution" "$?" "0"

# --- 1b. the pointer-induction (phi) reader callee must ALSO promote -----
cat > "$tmp/promote32.c" <<'EOF'
int sum(const int *p, int n) { int s = 0; for (int i = 0; i < n; i++) s += p[i]; return s; }
int probe(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i * 3;
    return sum(a, 4);
}
int main(void) { return probe() == 18 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/promote32.c" -o "$tmp/promote32.s"
check_present "promote-i32: .LCA_ global emitted (phi reader callee)" "$tmp/promote32.s" '^\.LCA_[0-9]+:'
sed -n '/^probe:/,/^\.cfi_endproc/p' "$tmp/promote32.s" > "$tmp/probe32-body.s"
n_stores32=$(grep -cE 'mov[lq]\s+\$-?[0-9]+, [^%]*\(' "$tmp/probe32-body.s" || true)
check_eq "promote-i32: dword stores eliminated" "$n_stores32" "0"
check_present "promote-i32: global address materialized" "$tmp/probe32-body.s" 'lea[a-z]*\s+\.LCA_[0-9]+\(%rip\)'
"$ccc" -O2 "$tmp/promote32.c" -o "$tmp/promote32.bin" && "$tmp/promote32.bin"
check_eq "promote-i32: execution" "$?" "0"

# --- 2. runtime-written array: no promotion -----------------------------------
cat > "$tmp/keep.c" <<'EOF'
int sum(const int *p, int n) { int s = 0; for (int i = 0; i < n; i++) s += p[i]; return s; }
int probe(int k, int v) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    a[k & 3] = v;
    return sum(a, 4);
}
int main(void) { return (probe(0, 40) == 46 && probe(3, 9) == 12) ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/keep.c" -o "$tmp/keep.s"
check_absent "keep: no .LCA_ global" "$tmp/keep.s" '^\.LCA_[0-9]+:'
"$ccc" -O2 "$tmp/keep.c" -o "$tmp/keep.bin" && "$tmp/keep.bin"
check_eq "keep: execution" "$?" "0"

# --- 2b. the variable-index store with a PHI-FREE readonly callee ----------
# The store itself must reject: the callee is provably read-only with no
# loop at all, so nothing else can save the shape. (Shipped-hole repro.)
cat > "$tmp/hole.c" <<'EOF'
int rd(const int *p) { return p[0] + p[1]; }
int probe(int k, int v) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    a[k & 3] = v;
    return rd(a);
}
int main(void) { return (probe(0, 40) == 41 && probe(3, 9) == 1) ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/hole.c" -o "$tmp/hole.s"
check_absent "hole: no .LCA_ global (variable-index store rejects)" "$tmp/hole.s" '^\.LCA_[0-9]+:'
"$ccc" -O2 "$tmp/hole.c" -o "$tmp/hole.bin" && "$tmp/hole.bin"
check_eq "hole: execution" "$?" "0"

# --- 2c. `return a;` — no promotion, no ICE ---------------------------------
cat > "$tmp/leak.c" <<'EOF'
int *leak(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    return a;
}
int main(void) { return leak() != 0 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/leak.c" -o "$tmp/leak.s"
check_absent "leak: no .LCA_ global (terminator escape rejects)" "$tmp/leak.s" '^\.LCA_[0-9]+:'
"$ccc" -O2 "$tmp/leak.c" -o "$tmp/leak.bin" && "$tmp/leak.bin"
check_eq "leak: execution" "$?" "0"

# --- 2d. alignment is preserved on promotion ---------------------------------
# The reader is noinline: with direct loads SCCP folds the whole function
# to a constant and no array survives to Phase 11e — the callee keeps the
# array live so the alignment of the promoted global is observable.
cat > "$tmp/align4.c" <<'EOF'
__attribute__((noinline)) int rd(const int *p) { return p[0] + p[1]; }
int probe(void) {
    int a[2];
    a[0] = 11;
    a[1] = 22;
    return rd(a);
}
int main(void) { return probe() == 33 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/align4.c" -o "$tmp/align4.s"
check_present "align4: .LCA_ global emitted" "$tmp/align4.s" '^\.LCA_[0-9]+:'
check_present "align4: natural alignment preserved" "$tmp/align4.s" '\.align\s+4\s*$'
check_absent "align4: no align-1 placement" "$tmp/align4.s" '\.align\s+1\s*$'
"$ccc" -O2 "$tmp/align4.c" -o "$tmp/align4.bin" && "$tmp/align4.bin"
check_eq "align4: execution" "$?" "0"

cat > "$tmp/align64.c" <<'EOF'
__attribute__((noinline)) int rd(const int *p) { return p[0] + p[1]; }
int probe(void) {
    _Alignas(64) int a[2];
    a[0] = 5;
    a[1] = 6;
    return rd(a);
}
int main(void) { return probe() == 11 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/align64.c" -o "$tmp/align64.s"
check_present "align64: .LCA_ global emitted" "$tmp/align64.s" '^\.LCA_[0-9]+:'
check_present "align64: explicit alignment honored" "$tmp/align64.s" '\.align\s+64\s*$'
"$ccc" -O2 "$tmp/align64.c" -o "$tmp/align64.bin" && "$tmp/align64.bin"
check_eq "align64: execution" "$?" "0"

# --- 3. escaping address: no promotion -----------------------------------------
cat > "$tmp/escape.c" <<'EOF'
static const int *keep;
int probe(void) {
    int a[3];
    for (int i = 0; i < 3; i++) a[i] = i + 5;
    keep = a;
    return keep[0] + keep[1] + keep[2];
}
int main(void) { return probe() == 18 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/escape.c" -o "$tmp/escape.s"
check_absent "escape: no .LCA_ global" "$tmp/escape.s" '^\.LCA_[0-9]+:'
"$ccc" -O2 "$tmp/escape.c" -o "$tmp/escape.bin" && "$tmp/escape.bin"
check_eq "escape: execution" "$?" "0"

# --- 4. write-through callee: no promotion ------------------------------------
cat > "$tmp/write.c" <<'EOF'
int bump(int *p, int n) { for (int i = 0; i < n; i++) p[i] = p[i] + 1; return p[0]; }
int probe(void) {
    int a[4];
    for (int i = 0; i < 4; i++) a[i] = i;
    bump(a, 4);
    return a[0] + a[1] + a[2] + a[3];
}
int main(void) { return probe() == 10 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/write.c" -o "$tmp/write.s"
check_absent "write: no .LCA_ global" "$tmp/write.s" '^\.LCA_[0-9]+:'
"$ccc" -O2 "$tmp/write.c" -o "$tmp/write.bin" && "$tmp/write.bin"
check_eq "write: execution" "$?" "0"

# --- 5. gap initialization: no promotion ---------------------------------------
cat > "$tmp/gap.c" <<'EOF'
int probe(void) {
    unsigned char a[8];
    a[0] = 1; a[2] = 3; a[4] = 5; a[6] = 7;
    return a[0] + a[2] + a[4] + a[6];
}
int main(void) { return probe() == 16 ? 0 : 1; }
EOF
"$ccc" -O2 -S "$tmp/gap.c" -o "$tmp/gap.s"
check_absent "gap: no .LCA_ global" "$tmp/gap.s" '^\.LCA_[0-9]+:'
"$ccc" -O2 "$tmp/gap.c" -o "$tmp/gap.bin" && "$tmp/gap.bin"
check_eq "gap: execution" "$?" "0"

# --- 6. the full battery, differential across optimization levels --------------
for opt in -O0 -O1 -O2 -O3 -Os; do
    "$ccc" "$opt" "$battery" -o "$tmp/battery$opt.bin"
    "$tmp/battery$opt.bin" > "$tmp/battery$opt.out" 2>&1
    check_eq "battery $opt exit" "$?" "0"
done
for opt in -O1 -O2 -O3 -Os; do
    if ! diff -q "$tmp/battery-O0.out" "$tmp/battery$opt.out" >/dev/null; then
        echo "FAIL: battery $opt output differs from -O0:" >&2
        diff "$tmp/battery-O0.out" "$tmp/battery$opt.out" | head -5 >&2
        fail=1
    fi
done

exit $fail
