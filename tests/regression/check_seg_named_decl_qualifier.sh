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
C

"$CCC" -O2 -S -o "$tmp/t.s" "$tmp/t.c"

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

for fn in rd_param rd_local rd_proto_ret rd_def_ret rd_typedef; do
    require_gs "$fn"
done

plain=$(body rd_plain)
if grep -q '%gs:' <<<"$plain"; then
    echo "FAIL: rd_plain: unqualified pointer load carries a %gs override" >&2
    echo "$plain" >&2
    exit 1
fi

echo "PASS: __seg_gs declaration qualifier reaches params, locals, returns, typedefs"
