#!/usr/bin/env bash
# Operand constraints: `*`, `.` and `->`, and the `void` value rules.
#
# Every expectation was recorded from gcc-16.2.0 (`-std=gnu17 -fsyntax-only`,
# and again under gnu90/gnu23) before it was encoded here, and gcc-16 is
# re-run as an oracle whenever it is available, so this file cannot rot into
# self-referential expectations.
#
# The contract:
#
#   * `*e` (C23 6.5.3.2p2) on anything that is not a pointer is
#     "invalid type argument of unary '*' (have 'T')", quoting gcc's
#     type_to_string spelling of the *integer-promoted* operand type
#     (`'int'` for a `char`/`_Bool`, `'unsigned int'` for an enumeration,
#     `'long unsigned int'`, `'complex double'`, `'__vector(4) int'`);
#     `*` of an array, a function designator and a pointer stays legal;
#   * `*p` on a `void *` is gcc's GNU extension with the default-on, unkinded
#     warning `dereferencing 'void *' pointer` at the `*`; the resulting
#     lvalue has type `void`, so using its value is "void value not ignored as
#     it ought to be" and writing to it is "invalid use of void expression";
#   * `->` needs a pointer to a record: a non-pointer operand is "invalid type
#     argument of '->' (have 'T')", a pointer to a non-record (or an array or
#     function designator) is "request for member 'x' in something not a
#     structure or union", and `void *` warns first;
#   * `.` needs a record: a pointer operand is "'p' is a pointer; did you mean
#     to use '->'?", an array, a function designator or a scalar is the same
#     "request for member" error, and a member of an incomplete record is
#     "invalid use of undefined type 'struct T'";
#   * an incompatible record conversion is worded per context (initializing /
#     assigning / returning / argument N of 'f'), two distinct records in `?:`
#     are "type mismatch in conditional expression", two incompatible pointer
#     arms are gcc's `-Wincompatible-pointer-types` (error from C99 on,
#     warning in gnu90), and a `void` value keeps gcc's two wordings: the
#     "not ignored" form for initialization/`return`, "invalid use" for
#     assignment/argument passing;
#   * `-w` silences the unkinded `void *` warning, `-Werror` promotes it to
#     "error: ... [-Werror]", `-Werror -Wno-error` demotes it back, exactly as
#     gcc-16.2 does;
#   * the legal forms compile, lower and run (a gate that only asserted
#     rejections would pass with `*`, `.`, `->` deleted);
#   * the `dereferencing 'void *' pointer` warning is about the *evaluated*
#     indirection, so it is silent inside `sizeof`, `_Countof`,
#     `__alignof__(expr)` and a `_Generic` selector while those operators'
#     own errors still fire;
#   * `_Countof` names its operand type (`to type 'void'`, `'int (*)[3]'`,
#     `'int(void)'`) and carets the operand's parenthesis.
#
# Two measured gcc behaviours are deliberately NOT pinned to gcc's column:
#
#   * gcc carets "type mismatch in conditional expression" and "pointer type
#     mismatch in conditional expression" at the `:`, while lccc's AST records
#     the conditional at the start of its first operand (it keeps no `:` span).
#     The message, severity and `-W` classification match; only the column
#     differs, so those cases are asserted by line/severity/message.
#   * gcc appends `{aka 'int'}` for a typedef'd operand (`'T' {aka 'int'}`).
#     The inferred type no longer carries the spelling a declaration used, so
#     lccc prints the canonical type (`'int'`); asserted explicitly below.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
if [[ ! -x $CCC ]]; then
    echo "SKIP: $CCC not built" >&2
    exit 1
fi
CCC=$(cd "$(dirname -- "$CCC")" && pwd)/$(basename -- "$CCC")
GCC=${GCC:-gcc-16}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-operand-diags.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"

fail() { echo "FAIL: $*" >&2; exit 1; }

diag_lines() {
    grep -E '^[^:]+:[0-9]+:[0-9]+: (error|warning):' "$1" || true
}

# run_cc <compiler> <file> <std> [flags...] -> exit code, stderr in <file>.<tag>.err
run_cc() {
    local compiler=$1 file=$2 std=$3
    shift 3
    set +e
    "$compiler" -std="$std" -fsyntax-only -c "$@" "$file" 2>"$file.$std.err"
    local rc=$?
    set -e
    echo "$rc"
}

