# SPILL-01 — causal stack-reference census

**Status:** tool shipped (`scripts/stack_census.py`), compiler side shipped
(`src/backend/stack_layout/slot_census.rs`, opt-in with `CCC_SLOT_CENSUS=1`).
**Baseline:** `93f2a43b` + ZERO-REM-1.
**Gate criterion from the follow-up doc:** classification must cover ≥ 95 % of
stack references on the benchmark corpus. **Measured: 98.74 %** (1251 / 1267).

---

## 1. Why a compiler-side cause is needed

`scripts/ra_quality_census.py` counts how many stack references a function emits
(`stkref`). It cannot say *why* any of them exists, and that is the question that
decides where engineering time goes:

* a slot holding a spilled register **candidate** is an allocator problem;
* a slot holding an `alloca`, an address-taken local, an i128/vector value or an
  outgoing-argument area is **structurally required memory** that no eviction
  policy can remove.

Tuning an allocator against a number that is mostly `alloca` traffic is how
optimization budgets are wasted. Only the allocator knows which is which — it is
the component that decided a value would not get a register — so the cause is
published next to the frame layout instead of being re-derived (and eventually
drifting) in a script. `RegAllocResult` now carries the allocator's own
`eligible` set, and `build_slot_census` turns it into a per-slot cause:

| cause | meaning |
|---|---|
| `alloca` | explicit alloca / parameter home: addressable by construction |
| `address` | has a register home **and** a memory home: something observes its address |
| `wide` | i128 / vector / wide value: no single-register home is modelled |
| `spill` | register pressure: an allocation candidate that lost |
| `nongpr` | float / long-double in a configuration without FP register homes |
| `temp` | unclassified — reported separately so it cannot be folded into `spill` |

## 2. Why the measurement is taken on the assembly

The census is emitted during frame layout but is *joined* with the
**post-peephole** assembly, because peepholes delete accesses (dead stores,
unused callee saves) and add others (address materialization). A census taken
before emission would report traffic that never reaches the object file.

Offset arithmetic: `[SLOT-MAP]` offsets are absolute byte offsets from the
**entry `%rsp`** (negative). The assembly uses `%rsp`/`%rbp`-relative
displacements after the prologue, so the tool recovers the prologue geometry
from the emitted text itself:

```
p pushes then `subq $N, %rsp`
    %rsp operand  ->  entry offset = disp − 8p − N
    %rbp operand  ->  entry offset = disp − 8p        (`movq %rsp, %rbp`)
    disp ≥ N (rsp) -> below the frame: outgoing argument area
    disp ≥ 8 (rbp) -> incoming stack argument
```

Indexed operands (`16(%rsp,%rdx)`) carry a dynamic index, so their displacement
is only the base of a walked range; they are attributed to the nearest declared
slot instead of demanding an exact hit — "which element of `m`" is unknowable
statically, while "which object" is exactly the question being asked.

## 3. Corpus result (`-O2`, benchmark programs + kernel corpus, 56 files)

```
   coverage            : 98.74% (1251/1267 references attributed)
   pressure-driven     : 136 refs (10.7%)
   structural (memory) : 301 refs (23.8%)
   frame protocol      : 562 saves, 1 outgoing-arg, 5 incoming-arg

   cause        refs     share
   csave         562     44.4%   callee-saved save/restore
   temp          246     19.4%   unclassified
   alloca        233     18.4%   explicit addressable objects
   spill         136     10.7%   register pressure  ← the only removable kind
   nongpr         42      3.3%
   wide           26      2.1%
   unknown        16      1.3%
   incoming        5      0.4%
   argout          1      0.1%
```

## 4. What the data says

1. **The backlog's spill framing is wrong for this corpus.** Spills are 10.7 %
   of stack references. The dominant category — 44.4 % — is the *frame
   protocol*: callee-saved registers saved on entry and restored on exit. Every
   register the allocator takes from the callee-saved pool costs two stack
   references per call, and the corpus is full of functions that pay it for
   registers they barely use.
2. **`sha256_transform`, the RA-PRESSURE-3 target, has zero spill references.**
   Its 21 stack references are 5 `alloca` (the `m[64]` array — irreducible, it
   is address-taken by the schedule) and 12 `csave`. Its instruction-count gap
   to Clang was never a spill problem; ZERO-REM-1 closed 16 of the 23
   instructions by removing dead vectorizer code instead.
3. **The busiest slots are array allocas, not spills** (`--per-slot`): the top
   slot is `main`'s `-464:alloca` with 55 references (51 stores). That is
   benchmark scaffolding writing into a local array, not a register allocation
   failure.
4. `temp` at 19.4 % is the honest remainder: values that are neither eligible
   nor type-classified. It is reported, not hidden, and it is the bucket to
   attack if the census is to become exhaustive.

## 5. Usage

```sh
scripts/stack_census.py --corpus --json /tmp/census.json --per-slot 12
scripts/stack_census.py tests/benchmark/programs/sha256_transform.c --show-unknown
scripts/stack_census.py --corpus --min-coverage 95     # gate, exit 1 below
```

`CCC_SLOT_CENSUS=1` is set by the tool; the diagnostic is purely observational
(verified: `sha256_transform` is 136 instructions with and without the variable).

## 6. RA-GLA-04

The follow-up item asked for "per-planned-location-piece slot traffic, machine
readable, before reopening any spill gap". `scripts/stack_census.py --per-slot N`
prints exactly that, and the JSON carries `slot_traffic` per function
(`offset -> {load, store, addr}`):

```
   function                       offset    cause   load  store   addr  total
   main                             -464   alloca      2     51      2     55
   main                             -120   alloca      1     18      1     20
   main                              -96    spill     10      2      0     12
   main                             -280   nongpr      5      7      0     12
   chacha20_core                     -88   alloca      6      5      0     11
   sha256_transform                 -312   alloca      5      2      2      9
```
