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
#  14. known divergence: GCC rejects a generic `struct c *` stored into a
#                       `__seg_gs` member; lccc accepts it. Pinned here so
#                       that fixing it flips this check.
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

"$CCC" -O2 -S -o "$tmp/t.s" "$tmp/t.c"

cat >"$tmp/divergence.c" <<'C'
struct c { long v[2]; };
struct g { struct c __seg_gs *scp; };
void store_generic(struct g *t, struct c *q) { t->scp = q; }
C

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
grep -q '%gs:gobj' <<<"$(body rd_gobj)" || { echo "FAIL: rd_gobj: gobj not loaded via %gs:gobj" >&2; body rd_gobj >&2; exit 1; }
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

# 14: pinned divergence. GCC: "assignment from pointer to non-enclosed address
# space" is an error. lccc currently accepts it; when that changes, this check
# must be updated to expect the rejection instead.
if "$CCC" -S -o "$tmp/div.s" "$tmp/divergence.c" 2>"$tmp/div.err"; then
    echo "KNOWN-DIVERGENCE: generic->__seg_gs member store accepted, GCC rejects" >&2
else
    echo "NOTE: generic->__seg_gs member store is now rejected; flip this check to expect rejection" >&2
    exit 1
fi

echo "PASS: __seg_gs declaration qualifier reaches params, locals, returns, typedefs; object vs pointee placement"