# expect_diags <file> <std> <expected-rc>; expected lines on stdin.
expect_diags() {
    local file=$1 std=$2 want=$3 expected got
    expected=$(cat)
    got=$(run_cc "$CCC" "$file" "$std")
    [[ $got == "$want" ]] || { cat "$file.$std.err" >&2; fail "$file@$std: exit $got, contract wants $want"; }
    diff -u <(printf '%s\n' "$expected") <(diag_lines "$file.$std.err") \
        || { cat "$file.$std.err" >&2; fail "$file@$std: diagnostic lines differ from the gcc-16.2 record"; }
    ! grep -qE 'panicked at|internal compiler error|RUST_BACKTRACE' "$file.$std.err" \
        || { cat "$file.$std.err" >&2; fail "$file@$std: ICE instead of a diagnostic"; }
    # The oracle must agree character for character, so the record cannot be
    # lccc's output in disguise.
    if [[ -n ${oracle:-} ]]; then
        run_cc "$GCC" "$file" "$std" >/dev/null
        diff -u <(printf '%s\n' "$expected") <(diag_lines "$file.$std.err") \
            || { cat "$file.$std.err" >&2; fail "$file@$std: gcc oracle drifted from the encoded record"; }
    fi
}

# expect_message_only <file> <std> <expected-rc> <regex> : a documented
# deviation, asserted by message and severity only.
expect_message_only() {
    local file=$1 std=$2 want=$3 pattern=$4 got
    got=$(run_cc "$CCC" "$file" "$std")
    [[ $got == "$want" ]] || { cat "$file.$std.err" >&2; fail "$file@$std: exit $got, contract wants $want"; }
    grep -qE "$pattern" "$file.$std.err" \
        || { cat "$file.$std.err" >&2; fail "$file@$std: missing $pattern"; }
}

oracle=
if command -v "$GCC" >/dev/null 2>&1; then
    ver=$("$GCC" -dumpfullversion 2>/dev/null || "$GCC" -dumpversion)
    if [[ $ver == 16.* ]]; then oracle=1; else echo "note: $GCC is $ver, oracle cross-check skipped" >&2; fi
else
    echo "note: $GCC not found, oracle cross-check skipped" >&2
fi

