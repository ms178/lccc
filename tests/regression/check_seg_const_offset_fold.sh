#!/usr/bin/env bash
# Segment-override constant-displacement fold gate.
#
# WHY THIS EXISTS
# ---------------
# A per-CPU access with a computed base and a constant struct displacement
#
#     per_cpu_ptr(&pcp, cpu)->field        /* struct S __seg_gs *ptr; ptr->f */
#
# must fold the displacement into the segment memory operand, exactly as GCC
# and Clang do:
#
#     movq %gs:8(%rax), %rax               /* 1 instruction */
#
# lccc used to materialise the address first:
#
#     leaq 8(%rax), %rcx
#     movq %gs:(%rcx), %rax                /* 2 instructions + a second GPR */
#
# That is the kernel's hottest per-CPU shape (`this_cpu_read`, per-CPU
# counters in softirq/sched paths), so the extra laq/register is a real
# regression against the oracle compilers. The fold lives in
# emit_seg_load_with_const_offset (x86 backend) and is reached from
# generate_load's segment branch; the GEP fold map keeps seg-overridden loads
# as foldable consumers so the producer really is skipped.
#
# The same displacement must fold when the field feeds an ALU instruction
# (`acc + p->b` -> `addq %gs:8(%rdi), %reg`): the segment operand replaces a
# whole scratch load, so the fold saves a register as well as an instruction.
# GCC keeps the scratch load in this leaf-add form, so for it lccc emits fewer
# instructions than the oracle, not merely as few.
#
# This gate is the codegen half of the contract; seg_gs_const_offset_fold.c
# is the behavioural half (it installs a real GS base and checks the values).
# A gate that only greps for the operand shape is not enough on its own —
# likewise a behavioural test cannot see the extra instruction. Both run.
set -euo pipefail

CCC=${CCC:-./target/fastbuild/lccc}
REPO=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-seg-const-offset-fold.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
cd "$REPO"

if ! command -v "$CCC" >/dev/null 2>&1; then
    echo "FAIL: compiler '$CCC' not found" >&2
    exit 2
fi

# The folded shape: a register base plus a constant displacement.
cat >"$tmp/fold.c" <<'EOF'
struct S { long a; long b; long c[4]; };
long read_b(struct S __seg_gs *p) { return p->b; }         /* +8            */
long read_c3(struct S __seg_gs *p) { return p->c[3]; }     /* +16+8*3 = +40 */
long read_a(struct S __seg_gs *p) { return p->a; }         /* +0            */
long fold_after_call(struct S __seg_gs *base(void)) { return base()->b; }
struct F { long a; double d; float f; };
double read_d(struct F __seg_gs *p) { return p->d; }       /* +8            */
float  read_f(struct F __seg_gs *p) { return p->f; }       /* +16           */
/* ALU consumers: the field is read straight in the operand
 * (`addq %gs:8(%rdi), %reg`, `cmpq ...`), never via a scratch load. */
long acc_b(struct S __seg_gs *p, long acc) { return acc + p->b; }
int  cmp_c3(struct S __seg_gs *p, long x) { return p->c[3] == x; } /* +40  */
EOF

"$CCC" -O2 -S -o "$tmp/fold.s" "$tmp/fold.c"

fail=0

# body_of <symbol>  -> the instruction lines of that function, no directives.
body_of() {
    awk -v fn="^$1:" '
        $0 ~ fn { inb = 1; next }
        inb && /^\.size/ { exit }
        inb && /^[[:space:]]*#/ { next }
        inb && /^[[:space:]]*\./ { next }
        inb { print }
    ' "$tmp/fold.s"
}

# NOTE: `grep -c ... >/dev/null`, never `grep -q`: these run under `set -o
# pipefail`, and `grep -q` exits at the first match, killing `body_of` with
# SIGPIPE and turning a real pass into a spurious failure (the pipefail-sigpipe
# gate checks exactly this file).
expect() { # expect <symbol> <needle> <description>
    if ! body_of "$1" | grep -cF -- "$2" >/dev/null; then
        echo "FAIL: $1: expected $3 ('$2')" >&2
        echo "--- actual ---" >&2
        body_of "$1" >&2
        fail=1
    fi
}

reject() { # reject <symbol> <needle> <description>
    if body_of "$1" | grep -cF -- "$2" >/dev/null; then
        echo "FAIL: $1: unwanted $3 ('$2')" >&2
        echo "--- actual ---" >&2
        body_of "$1" >&2
        fail=1
    fi
}

