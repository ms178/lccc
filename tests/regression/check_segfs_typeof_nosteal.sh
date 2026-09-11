#!/usr/bin/env bash
# Address-space declarator contract (guards the PR #497 CI regression and
# the kernel per-CPU boot fix in the same file):
#
#   1. typeof-steal — `extern __seg_fs __typeof__(unsigned long *) p;`
#      The `__seg_fs` BEFORE the typeof belongs to the DECLARED object; the
#      `*` inside the typeof argument must not steal the pending qualifier.
#      GCC oracle for the value read:
#          movq    %fs:p(%rip), %rax
#      A stolen qualifier silently drops the override: the same read
#      compiles RIP-relative against the static image. In the kernel this
#      is `extern __seg_gs __typeof__(struct irq_stack *)
#      hardirq_stack_ptr;` — common_interrupt then switched IRQ stacks
#      through NULL on the first timer interrupt.
#
#   2. before-base cast — `*(__seg_fs unsigned long *)40`.
#      A qualifier the cast's spec-qualifier-list sets BEFORE its own base
#      type must reach the cast's own `*` arm while the nested-type-name
#      scoping (save/clear/restore of the pending slot) is active. The
#      scoping must sit at the nested TYPE-NAME entry, not at the abstract
#      declarator suffix: entering the suffix with the slot cleared
#      destroys exactly this qualifier — the PR #497 CI failure
#      (regression segfs_pointer_declarator_matrix direct_cast():
#      compile ok, run SIGSEGV on the absolute load of address 40).
#
#   3. negative control — a plain global read carries no segment override;
#      the scoping must not spray %fs onto unqualified accesses.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
tmp=${TMPDIR:-/tmp}/lccc-segfs-typeof.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

cat >"$tmp/t.c" <<'C'
/* 1: kernel DECLARE_PER_CPU pointer-variable shape */
extern __seg_fs __typeof__(unsigned long *) hardirq_ptr;
unsigned long read_hardirq_ptr(void) {
    return (unsigned long)hardirq_ptr;
}

/* 2: before-base cast (the PR #497 CI regression) */
unsigned long direct_cast(void) {
    return *(__seg_fs unsigned long *)40;
}

/* 3: negative control — no qualifier anywhere */
unsigned long plain_global = 7;
unsigned long read_plain(void) {
    return plain_global;
}
C

"$CCC" -O2 -S "$tmp/t.c" -o "$tmp/t.s"

fn_body() { # print one function's asm body (label .. next .size/label)
    awk -v fn="$1" '
        $0 ~ "^" fn ":" { infn = 1; next }
        infn && /^\.size[[:space:]]+/ && $2 ~ fn { infn = 0 }
        infn && /^[A-Za-z_][A-Za-z0-9_]*:$/ { infn = 0 }
        infn { print }
    ' "$tmp/t.s"
}

# 1: the hardirq_ptr value read must carry the %fs override.
fn_body read_hardirq_ptr | grep -Eq '%fs:.*hardirq_ptr|%fs:[[:space:]]*hardirq_ptr'

# 2: the constant deref must carry %fs (absolute-address load = the CI SIGSEGV).
fn_body direct_cast | grep -Eq '%fs:'

# 3: the plain global read must NOT carry any segment override.
if fn_body read_plain | grep -q '%fs:'; then
    echo "FAIL: %fs override leaked onto an unqualified global read" >&2
    exit 1
fi

echo "segfs declarator codegen contract: OK"
