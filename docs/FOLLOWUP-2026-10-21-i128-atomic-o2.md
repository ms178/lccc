# Follow-up: i128 atomic results miscompiled by a `-O2` middle-end pass

Date: 2026-10-21
Status: **fixed** (2026-10-21, fourth investigation) -- see
"Resolution" at the end. The three hypotheses below are retained because
each one is a plausible reading of the evidence and each is wrong.
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

## Reduction matrix (2026-10-21, third investigation)

Two further hypotheses were tested and **both disproved**:

* *The merged atomic address.* A guard excluding every address reaching a
  16-byte atomic from CSE was implemented and confirmed by IR dump to leave the
  four `GlobalAddr "v"` values distinct — the state the disabled pass produces.
  The miscompile was unchanged.
* *Clobbered scratch registers.* The inline `cmpxchg16b` path uses `%rcx` and
  `%r8` as unconditional scratch and `prepare_atomic_side_effect` only
  invalidates the register cache, so both were saved and restored around the
  sequence. The miscompile was unchanged. (Both changes were reverted.)

Reducing the program by which of the three mixed values is consumed, all at
`-O2 -mcx16` against GCC 16.2:

| mixed value | 1 block | 2 blocks |
| --- | --- | --- |
| compare-exchange result | match | match |
| `expected` after the CAS | match | match |
| the object `v` itself | **fail** (gcc 1, lccc 219) | **fail** (gcc 69, lccc 30) |
| all three | match | **fail** (gcc 220, lccc 181) |

So the corrupted value is the one read back out of the 128-bit object, not the
value the atomic returned and not the value written through `expected`. A
single block is enough.

Hand-flattening that same code out of its `do { … } while (0)` wrapper makes it
**pass**, as does writing the CAS's desired operand as the literal `k + 2`
instead of the expression `k + 1 + (u128)1`. The trigger therefore depends on
the block/SSA structure the wrapper produces, not on the arithmetic, and it has
not been isolated further. The failing cell is reproducible with
`-DW=2 -DN=1` over the reduction source used to produce the table.

## Resolution (2026-10-21, fourth investigation)

Fixed. The cause is in `src/backend/liveness.rs`, not in `global_addr_cse`,
not in DCE, and not in the 16-byte lowering.

### Reduction

The compare-exchange is not needed. This is the whole trigger, and it is
wrong at `-O2`/`-O3` while `-O0`/`-O1` are correct (GCC 16.2 returns 0 at
every level; lccc returned 219):

```c
typedef unsigned __int128 u128;
static _Alignas(16) _Atomic u128 v;
unsigned long long h;
int main(void) {
    u128 k = ((u128)0x0123456789ABCDEFULL << 64) | 0xFEDCBA9876543210ULL;
    __atomic_store_n(&v, k + 1, __ATOMIC_RELAXED);
    do { u128 t_ = (v);
         h = h * 1000003ULL + (unsigned long long)t_
             + (unsigned long long)(t_ >> 64); } while (0);
    return (int)h;
}
```

### Cause

`instruction_is_call_point` decides which IR instructions destroy the
caller-saved registers, so that `collect_call_spanning_owners` can keep such
values out of caller-saved homes. It covered i128 division and the `F128`
helpers -- calls for exactly this reason -- but had **no arm for atomic
instructions**. On x86-64 a 16-byte atomic load, store or RMW lowers to a
`__atomic_*_16` libatomic **call**, so the call was invisible to
`call_points`, `call_spanning` came back empty for values living across it,
and every caller-saved wave was free to home one in a register the call
destroys.

`gaddrcse` supplies the value: merging the two materialisations of `v`'s
address stretches one live range past the atomic. The one-instruction diff
between the pass being on and off:

```
gaddrcse off:   movdqu v(%rip), %xmm0     <- RIP-relative, correct
gaddrcse on:    movdqu (%r11), %xmm0      <- %r11 occurs exactly once in the
                                             whole .s file, as this operand
```

Nothing in the function ever wrote `%r11`; the load dereferenced whatever the
PLT stub left behind. That is a memory-safety bug as well as a wrong value,
and it explains every earlier observation: why `-O1` was correct, why
disabling DCE changed nothing, and why the three hypotheses above all failed.

### Fix

`instruction_is_call_point` now returns true for `AtomicRmw`, `AtomicInc`,
`AtomicCmpxchg`, `AtomicLoad` and `AtomicStore` on `I128`/`U128`. Narrower
atomics stay excluded: they are inline on every supported target (`lock
cmpxchg8b` on i386, plain `lock` forms on x86-64) and are not calls. The
address is now homed in `%rbx`, which survives the call.

### Verification

* The reduction above matches GCC 16.2 at `-O0..-O3`.
* The original two-block reproducer matches GCC 16.2 (220) at `-O0..-O3`
  across five ISA configs: default, `-mcx16`, `-march=x86-64-v3`,
  `-march=x86-64-v2`, `-mno-cx16` -- 20/20.
* `tests/regression/check_i128_atomic_call_clobber.sh` (gate
  `i128-atomic-call-clobber`) asserts both halves: structurally that no
  memory operand in `main` is based on a register `main` never writes, and
  behaviourally the GCC 16.2 differential above. Reverting only the liveness
  change makes the gate fail with
  `main dereferences register(s) it never writes: %r11`, so the gate is
  verified to catch the defect rather than merely to pass.

`check_atomic16_runtime.sh` can now build its runtime half at `-O2`; that is
deliberately left alone here, since moving it changes what an existing gate
covers and belongs in its own change.
