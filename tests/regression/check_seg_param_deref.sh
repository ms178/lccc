#!/usr/bin/env bash
# Segment-space gate for parameters and pointer objects (linux-6.18.55
# softirq/fault/setup per-CPU accesses).
#
#   * `T __seg_gs *p` parameter: `*p` must lower to a %gs-prefixed access.
#     Before the fix the qualifier was dropped on the parameter's CType and
#     the access was a plain `movl (%rdi)` — a silent per-CPU miscompile.
#   * `&obj` of a __seg_gs object is the object's ADDRESS (no %gs base is
#     applied to the address-of), so it is passed as a plain `leaq`.
#   * A `T __seg_gs *g` global is ordinary memory: the pointer variable
#     itself must not be %gs-prefixed; only its pointee is.
# Checks the emitted assembly text (-S); the runtime values are covered by
# tests/regression/seg_param_address_space.c.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-seg-param-deref.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/t.c" <<'C'
static int __seg_gs obj = 41;
static int __seg_gs arr[4] = {1, 2, 3, 4};
int __seg_gs *seg_ptr_global;
int read_param(int __seg_gs *p) { return *p; }
void write_param(int __seg_gs *p, int v) { *p = v; }
int second(int __seg_gs *p) { return p[1]; }
int addr_obj(void) { return read_param(&obj); }
void store_global(void) { seg_ptr_global = &arr[1]; }
struct item { int v; };
static struct item *__seg_gs ptr_obj;
struct item *load_ptr_obj(void) { return ptr_obj; }
void store_ptr_obj(struct item *p) { ptr_obj = p; }
C

"$CCC" -O2 -S -o "$tmp/t.s" "$tmp/t.c"

# Extract one function body (label line through the next .size or blank run).
body() {
    awk -v f="$1:" '$0==f{on=1;next} on&&/^\s*\.size/{exit} on{print}' "$tmp/t.s"
}
fail=0
need() { # function, regex
    if ! body "$1" | grep -Eq "$2"; then
        echo "FAIL: $1 lacks /$2/" >&2
        body "$1" >&2
        fail=1
    fi
}
forbid() { # function, regex
    if body "$1" | grep -Eq "$2"; then
        echo "FAIL: $1 has forbidden /$2/" >&2
        body "$1" >&2
        fail=1
    fi
}

need read_param '%gs:\(%rdi\)'
need write_param '%gs:\(%rdi\)'
need second '%gs:\(%rcx\)'
forbid addr_obj '%gs'
need addr_obj 'leaq obj\(%rip\), %rdi'
forbid store_global '%gs:seg_ptr_global'
need store_global 'seg_ptr_global\(%rip\)'
# `struct item *__seg_gs ptr_obj`: the qualifier after the last `*` places the
# pointer OBJECT in %gs (the per-CPU pointer case), so it is accessed through %gs.
need load_ptr_obj 'movq %gs:ptr_obj\(%rip\), %rax'
need store_ptr_obj 'movq %rdi, %gs:ptr_obj\(%rip\)'

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "check_seg_param_deref: ok"
