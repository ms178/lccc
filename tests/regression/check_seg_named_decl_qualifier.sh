#!/usr/bin/env bash
# Declaration-level named address space contract (kernel srcu_read_lock_fast /
# srcu_read_unlock_fast boot blocker, 6.18.55 per-CPU counters).
#
# `T __seg_gs *x` qualifies the pointer that the declarator builds, and that
# qualifier must reach every place the object's type is consumed:
#
#   1. parameter        `long f(struct c __seg_gs *p) { return p->v[0]; }`
#   2. local            `struct c __seg_gs *p = lk(); p->v[1]`
#   3. prototype return `struct c __seg_gs *lk(void);` then `lk()->v[0]`
#   4. definition ret.  `struct c __seg_gs *mk(void) { ... }` then `mk()->v[0]`
#   5. argument pass    a call taking `struct c __seg_gs *` accepts the
#                       qualified result of 3/4 (no "incompatible pointer")
#   6. typedef          `typedef struct c __seg_gs *gp_t;` then `gp_t g; g->v[0]`
#   7. parameter list of a qualified function must not inherit the qualifier:
#      `struct c __seg_gs *lk2(struct s *ssp)` keeps `ssp` generic, so a
#      plain `struct s *` argument is accepted (GCC accepts it).
#   8. negative control — a plain `struct c *` load carries no %gs override.
#   9. function pointer  `struct c __seg_gs *(*fp)(void)`: the return-type
#                       pointer is %gs, the function pointer load is not.
#  10. pointer to ptr   `struct c __seg_gs **pp`: `pp[0]` is generic; only the
#                       final `->` dereference is %gs (exactly one override).
#  11. global pointer   `struct c __seg_gs *gp`: the variable `gp` itself is
#                       generic memory (never `%gs:gp`); `gp->v[0]` is %gs.
#  12. global pointer array `struct c __seg_gs *garr[4]`: elements are generic
#                       loads, one %gs override for the dereference.
#  13. positive control `struct c __seg_gs gobj`: a plain object declared in
#                       the space is loaded as `%gs:gobj`.
#  14. constraint: a generic `struct c *` stored into a `__seg_gs` member is
#                       rejected (GCC 14.2: "assignment from pointer to
#                       non-enclosed address space"), as are the reverse, an
#                       initializer, and `void *` in either direction. An
#                       explicit cast and a same-space store stay accepted.
#  15. whole-type qualifier `__seg_gs __typeof__(struct c *) gpv;`: the pointer
#                       object itself is %gs (kernel per-CPU pointer shape).
#                       The pointee is NOT qualified by this form.
#
# GCC is the oracle for the accepted/rejected set and for the %gs prefix on
# each load; the check is on the emitted assembly, not only the exit status.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-seg-named-decl.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/t.c" <<'C'
struct c { long v[2]; };
struct s { int x; };

struct c __seg_gs *lk(void);
struct c __seg_gs *mk(void) { return 0; }
struct c __seg_gs *lk2(struct s *ssp) { (void)ssp; return 0; }
typedef struct c __seg_gs *gp_t;

static inline void take(struct c __seg_gs *p) { (void)p; }

long rd_param(struct c __seg_gs *p) { return p->v[0]; }
long rd_local(void) { struct c __seg_gs *p = lk(); return p->v[1]; }
long rd_proto_ret(void) { return lk()->v[0]; }
long rd_def_ret(void) { return mk()->v[1]; }
void pass_ret(void) { take(lk()); take(mk()); }
long rd_typedef(gp_t g) { return g->v[0]; }
void pass_generic(struct s *s) { lk2(s); }
long rd_plain(struct c *q) { return q->v[0]; }

struct c __seg_gs *(*fp)(void);
long rd_fnptr(void) { return fp()->v[0]; }
struct c __seg_gs **pp;
long rd_pp(void) { return pp[0]->v[0]; }
struct c __seg_gs *gp;
long rd_gp(void) { return gp->v[0]; }
struct c __seg_gs *garr[4];
long rd_garr(int i) { return garr[i]->v[0]; }
struct c __seg_gs gobj;
long rd_gobj(void) { return gobj.v[0]; }
extern __seg_gs __typeof__(struct c *) gpv;
long rd_whole(void) { return (long)gpv; }
C

cat >"$tmp/divergence.c" <<'C'
struct c { long v[2]; };
struct g { struct c __seg_gs *scp; };
void store_generic(struct g *t, struct c *q) { t->scp = q; }
C

