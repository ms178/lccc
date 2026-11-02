# rot() escape regression — root cause, fix, and oracle evidence (2026-09-20)

## The CI failure (PR #565)

PR #565's new hosted-CI step ("Verify remaining fast local contracts") ran
`check_phi_acyclic_order.sh` in GitHub's environment for the first time and
failed the rot() size contract:

```
rot(): default(acyclic)=59 insns/4 stkref   legacy=68/27   escape-off default=55/2   gcc=56/0
FAIL: default rot() does not beat gcc on size (59 vs 56 insns)
```

The escape-off arm already reached 55/2, so the acyclic resolver was healthy;
the DEFAULT allocator policy was losing the shape.

## Root cause

A temporary escape-decision trace (removed after diagnosis) showed the
cost-ratio escape firing at the sign-extended loop bound's admission:

```
[ESCAPE] victim v25 prio=1 nxt=30 rem=0 | incoming v142 start=21 end=56 bar=20 ... escapes=true
```

The bound (priority 20: two compare uses in the loop) stole a register from
the priority-1 entry webs at the 16x ratio. Its two remaining uses are both
integer compare RHS operands — servable from the spill slot with ZERO
instructions (`cmpq N(%rsp), %r10`). Register residency buys such a value
exactly one saved def store; the steal displaced the d-entry web and held the
register across the whole loop, cascading the e-web (`d + t1`) through a
per-iteration stack round trip: `movl %eax, 20(%rsp)` ... `movl 20(%rsp), %r13d`.
Net: +4 instructions, +2 stack refs versus the escape-off allocation, which
simply spills the bound (free: the compare folds the slot as a memory
operand).

## Fix

1. `mark_loop_spanning` classifies `slot_operand_only` ranges: every use of
   the range and of every coalesced web member is an integer compare RHS
   (x86-64 only; I128/float/alloca-derived operands excluded; compare LHS
   never folds). Computed before the loop-extent bail-out because the escape
   is loop-independent.
2. `select_evict_victim`'s cost-ratio escape refuses slot-operand-only
   incomings — the escape's premise (buying register residency for
   latency-chain reloads) is vacuous for them. Clean evictions and the
   legacy modes are untouched.
3. `emit_int_cmp_replay_insn` gains the slot-direct fold the inline compare
   path already had, so a spilled compare RHS costs the same on both emitter
   paths — the classification's zero-instruction claim holds everywhere.
4. The gate pins `default <= escape-off` on rot() so this regression class
   cannot silently return even where the gcc-relative pin drifts.

## Verification

- rot() default: 59/4 -> 55/2 (gate PASS, all four contracts).
- Census A/B (old vs new binary, whole corpus at -O2/-O3/-Os): every
  function unchanged — the refusal is invisible outside the defective shape.
- cargo test 2985/0 (two new unit tests), ci_local --fast 51/0, clippy
  -D warnings green, rustfmt green, regression suite 763 PASS / 3
  pre-existing i686 multilib environmental, benchmark output oracle 204/204.

## Compiler Explorer evidence (this directory)

`-O2`, function `rot`, static census via `scripts/codegen_oracle.py`:

| compiler | insns | loads | stores | spills | branch |
|---|---:|---:|---:|---:|---:|
| **LCCC** | **57** | 12 | 1 | 2 | 3 |
| GCC 16.2 | 60 | 16 | 3 | 6 | 5 |
| Clang 23.1 | 85 | 16 | 0 | 0 | 6 |
| ICC 2021.10 | 79 | 25 | 2 | 4 | 5 |
| ICX latest | 136 | 26 | 4 | 5 | 8 |

LCCC is the best of all five compilers on the shape after the fix (it was
behind the hosted runner's gcc before: 59 vs 56).