# Displacement folded into the segment operand, base register used directly.
# `c[3]` is the struct's third array element after a and b: 16 + 8*3 = 40.
expect read_b  '%gs:8(%rdi)'  'folded +8 displacement on the base register'
expect read_c3 '%gs:40(%rdi)' 'folded +40 displacement on the base register'
# Offset 0 stays the plain form (no bogus "+0" displacement).
expect read_a  '%gs:(%rdi)'   'zero displacement form'
reject read_a  '%gs:0(%rdi)'  'zero displacement must not print as 0(%rdi)'
# No address materialisation for any of the three.
reject read_b  'leaq'         'address materialisation (leaq)'
reject read_c3 'leaq'         'address materialisation (leaq)'
# A call-separated base still folds: the load is what carries the displacement.
expect fold_after_call '%gs:8(%rax)' 'folded displacement after a call'
reject fold_after_call 'leaq'        'address materialisation (leaq) after a call'
# Scalar FP per-CPU loads stay in the SSE domain: one `movsd`/`movss`, never a
# GPR round trip (`movq %gs:..,%rax` + a cross-domain move).
expect read_d 'movsd %gs:8(%rdi)'  'SSE-domain double load'
expect read_f 'movss %gs:16(%rdi)' 'SSE-domain float load'
reject read_d 'movq'               'GPR round trip for a double load'
reject read_f 'movq'               'GPR round trip for a float load'
# ALU fusion: the per-CPU field feeds the add/compare directly (`addq
# %gs:8(%rdi), %reg`), one instruction and no scratch register — the shape
# `this_cpu_ptr(p)->field` takes inside any accumulation.  Any `movq %gs:`
# left in these two bodies means the consumer got a materialised load
# instead; the parity loop below then pins the operand text against GCC.
expect acc_b  'addq %gs:8(%rdi)'   'ALU-fused per-CPU field read (add)'
reject acc_b  'movq %gs:'          'unfused segment load in an add consumer'
reject cmp_c3 'movq %gs:'          'unfused segment load in a compare consumer'

# Cross-check against the GCC 14 oracle (required, see lib/gcc_oracle.sh): the
# same instruction count and the same folded segment operand for each shape.
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_seg_const_offset_fold" || exit 1
"$GCC_ORACLE_BIN" -O2 -S -o "$tmp/fold_gcc.s" "$tmp/fold.c"
count_body() { # count_body <file> <symbol>
    awk -v fn="^$1:" '
        $0 ~ fn { inb = 1; next }
        inb && /^\.size/ { exit }
        inb && /^[[:space:]]*\./ { next }
        inb && /^[[:space:]]*#/ { next }
        inb { n++ }
        END { print n + 0 }
    ' "$2"
}
# `cmp_c3` is deliberately absent here: its one extra instruction is the
# boolean-materialisation tail (`sete %r9b; movzbl %r9b, %r9d; movl
# %r9d, %eax` where GCC zeroes %eax and does `sete %al`), which the SAME
# function shows without any segment override (`cmpq %rsi, 40(%rdi)`) —
# it is a return-register coalescing gap, not a segment-fold gap.
# docs/FOLLOWUP-2026-10-11-kernel-percpu-segment-fold.md §5.4 carries the
# reproducer; the folded OPERAND of cmp_c3 is still pinned by the loop
# below, so the segment half of the contract stays checked here.
for fn in read_b read_c3 read_a acc_b; do
    ours=$(count_body "$fn" "$tmp/fold.s")
    theirs=$(count_body "$fn" "$tmp/fold_gcc.s")
    if [ "$ours" -gt "$theirs" ]; then
        echo "FAIL: $fn: lccc $ours instructions vs GCC $theirs" >&2
        fail=1
    fi
done
# Exact-operand cross-check: the segment operand GCC folds must be the
# operand lccc emits, character for character.  Instruction counts alone
# cannot see a wrong constant — this gate itself once expected +32 for a
# +40 field (`c[3]` after `long a, b`), so the oracle compares the operand
# text, not just the count.  GCC spells a zero displacement the same way
# (`%gs:(%rdi)`), so all four shapes are compared.
for fn in read_b read_c3 read_a fold_after_call read_d read_f acc_b cmp_c3; do
    want=$(awk -v fn="^$fn:" '
        $0 ~ fn { inb = 1; next }
        inb && /^\.size/ { exit }
        inb && /%gs:[^,)]*\(%r[a-z0-9]+\)/ {
            match($0, /%gs:[^,)]*\(%r[a-z0-9]+\)/)
            print substr($0, RSTART, RLENGTH)
            exit
        }
    ' "$tmp/fold_gcc.s")
    if [ -z "$want" ]; then
        # A vacuous oracle would silently pass: every shape listed here is
        # expected to make GCC emit a folded segment operand.
        echo "FAIL: $fn: GCC emitted no folded segment operand to compare against" >&2
        fail=1
    elif ! body_of "$fn" | grep -cF -- "$want" >/dev/null; then
        echo "FAIL: $fn: GCC folds '$want' but lccc emits:" >&2
        body_of "$fn" >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "FAIL: segment constant-displacement fold gate" >&2
    exit 1
fi
echo "PASS: segment constant-displacement fold (%gs:disp(base), no leaq, GCC instruction parity)"