cat > operand_deref.c <<'C'
void f_int(void){ int x; *x; }
void f_bool(void){ _Bool b; *b; }
void f_char(void){ char c; *c; }
void f_short(void){ short s; *s; }
void f_long(void){ long l; *l; }
void f_ulong(void){ unsigned long l; *l; }
void f_ullong(void){ unsigned long long l; *l; }
void f_float(void){ float x; *x; }
void f_double(void){ double d; *d; }
void f_long_double(void){ long double d; *d; }
void f_float16(void){ _Float16 h; *h; }
void f_complex(void){ _Complex double c; *c; }
enum E { A };
void f_enum(void){ enum E e; *e; }
struct S { int a; };
union U { int a; };
void f_struct(void){ struct S s; *s; }
void f_union(void){ union U u; *u; }
void f_anon(void){ struct { int a; } s; *s; }
void f_const(void){ const int x = 0; *x; }
void f_vector(void){ __attribute__((vector_size(16))) int v; *v; }
C
cat > operand_void_ptr.c <<'C'
void g(void);
void w_unused(void *p){ *p; }
void w_value(void *p){ int y = *p; (void)y; }
void w_assign(void *p){ *p = 1; }
void w_postinc(void *p){ (*p)++; }
void w_compound(void *p){ *p += 1; }
void w_preinc(void *p){ ++*p; }
void w_negate(void *p){ int x = !*p; (void)x; }
void w_call(void){ *g(); }
void w_subscript(void *p){ p[0]; }
void w_subscript_reversed(void *p){ 0[p]; }
void w_subscript_store(void *p){ p[0] = 1; }
void w_subscript_value(void *p){ int y = p[0]; (void)y; }
C
cat > operand_member.c <<'C'
struct S { int a; };
union U { int a; };
enum E { A };
struct S *f(void);
int g(void);
void a_int(void){ int x; x->a; }
void a_double(void){ double d; d->x; }
void a_enum(void){ enum E e; e->x; }
void a_union(void){ union U u; u->a; }
void a_struct(void){ struct S s; s->a; }
void a_int_ptr(void){ int *p = 0; p->a; }
void a_void_ptr(void *p){ p->a; }
void a_array(void){ int a[3]; a->x; }
void a_func(void){ g->x; }
void a_func_ptr(void){ int (*fp)(void) = 0; fp->x; }
void a_missing(void){ struct S *p = 0; p->b; }
void a_ptr_to_ptr(struct S **pp){ pp->a; }
void a_int_ptr_to_ptr(int **pp){ (void)pp->a; }
void a_triple(struct S ***ppp){ (void)ppp->a; }
void a_array_of_ptrs(struct S *a[3]){ (void)a->a; }
void d_int(void){ int x; x.a; }
void d_double(void){ double d; d.x; }
void d_int_ptr(int *p){ (void)p.a; }
void d_void_ptr(void *p){ (void)p.a; }
void d_array(void){ int a[3]; a.x; }
void d_func(void){ g.x; }
void d_pointer(void){ struct S *p = 0; p.a; }
void d_call(void){ f().a; }
void d_ptr_to_ptr(struct S **pp){ (void)pp.a; }
void d_missing(void){ struct S s; s.b; }
void d_incomplete(void){ struct T *p = 0; p->a; }
C
cat > operand_convert.c <<'C'
struct S { int a; };
struct T { int b; };
union U { int a; };
void g(void);
void h(int);
void init_record_scalar(struct S s){ int x = s; (void)x; }
void assign_record_scalar(struct S s){ int x; x = s; }
int ret_record_scalar(struct S s){ return s; }
void arg_record_scalar(struct S s){ h(s); }
void init_void_value(void){ int x = g(); (void)x; }
void assign_void_value(void){ int x; x = g(); (void)x; }
int ret_void_value(void){ return g(); }
void arg_void_value(void){ h(g()); }
void bitnot_void(void){ ~g(); }
void switch_record(struct S s){ switch (s) {} }
void switch_union(union U u){ switch (u) {} }
C
cat > operand_conditional.c <<'C'
struct S { int a; };
struct T { int b; };
union U { int a; };
void c_record_mismatch(struct S s, struct T t){ (void)(1 ? s : t); }
void c_record_scalar(struct S s){ (void)(1 ? s : 0); }
void c_union_struct(union U u, struct S s){ (void)(1 ? u : s); }
void c_record_record_same(struct S s, struct S r){ (void)(1 ? s : r); }
void c_pointer_mismatch(struct S *p, int *q){ (void)(1 ? p : q); }
C
cat > legal.c <<'C'
struct S { int a; };
struct T { struct S s; };
int h(void);
void l_pointer(int *p){ *p = 1; }
void l_array(void){ int a[3]; *a = 1; }
void l_func(void){ (void)*h; }
void l_func_ptr(void){ int (*fp)(void) = 0; (void)*fp; }
void l_member(struct S *p){ p->a = 1; }
void l_member_value(void){ struct S s; s.a = 1; }
void l_member_nested(struct T *t){ t->s.a = 1; }
void l_subscript(int *p){ p[0] = 1; }
void l_subscript_reversed(int *p){ 0[p] = 1; }
void l_void_ptr(int *p){ void *v = p; (void)v; }
void l_conditional(int c, struct S s, struct S r){ (void)(c ? s : r); }
int main(void){ return 0; }
C

