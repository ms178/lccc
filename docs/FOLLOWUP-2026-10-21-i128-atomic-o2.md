# Follow-up: i128 atomic results miscompiled by a `-O2` middle-end pass

Date: 2026-10-21
Status: open, not fixed here
Found by: differential runtime testing against GCC 16.2 while validating the
16-byte atomic lowering.

## Summary

A 128-bit atomic produces the correct value at every optimisation level, but
when its result feeds *inlined* i128 arithmetic, `-O2`/`-O3` compute the wrong
answer. `-O0` and `-O1` are correct. This is a middle-end optimisation bug,
not a lowering bug: the emitted atomic sequence is right, and the same values
print correctly when they are directly observable.

## Evidence

Program: `N` blocks of `store; compare-exchange`, each mixing the boolean
result, the updated `expected` and the new object value into a hash.

| build | result |
| --- | --- |
| gcc 16.2 `-O0/-O1/-O2/-O3` | `91f72b4dca80cfa6` |
| lccc `-O0` | `91f72b4dca80cfa6` — correct |
| lccc `-O1` | `91f72b4dca80cfa6` — correct |
| lccc `-O2` | `02314298393076e6` — **wrong** |
| lccc `-O3` | `02314298393076e6` — **wrong** |

Bisection, each step measured:

1. `N=1` block: correct at `-O2`. `N=2` blocks: already wrong. The bug needs
   two atomic operations, not one.
2. Replacing the hashing helper with a `__attribute__((noinline))` function
   taking the `u128` by value: **correct at `-O2`**. Inlining the same
   arithmetic into `main`: wrong. So the defect is in the optimiser's handling
   of the inlined consumer, not in the atomic lowering or the 128-bit call ABI.
3. The same arithmetic fed by a *non-atomic* `u128` source (loop over a static
   array, shifts/truncates/multiplies): **correct at `-O1/-O2/-O3`**. So plain
   i128 arithmetic is fine; the atomic definition of the value is required.
4. Printing the values instead of hashing them: **correct at `-O2`**, for all
   nine `(success, failure)` ordering pairs, matching GCC 16.2 exactly.

(2) + (3) + (4) together localise it to a pass that runs at `-O2`, reasons
about the atomic instruction's memory effects, and reuses a stale value once
the consumer is inlined and the intermediate results are no longer directly
observable.

## Minimal reproducer

```c
typedef unsigned __int128 u128;
static _Alignas(16) _Atomic u128 v;
static unsigned long long h;

int main(void) {
    u128 k = ((u128)0x0123456789ABCDEFULL << 64) | 0xFEDCBA9876543210ULL;
#define MIX(x) do { u128 t_ = (x); \
    h = h * 1000003ULL + (unsigned long long)t_ + (unsigned long long)(t_ >> 64); \
} while (0)
    {   __atomic_store_n(&v, k + 1, __ATOMIC_RELAXED);
        u128 e = k + 1;
        MIX((u128)__atomic_compare_exchange_n(&v, &e, k + 1 + (u128)1, 0,
                                              __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
        MIX(e); MIX(v); }
    {   __atomic_store_n(&v, k + 2, __ATOMIC_RELAXED);
        u128 e = k + 2;
        MIX((u128)__atomic_compare_exchange_n(&v, &e, k + 2 + (u128)1, 0,
                                              __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST));
        MIX(e); MIX(v); }
    return (int)h;   /* gcc 16.2 -O2: 0xd674e05bde401cdc, lccc -O2: 0xb5afa0312156b616 */
}
```

## Why it is not fixed in this change

The 16-byte lowering this change touches is now verified correct: it matches
GCC 16.2 bit-for-bit for every load, store, RMW op and ordering, at every
optimisation level, and `tests/regression/check_atomic16_runtime.sh` asserts
exactly that. Fixing this one means finding and correcting an optimiser pass,
and a wrong change there silently miscompiles unrelated code — it needs its
own change with its own differential test corpus rather than being bundled
into an ISA-policy commit.

## Consequence for the test suite

`tests/regression/check_atomic16_runtime.sh` builds its runtime half at `-O1`,
where the lowering and the consumer are both correct, so the gate tests what it
claims to test. When this is fixed, the runtime half should move to `-O2` and a
second gate should assert the reproducer above.

## Suggested first probes

* Diff the IR immediately before and after each `-O2`-only pass for the
  reproducer, watching the `AtomicCmpxchg` result value and the `v` slot.
* Check that `AtomicStore`/`AtomicCmpxchg` on `I128`/`U128` are treated as
  memory-clobbering for CSE/GVN and for store-to-load forwarding of the
  16-byte slot; the 64-bit widths may be relying on an alias rule that does
  not generalise to a two-slot value.
