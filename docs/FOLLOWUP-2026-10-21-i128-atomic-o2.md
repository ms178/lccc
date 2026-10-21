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

A second, wider program (every op crossed with every legal ordering, hashed)
splits by lowering strategy, which narrows it further. Against
`gcc 16.2 -O2 -mcx16` (`hash=11d32f8da1731c3e`):

| config | `-O1` | `-O2` |
| --- | --- | --- |
| `-mcx16` (inline `cmpxchg16b`) | match | match |
| `-march=x86-64-v3` | match | match |
| `-march=x86-64-v2` | match | match |
| `-mno-cx16` (libatomic call) | match | **`62a84940b7697ee2`** |
| default (libatomic call) | match | **`62a84940b7697ee2`** |

Every config agrees with GCC at `-O1`, so the lowering is right for all of
them; at `-O2` only the two configs whose compare-exchange is a *call* into
libatomic diverge, while the inline `cmpxchg16b` configs stay correct. Note
the first reproducer above diverges under `-mcx16` too, so the call form is
not the only trigger — but that the defect tracks the call form here points
hard at call-clobber modelling: a pass is not treating the libatomic call as
clobbering the 16-byte object it mutates through a pointer.

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

## Correction (2026-10-21, second investigation)

The "call-clobber modelling for the libatomic call form" reading above is
**wrong** and should not be trusted. Pass bisection with
`CCC_DISABLE_PASSES` over all 73 kill switches identified `gaddrcse`
(`src/passes/global_addr_cse.rs`) as the unique single pass whose removal
restores the correct answer, and `CCC_NO_GADDR_CSE=1` agrees. But a guard that
excludes every address reaching a 16-byte atomic from CSE — verified by IR dump
to leave the four `GlobalAddr "v"` values distinct, exactly as with the pass
disabled — **does not fix the miscompile**. So the atomic's address is not the
trigger.

Diffing the final IR with the pass on and off shows what actually still merges:
the address of the *accumulator*, a plain `unsigned long long h` that the
arithmetic consumer reads and writes. With `gaddrcse` on, every `GlobalAddr "h"`
collapses to one value; with it off they stay distinct. Merging a plain
global's address is what exposes the defect; the 16-byte atomics in the same
block are what make it wrong.

Ruled out by reading the code, both are default-closed and clear their state on
every atomic including `AtomicInc`:

* `src/passes/redundant_loads.rs` (lines ~246-260)
* `src/passes/store_load_forward.rs` (lines ~399-405)

Disabling either of those, or `slforward`, does not restore the answer — it
turns the wrong value into a **SIGSEGV**, which is a second and independent
defect, not a fix.

Also measured: plain 64-bit load/store and 64-bit atomics with the same shape
match GCC 16.2 exactly; only the 128-bit form diverges. Adding a `printf` of
the intermediate values makes the 128-bit case correct under `gaddrcse`, so the
bad transformation needs the values to be consumed only by arithmetic.

Next step that follows from this: instrument the passes that key on address
*value identity* rather than on proven aliasing, and find which one treats two
accesses to the merged `h` as interchangeable across the intervening 16-byte
atomic. The candidates that were individually disabled without effect (`gvn`,
`globaldse`, `dse`, `copyprop`, `simplify`, `constfold`, `licm`, `cvp`, `sccp`)
should be re-tested in combination, since no single one of them is sufficient.

### Narrowest reproducer found

The merged address is the accumulator's, so the atomic object and the
accumulator must both be file-scope. `gcc 16.2 -O2 -mcx16` returns 220; lccc
`-O2` returns 181 with `gaddrcse` on and 220 with it off.

```c
typedef unsigned __int128 u128;
static _Alignas(16) _Atomic u128 v;
unsigned long long h;                 /* this is the address that gets merged */

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
    return (int)h;
}
```