# ------------------------------------------------ `*` on a non-pointer (18) --
for std in gnu90 gnu17 gnu23; do
expect_diags operand_deref.c "$std" 1 <<'EOF'
operand_deref.c:1:26: error: invalid type argument of unary '*' (have 'int')
operand_deref.c:2:29: error: invalid type argument of unary '*' (have 'int')
operand_deref.c:3:28: error: invalid type argument of unary '*' (have 'int')
operand_deref.c:4:30: error: invalid type argument of unary '*' (have 'int')
operand_deref.c:5:28: error: invalid type argument of unary '*' (have 'long int')
operand_deref.c:6:38: error: invalid type argument of unary '*' (have 'long unsigned int')
operand_deref.c:7:44: error: invalid type argument of unary '*' (have 'long long unsigned int')
operand_deref.c:8:30: error: invalid type argument of unary '*' (have 'float')
operand_deref.c:9:32: error: invalid type argument of unary '*' (have 'double')
operand_deref.c:10:42: error: invalid type argument of unary '*' (have 'long double')
operand_deref.c:11:35: error: invalid type argument of unary '*' (have '_Float16')
operand_deref.c:12:42: error: invalid type argument of unary '*' (have 'complex double')
operand_deref.c:14:30: error: invalid type argument of unary '*' (have 'unsigned int')
operand_deref.c:17:34: error: invalid type argument of unary '*' (have 'struct S')
operand_deref.c:18:32: error: invalid type argument of unary '*' (have 'union U')
operand_deref.c:19:41: error: invalid type argument of unary '*' (have 'struct <anonymous>')
operand_deref.c:20:38: error: invalid type argument of unary '*' (have 'int')
operand_deref.c:21:62: error: invalid type argument of unary '*' (have '__vector(4) int')
EOF
done

# ------------------------------------- `void *` indirection and subscripts ---
for std in gnu90 gnu17 gnu23; do
expect_diags operand_void_ptr.c "$std" 1 <<'EOF'
operand_void_ptr.c:2:25: warning: dereferencing 'void *' pointer
operand_void_ptr.c:3:32: warning: dereferencing 'void *' pointer
operand_void_ptr.c:3:32: error: void value not ignored as it ought to be
operand_void_ptr.c:4:25: warning: dereferencing 'void *' pointer
operand_void_ptr.c:4:28: error: invalid use of void expression
operand_void_ptr.c:5:27: warning: dereferencing 'void *' pointer
operand_void_ptr.c:5:30: error: invalid use of void expression
operand_void_ptr.c:6:27: warning: dereferencing 'void *' pointer
operand_void_ptr.c:6:30: error: invalid use of void expression
operand_void_ptr.c:7:27: warning: dereferencing 'void *' pointer
operand_void_ptr.c:7:25: error: invalid use of void expression
operand_void_ptr.c:8:34: warning: dereferencing 'void *' pointer
operand_void_ptr.c:8:33: error: invalid use of void expression
operand_void_ptr.c:9:21: error: void value not ignored as it ought to be
operand_void_ptr.c:10:29: warning: dereferencing 'void *' pointer
operand_void_ptr.c:11:38: warning: dereferencing 'void *' pointer
operand_void_ptr.c:12:35: warning: dereferencing 'void *' pointer
operand_void_ptr.c:12:39: error: invalid use of void expression
operand_void_ptr.c:13:43: warning: dereferencing 'void *' pointer
operand_void_ptr.c:13:42: error: void value not ignored as it ought to be
EOF
done

# -------------------------------------------------- `.` and `->` operands ----
for std in gnu90 gnu17 gnu23; do
expect_diags operand_member.c "$std" 1 <<'EOF'
operand_member.c:6:27: error: invalid type argument of '->' (have 'int')
operand_member.c:7:33: error: invalid type argument of '->' (have 'double')
operand_member.c:8:31: error: invalid type argument of '->' (have 'unsigned int')
operand_member.c:9:33: error: invalid type argument of '->' (have 'union U')
operand_member.c:10:35: error: invalid type argument of '->' (have 'struct S')
operand_member.c:11:36: error: request for member 'a' in something not a structure or union
operand_member.c:12:28: warning: dereferencing 'void *' pointer
operand_member.c:12:28: error: request for member 'a' in something not a structure or union
operand_member.c:13:32: error: request for member 'x' in something not a structure or union
operand_member.c:14:21: error: request for member 'x' in something not a structure or union
operand_member.c:15:47: error: request for member 'x' in something not a structure or union
operand_member.c:16:41: error: 'struct S' has no member named 'b'
operand_member.c:17:37: error: 'pp' is a pointer to pointer; did you mean to dereference it before applying '->' to it?
operand_member.c:18:42: error: request for member 'a' in something not a structure or union
operand_member.c:19:42: error: request for member 'a' in something not a structure or union
operand_member.c:20:46: error: 'a' is a pointer to pointer; did you mean to dereference it before applying '->' to it?
operand_member.c:21:27: error: request for member 'a' in something not a structure or union
operand_member.c:22:33: error: request for member 'x' in something not a structure or union
operand_member.c:23:32: error: request for member 'a' in something not a structure or union
operand_member.c:24:34: error: request for member 'a' in something not a structure or union
operand_member.c:25:32: error: request for member 'x' in something not a structure or union
operand_member.c:26:21: error: request for member 'x' in something not a structure or union
operand_member.c:27:41: error: 'p' is a pointer; did you mean to use '->'?
operand_member.c:28:23: error: 'f()' is a pointer; did you mean to use '->'?
operand_member.c:29:43: error: request for member 'a' in something not a structure or union
operand_member.c:30:36: error: 'struct S' has no member named 'b'
operand_member.c:31:44: error: invalid use of undefined type 'struct T'
EOF
done