asm_checks() { # the emitted-assembly checks, on $tmp/t.s for the current -O level
# Body of one function from the emitted assembly.
# Ends at the first `ret` (lccc indents with spaces, GCC with tabs).
body() { awk -v f="$1" '$0 ~ "^"f":" {on=1} on {print} on && /^[ \t]*ret[ \t]*$/ {exit}' "$tmp/t.s"; }

require_gs() { # require_gs <fn>  — the function must contain a %gs-prefixed access
    local b
    b=$(body "$1")
    if [[ -z $b ]]; then
        echo "FAIL: $1: no function body found in assembly" >&2
        exit 1
    fi
    if ! grep -q '%gs:' <<<"$b"; then
        echo "FAIL: $1: missing %gs segment override" >&2
        echo "$b" >&2
        exit 1
    fi
}

for fn in rd_param rd_local rd_proto_ret rd_def_ret rd_typedef rd_fnptr; do
    require_gs "$fn"
done

gs_count() { # gs_count <fn> -> number of %gs: overrides in the function body
    body "$1" | grep -o '%gs:' | wc -l | tr -d ' '
}
expect_count() { # expect_count <fn> <n>
    local got
    got=$(gs_count "$1")
    if [[ $got != "$2" ]]; then
        echo "FAIL: $1: expected $2 %gs override(s), got $got" >&2
        body "$1" >&2
        exit 1
    fi
}
expect_no_gs_of() { # expect_no_gs_of <fn> <symbol>
    if grep -q "%gs:$2\b" <<<"$(body "$1")"; then
        echo "FAIL: $1: $2 itself is loaded through %gs (object storage, not pointee)" >&2
        body "$1" >&2
        exit 1
    fi
}

# 9-12: the function-pointer and pointer-level placement cases.
expect_count rd_fnptr 1
expect_count rd_pp 1
expect_no_gs_of rd_pp pp
expect_count rd_gp 1
expect_no_gs_of rd_gp gp
expect_count rd_garr 1
expect_no_gs_of rd_garr garr
# 13: a plain object of the declared space keeps its %gs storage.
# At -O2 the access is the direct `%gs:gobj`; at -O0/-O1 it is `leaq gobj(%rip)`
# and a %gs access through that address, the same effective address.
if [[ $opt == -O2 ]]; then want='%gs:gobj'; else want='%gs:'; fi
grep -q -- "$want" <<<"$(body rd_gobj)" || { echo "FAIL: rd_gobj ($opt): gobj not loaded via $want" >&2; body rd_gobj >&2; exit 1; }
# 15: the whole-type pointer object is in %gs; reading it is a %gs load.
grep -q '%gs:gpv' <<<"$(body rd_whole)" || { echo "FAIL: rd_whole: gpv not loaded via %gs:gpv" >&2; body rd_whole >&2; exit 1; }
# Locals: the pointer slot itself is generic; one deref override.
expect_count rd_local 1

plain=$(body rd_plain)
if grep -q '%gs:' <<<"$plain"; then
    echo "FAIL: rd_plain: unqualified pointer load carries a %gs override" >&2
    echo "$plain" >&2
    exit 1
fi

}

for opt in -O0 -O1 -O2; do
    "$CCC" "$opt" -S -o "$tmp/t.s" "$tmp/t.c"
    asm_checks
    echo "  emitted-assembly checks pass at $opt"
done

# 14: a named-space pointer conversion is a constraint violation. GCC 14.2
# rejects each of these with "from pointer to non-enclosed address space"; lccc
# must reject it with its address-space diagnostic (not an unrelated error).
expect_reject() { # expect_reject <label> <file>
    if "$CCC" -S -o "$tmp/rej.s" "$2" 2>"$tmp/rej.err"; then
        echo "FAIL: $1: accepted; GCC 14.2 rejects it" >&2
        exit 1
    fi
    # Assignment and initializer conversions say "incompatible address space";
    # an argument keeps the existing argument diagnostic (pointer_argument_compat).
    if ! grep -q -e 'incompatible address space' -e 'incompatible pointer type for argument' "$tmp/rej.err"; then
        echo "FAIL: $1: rejected, but not for the address space" >&2
        cat "$tmp/rej.err" >&2
        exit 1
    fi
}
expect_accept() { # expect_accept <label> <file>
    if ! "$CCC" -S -o "$tmp/acc.s" "$2" 2>"$tmp/acc.err"; then
        echo "FAIL: $1: rejected; GCC 14.2 accepts it" >&2
        cat "$tmp/acc.err" >&2
        exit 1
    fi
}
cat >"$tmp/void_out.c" <<'C'
struct c { long v[2]; };
struct c __seg_gs *gp;
void *src(void);
void f(void) { gp = src(); }
C
cat >"$tmp/init_in.c" <<'C'
long n;
long __seg_gs *g = &n;
C
cat >"$tmp/init_out.c" <<'C'
struct c { long v[2]; };
struct c __seg_gs *gp;
struct c *q = gp;
C
cat >"$tmp/arg_void.c" <<'C'
struct c { long v[2]; };
struct c __seg_gs *gp;
void sink(void *p);
void f(void) { sink(gp); }
C
expect_reject "14a: generic -> __seg_gs member store" "$tmp/divergence.c"
expect_reject "14b: __seg_gs -> generic local initializer" "$tmp/init_out.c"
expect_reject "14c: generic &n -> __seg_gs initializer" "$tmp/init_in.c"
expect_reject "14d: void * -> __seg_gs assignment" "$tmp/void_out.c"
expect_reject "14e: __seg_gs -> void * argument" "$tmp/arg_void.c"
cat >"$tmp/ok_cast.c" <<'C'
struct c { long v[2]; };
struct c __seg_gs *gp;
struct c __seg_gs *same;
struct c *q;
void f(void) { q = (struct c *)gp; same = gp; }
C
expect_accept "14f: explicit cast and same-space store" "$tmp/ok_cast.c"

echo "PASS: __seg_gs declaration qualifier reaches params, locals, returns, typedefs; object vs pointee placement"
