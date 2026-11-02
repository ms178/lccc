#!/usr/bin/env bash
# Duplicate-definition rule (C11 6.9p5) and the one deliberate exception.
#
# lccc silently accepted two definitions of one function and lowered both. The
# object then carried two bodies for one symbol (and two labels), which either
# duplicated the code or made the linker pick one at random. Both oracles
# diagnose it.
#
# The exception is GNU89 inline semantics: an `extern inline` body is a
# definition that does *not* provide the external definition (glibc's libio
# idiom), so a plain definition of the same function may follow it -- in either
# order -- and GCC accepts that translation unit. See
# `tests/regression/gnu89_extern_inline_redef_alias.c` for the full glibc shape.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-frontend-diag.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
status=0

fail() {
    echo "FAIL: $1" >&2
    shift
    for line in "$@"; do echo "    $line" >&2; done
    status=1
}

expect_error() {
    local what=$1 file=$2 want=$3
    shift 3
    if "$CCC" -std=gnu11 -fsyntax-only "$@" "$file" >"$tmp/out" 2>&1; then
        fail "$what: accepted but both oracles reject it" "$(cat "$tmp/out")"
        return
    fi
    if ! grep -qF "$want" "$tmp/out"; then
        fail "$what: rejected without the expected diagnostic" "want: $want" "got: $(cat "$tmp/out")"
    fi
}

expect_ok() {
    local what=$1 file=$2
    shift 2
    if ! "$CCC" -std=gnu11 -fsyntax-only "$@" "$file" >"$tmp/out" 2>&1; then
        fail "$what: rejected but both oracles accept it" "$(cat "$tmp/out")"
    fi
}

# --- two external definitions ---------------------------------------------
cat >"$tmp/dup.c" <<'C'
int f(void) { return 1; }
int f(void) { return 2; }
int main(void) { return f(); }
C
expect_error "duplicate external definition" "$tmp/dup.c" "redefinition of 'f'"

# --- two internal definitions ---------------------------------------------
cat >"$tmp/dup_static.c" <<'C'
static int g(void) { return 1; }
static int g(void) { return 2; }
int main(void) { return g(); }
C
expect_error "duplicate internal definition" "$tmp/dup_static.c" "redefinition of 'g'"

# --- prototype then definition is legal -----------------------------------
cat >"$tmp/decl_def.c" <<'C'
int h(void);
int h(void) { return 1; }
int main(void) { return h() - 1; }
C
expect_ok "declaration followed by its definition" "$tmp/decl_def.c"

# --- GNU89 `extern inline` husk then plain definition ---------------------
cat >"$tmp/husk.c" <<'C'
extern __inline int f(int x) { return x + 1; }
int f(int x) { return x + 2; }
int main(void) { return f(1) - 3; }
C
expect_ok "gnu89 extern-inline husk followed by the real definition" "$tmp/husk.c" -fgnu89-inline
expect_error "the same pair without gnu89 semantics" "$tmp/husk.c" "redefinition of 'f'"

# --- the attribute spelling is mode independent ---------------------------
cat >"$tmp/husk_attr.c" <<'C'
extern __inline __attribute__((__gnu_inline__)) int g(int x) { return x + 10; }
int g(int x) { return x + 20; }
int main(void) { return g(1) - 21; }
C
expect_ok "gnu_inline attribute husk" "$tmp/husk_attr.c"

# --- the glibc shape end to end ------------------------------------------
"$CCC" -std=gnu11 -fgnu89-inline tests/regression/gnu89_extern_inline_redef_alias.c -o "$tmp/husk_bin" \
    >"$tmp/out" 2>&1 || fail "glibc husk+alias translation unit did not build" "$(cat "$tmp/out")"
if [ -x "$tmp/husk_bin" ]; then
    out=$("$tmp/husk_bin") || fail "glibc husk+alias binary failed: $out"
    [ "$out" = "gnu89-redef-alias:ok" ] || fail "glibc husk+alias binary printed '$out'"
fi

[ "$status" -eq 0 ] && echo "frontend-diagnostics: ok"
exit "$status"