# --------------------- record/void conversions, switch controlling expr ------
for std in gnu90 gnu17 gnu23; do
expect_diags operand_convert.c "$std" 1 <<'EOF'
operand_convert.c:6:46: error: incompatible types when initializing type 'int' using type 'struct S'
operand_convert.c:7:51: error: incompatible types when assigning to type 'int' from type 'struct S'
operand_convert.c:8:43: error: incompatible types when returning type 'struct S' but 'int' was expected
operand_convert.c:9:39: error: incompatible type for argument 1 of 'h'
operand_convert.c:10:37: error: void value not ignored as it ought to be
operand_convert.c:11:40: error: invalid use of void expression
operand_convert.c:12:34: error: void value not ignored as it ought to be
operand_convert.c:13:30: error: invalid use of void expression
operand_convert.c:14:25: error: invalid use of void expression
operand_convert.c:15:41: error: switch quantity not an integer
operand_convert.c:16:39: error: switch quantity not an integer
EOF
done

# --------------- unevaluated operands stay quiet; their own errors do not ----
cat > operand_unevaluated.c <<'C'
struct S { int a; };
void a(void *p){ (void)sizeof(*p); }
void b(void *p){ (void)sizeof(p[0]); }
void c(void *p){ (void)__alignof__(p[0]); }
void d(void *p){ (void)_Generic(*p, int: 1); }
void e(void *p){ (void)(p[0], 1); }
void f(void *p){ *p; }
void g(void *p){ (void)*p; }
void h(struct S *sp, void *p){ (void)sizeof(p->a); (void)sp->a; }
void i(void *p){ (void)sizeof p[0]; }
C

# -------------------------- `_Countof` names the operand type and location ----
cat > operand_countof.c <<'C'
struct S { int a; };
struct T;
void a(int x){ (void)_Countof(x); }
void b(void *p){ (void)_Countof(p); }
void c(void *p){ (void)_Countof(*p); }
void d(void *p){ (void)_Countof(p[0]); }
void e(struct S *sp){ (void)_Countof(*sp); }
void f(struct T *tp){ (void)_Countof(*tp); }
void g(int (*pa)[3]){ (void)_Countof(*pa); }
void h(int (*fp)(void)){ (void)_Countof(*fp); }
void i(void){ int x[3]; (void)_Countof(x + 0); }
void j(void){ int x[3]; (void)_Countof(&x); }
void k(void){ int x[3], y[3]; (void)_Countof((x, y)); }
void l(void){ (void)_Countof(  ( 1 ) ); }
C

for std in gnu17 gnu23; do
expect_diags operand_unevaluated.c "$std" 1 <<'EOF'
operand_unevaluated.c:5:33: error: '_Generic' selector of type 'void' is not compatible with any association
operand_unevaluated.c:6:26: warning: dereferencing 'void *' pointer
operand_unevaluated.c:7:18: warning: dereferencing 'void *' pointer
operand_unevaluated.c:8:24: warning: dereferencing 'void *' pointer
operand_unevaluated.c:9:46: error: request for member 'a' in something not a structure or union
EOF
done
expect_diags operand_countof.c gnu23 1 <<'EOF'
operand_countof.c:3:30: error: invalid application of '_Countof' to type 'int'
operand_countof.c:4:32: error: invalid application of '_Countof' to type 'void *'
operand_countof.c:5:32: error: invalid application of '_Countof' to type 'void'
operand_countof.c:6:32: error: invalid application of '_Countof' to type 'void'
operand_countof.c:7:37: error: invalid application of '_Countof' to type 'struct S'
operand_countof.c:8:37: error: invalid application of '_Countof' to type 'struct T'
operand_countof.c:10:40: error: invalid application of '_Countof' to type 'int(void)'
operand_countof.c:11:39: error: invalid application of '_Countof' to type 'int *'
operand_countof.c:12:39: error: invalid application of '_Countof' to type 'int (*)[3]'
operand_countof.c:13:45: error: invalid application of '_Countof' to type 'int *'
operand_countof.c:14:29: error: invalid application of '_Countof' to type 'int'
EOF

# --------------------- pointers to an undefined record type (C23 6.5.3.2p4) ---
# gcc-16.2 accepts `&*p` -- it never computes the value -- and rejects every
# other use with "invalid use of undefined type", including a discarded
# statement and pointer arithmetic. `sizeof`/`_Alignof` keep their own
# "invalid application of ..." wording, caretted at the `(` / the operator,
# and suppress the deref error (`sizeof(*p)` reports once). Measured on
# gcc-16.2.0 with -std=gnu23.
cat > operand_incomplete.c <<'C'
struct f;
struct f *p;
void a(void){ (void)*p; }
void b(void){ *p; }
void c(void){ (void)&*p; }
void d(void){ (void)sizeof(*p); }
void e(void){ (void)sizeof(p[0]); }
void g(void){ (void)__alignof__(*p); }
void h(void){ (void)sizeof(struct f); }
void i(void){ (void)_Alignof(struct f); }
union u;
void j(void){ (void)sizeof(union u); }
C

# Pointer arithmetic needs the size of the pointed-to type, so ++/--, p+n, p-n,
# p+=n and p-p on a pointer to an undefined record are all ill-formed. gcc-16.2
# words them three ways: "increment/decrement of pointer to an incomplete type
# 'struct S'" for the steps, "invalid use of undefined type 'struct S'" for the
# mixed arithmetic and "arithmetic on pointer to an incomplete type" (no type
# name) for the difference of two such pointers.
cat > operand_incomplete_arith.c <<'C'
struct S;
struct S *p;
void a(void){ p++; }
void b(void){ ++p; }
void c(void){ p--; }
void d(void){ --p; }
void e(void){ p += 1; }
void f(void){ p -= 1; }
void g(void){ p = p + 1; }
void h(void){ p = p - 1; }
void i(void){ (void)(p - p); }
void j(void){ int n = 1; p += n; }
void k(void){ (void)(1 + p); }
void l(void){ (void)(p + 1); }
void m(void){ (void)(&p[1] - &p[0]); }
C

expect_diags operand_incomplete.c gnu23 1 <<'EOF'
operand_incomplete.c:3:21: error: invalid use of undefined type 'struct f'
operand_incomplete.c:4:15: error: invalid use of undefined type 'struct f'
operand_incomplete.c:6:27: error: invalid application of 'sizeof' to incomplete type 'struct f'
operand_incomplete.c:7:29: error: invalid use of undefined type 'struct f'
operand_incomplete.c:7:27: error: invalid application of 'sizeof' to incomplete type 'struct f'
operand_incomplete.c:8:21: error: invalid application of '__alignof__' to incomplete type 'struct f'
operand_incomplete.c:9:28: error: invalid application of 'sizeof' to incomplete type 'struct f'
operand_incomplete.c:10:30: error: invalid application of '__alignof__' to incomplete type 'struct f'
operand_incomplete.c:12:28: error: invalid application of 'sizeof' to incomplete type 'union u'
EOF
expect_diags operand_incomplete_arith.c gnu17 1 <<'EOF'
operand_incomplete_arith.c:3:16: error: increment of pointer to an incomplete type 'struct S'
operand_incomplete_arith.c:4:15: error: increment of pointer to an incomplete type 'struct S'
operand_incomplete_arith.c:5:16: error: decrement of pointer to an incomplete type 'struct S'
operand_incomplete_arith.c:6:15: error: decrement of pointer to an incomplete type 'struct S'
operand_incomplete_arith.c:7:17: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:8:17: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:9:21: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:10:21: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:11:24: error: arithmetic on pointer to an incomplete type
operand_incomplete_arith.c:12:28: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:13:24: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:14:24: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:15:24: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:15:32: error: invalid use of undefined type 'struct S'
operand_incomplete_arith.c:15:28: error: arithmetic on pointer to an incomplete type
EOF

# Only the dereference directly underneath `&` is exempt. In particular, the
# outer `&*` must not suppress an independent `*q` nested in its operand; the
# following direct form remains valid in the same translation unit.
cat > operand_address_of_nesting.c <<'C'
struct S;
void nested(struct S *p, struct S *q) {
    &*((*q, p));
}
void direct(struct S *p) {
    (void)&*p;
}
C
for std in gnu90 gnu17 gnu23; do
    expect_diags operand_address_of_nesting.c "$std" 1 <<'EOF'
operand_address_of_nesting.c:3:9: error: invalid use of undefined type 'struct S'
EOF
done

# --------------------------- `?:` with two distinct records: message parity --
# gcc carets the `:`, which lccc's AST does not record (see the header).
for std in gnu90 gnu17 gnu23; do
    expect_message_only operand_conditional.c "$std" 1 \
        "operand_conditional\.c:4:.*error: type mismatch in conditional expression"
    expect_message_only operand_conditional.c "$std" 1 \
        "operand_conditional\.c:5:.*error: type mismatch in conditional expression"
    expect_message_only operand_conditional.c "$std" 1 \
        "operand_conditional\.c:6:.*error: type mismatch in conditional expression"
done
# ... and the valid pair stays accepted, as does a `struct S *` against a
# `void *` arm.
grep -qE 'operand_conditional\.c:(7|9):' operand_conditional.c.gnu17.err \
    && { cat operand_conditional.c.gnu17.err >&2; fail "?: accepted shapes were rejected"; }

# ------------------------------------------- a typedef name in the message ----
cat > typedef_spelling.c <<'C'
typedef int T;
void f(void){ T x; *x; }
typedef struct { int a; } A;
void g(void){ A a; a->b; }
C
for std in gnu90 gnu17 gnu23; do
    run_cc "$CCC" typedef_spelling.c "$std" >/dev/null
    grep -qF "error: invalid type argument of unary '*' (have 'int')" typedef_spelling.c.$std.err \
        || { cat typedef_spelling.c.$std.err >&2; fail "typedef_spelling.c@$std: canonical spelling missing"; }
    grep -qF "error: invalid type argument of '->' (have 'struct <anonymous>')" typedef_spelling.c.$std.err \
        || { cat typedef_spelling.c.$std.err >&2; fail "typedef_spelling.c@$std: anonymous record spelling missing"; }
done

# --------------------------------- unkinded warning: -w / -Werror parity ------
cat > severity.c <<'C'
void f(void *p){ *p; }
C
for std in gnu17 gnu23; do
    rc=$(run_cc "$CCC" severity.c "$std")
    [[ $rc == 0 ]] || { cat severity.c.$std.err >&2; fail "severity.c@$std: the GNU extension must be accepted (exit $rc)"; }
    grep -qF "warning: dereferencing 'void *' pointer" severity.c.$std.err \
        || { cat severity.c.$std.err >&2; fail "severity.c@$std: missing the default-on warning"; }
    rc=$(run_cc "$CCC" severity.c "$std" -w)
    [[ $rc == 0 ]] || fail "severity.c@$std -w: exit $rc"
    ! grep -qE '^[^:]+:[0-9]+:[0-9]+: (warning|error):' severity.c.$std.err \
        || { cat severity.c.$std.err >&2; fail "severity.c@$std -w: -w must silence it"; }
    rc=$(run_cc "$CCC" severity.c "$std" -Werror)
    [[ $rc == 1 ]] || fail "severity.c@$std -Werror: exit $rc, gcc promotes it"
    grep -qF "error: dereferencing 'void *' pointer [-Werror]" severity.c.$std.err \
        || { cat severity.c.$std.err >&2; fail "severity.c@$std -Werror: label missing"; }
    rc=$(run_cc "$CCC" severity.c "$std" -Werror -Wno-error)
    [[ $rc == 0 ]] || fail "severity.c@$std -Werror -Wno-error: exit $rc"
done

# --------------- two incompatible pointer arms: -W flag classification -------
cat > pointer_arms.c <<'C'
struct S { int a; };
void f(struct S *p, int *q){ (void)(1 ? p : q); }
void g(struct S *p, void *q){ (void)(1 ? p : q); }
void h(int *p, int *q){ (void)(1 ? p : q); }
C
for std in gnu90 gnu17; do
    # The whole verdict is `-Wincompatible-pointer-types`: a warning in gnu90,
    # an error from C99 on, silenced by -Wno-, demoted by -Wno-error=,
    # demoted by -fpermissive (all measured on gcc-16.2).
    want_rc=1
    if [[ $std == gnu90 ]]; then want_rc=0; fi
    rc=$(run_cc "$CCC" pointer_arms.c "$std")
    [[ $rc == $want_rc ]] || fail "pointer_arms.c@$std: exit $rc, expected the default verdict $want_rc"
    want_sev=error
    if [[ $std == gnu90 ]]; then want_sev=warning; fi
    grep -qF "$want_sev: pointer type mismatch in conditional expression [-Wincompatible-pointer-types]" \
        pointer_arms.c.$std.err \
        || { cat pointer_arms.c.$std.err >&2; fail "pointer_arms.c@$std: default severity is not $want_sev"; }
    lines=$(diag_lines pointer_arms.c.$std.err | wc -l)
    [[ $lines == 1 ]] || { cat pointer_arms.c.$std.err >&2; fail "pointer_arms.c@$std: $lines diagnostics, only line 2 is a violation"; }
    for flags in '-Wno-incompatible-pointer-types' '-Wno-error=incompatible-pointer-types' '-fpermissive'; do
        rc=$(run_cc "$CCC" pointer_arms.c "$std" $flags)
        case $flags in
            -Wno-incompatible-pointer-types)
                [[ $rc == 0 ]] || fail "pointer_arms.c@$std $flags: exit $rc"
                [[ -z $(diag_lines pointer_arms.c.$std.err) ]] \
                    || { cat pointer_arms.c.$std.err >&2; fail "pointer_arms.c@$std $flags: must silence"; } ;;
            *)
                [[ $rc == 0 ]] || fail "pointer_arms.c@$std $flags: exit $rc, must demote to a warning"
                grep -qF "warning: pointer type mismatch in conditional expression [-Wincompatible-pointer-types]" \
                    pointer_arms.c.$std.err \
                    || { cat pointer_arms.c.$std.err >&2; fail "pointer_arms.c@$std $flags: demotion label"; } ;;
        esac
        if [[ -n ${oracle:-} ]]; then
            orc=$(run_cc "$GCC" pointer_arms.c "$std" $flags)
            [[ $orc == $rc ]] || fail "pointer_arms.c@$std $flags: gcc oracle exits $orc, lccc $rc"
        fi
    done
done

# --------------------------------------------- the legal shapes still work ---
for std in gnu90 gnu17 gnu23; do
    rc=$(run_cc "$CCC" legal.c "$std")
    [[ $rc == 0 ]] || { cat legal.c.$std.err >&2; fail "legal.c@$std: exit $rc, legal operands rejected"; }
    [[ -z $(diag_lines legal.c.$std.err) ]] || { cat legal.c.$std.err >&2; fail "legal.c@$std: legal operands diagnosed"; }
    if [[ -n ${oracle:-} ]]; then
        rc=$(run_cc "$GCC" legal.c "$std")
        [[ $rc == 0 ]] || fail "legal.c@$std: gcc oracle rejects it too (record drift)"
    fi
done

# ... and a legal program that uses them executes correctly.
cat > legal_exec.c <<'C'
#include <stdio.h>
struct S { int a; int b; };
int main(void)
{
    int v = 7;
    int *p = &v;
    struct S s = { 1, 2 };
    struct S *q = &s;
    int arr[3] = { 3, 4, 5 };
    void *vp = &v;
    int *back = (int *)vp;
    *p += 1;
    q->b = s.a + *p;
    arr[1] += q->b;
    printf("%d %d %d\n", s.b, arr[1], *back);
    return 0;
}
C
"$CCC" -O1 legal_exec.c -o legal_exec
got=$("./legal_exec")
[[ $got == "9 13 8" ]] || fail "legal_exec printed '$got', expected '9 13 8'"

echo "PASS: operand constraint diagnostics match gcc-16.2 (messages, severity, columns where recorded)"
