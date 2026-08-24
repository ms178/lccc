# Session 75 — CFG liveness for the peephole layer, copy coalescing, eight new transforms

Date: 2026-08-24
Base: `daf3f48` (latest `ms178/lccc` main; PRs #222 and #223 = sessions 73/74).
Build: fastbuild profile, Rust 1.98.0, `-j2`, 8 GiB swap.

## 1. The structural change: exact liveness (PF-04, open since session 67)

Every peephole that deletes a definition asks the same question — *is this
register read on any path from here?* Until now it was answered by two
syntactic approximations: a block-local write-before-read scan and a
whole-function "no other mention" test. Both are sound and both are blind to
the most common shape in generated code:

```text
        movq %rdi, %rsi          # prologue: parameter home  (mentions %rdi)
.LBB2:
        leaq 0(,%rbx,4), %rdi    # loop body: %rdi written, never read
```

The whole-function test sees the prologue mention and gives up; the block-local
scan hits the back edge and gives up.

`peephole/passes/liveness.rs` computes the real answer: basic blocks from label
and branch boundaries, a successor graph, and a backward dataflow fixpoint over
the sixteen GP register families. It is conservative where it must be:

* a function is analysed only when it is `.cfi_startproc`-delimited and every
  transfer resolves (an indirect jump, a tail call to a symbol, or a jump table
  makes the whole function `None`, and callers fall back to the old proofs);
* unknown mnemonics READ everything they mention and write nothing;
* inline assembly reads everything;
* `ret` keeps the return value and the callee-saved set live, a `call` reads the
  SysV argument registers plus `%rax` and `%r10` (static chain) and clobbers the
  caller-saved set;
* a sub-register write (`sete %r10b`) reads the rest of the family, so the old
  whole-function proof is kept alongside — it settles exactly the boolean case
  the dataflow has to be pessimistic about. The three proofs are a union; each
  is independently sound.

`FileLiveness::refresh_at` recomputes one function after a rewrite, because a
transform can *extend* a live range and a later query in the same pass must not
see stale data.

Two model bugs were caught by the ABI unit tests while wiring this up, both of
which would have been miscompiles: `%r10` (static chain) was spelled as family
11 instead of 10 in the call-reads set, and a shift's `%cl` count is a fixed
operand that register renaming may not touch.

## 2. New transforms (all default-on, all skip-key gated)

| pass | rewrite | kernel effect |
| --- | --- | --- |
| `copy_coalesce::coalesce_register_copies` | rename the destination family onto the source family and drop `movq %S, %D` when `%S` dies at the copy — retires the parameter shuffle every function entry pays | sum8 14→12, bswp32 13→11, hsh 18→16, cntz 18→16 |
| `dead_writes::eliminate_dead_pure_writes` | delete any instruction whose only effect is writing a dead register (LEA, `movz*`, `setCC`, `cmov`, copies) | bswp32, lz, copy64 |
| `dead_writes::fold_load_test_into_cmp` | `movsbq (%rbx), %rsi; testq %rsi, %rsi` → `cmpb $0, (%rbx)` | strlen 12→10 (ties GCC) |
| `dead_writes::fold_accumulator_roundtrip` | `movq %r8,%rax; xorq %r10,%rax; movq %rax,%r8` → `xorq %r10, %r8` | hsh 16→14 |
| `dead_writes::reuse_redundant_loads` | second load of the same address becomes a register copy; the scan crosses conditional branches and fall-through-only labels | swapmax, isort |
| `flag_peepholes::eliminate_redundant_self_test` | `andl $1, %esi; testq %rsi, %rsi` → `andl $1, %esi` (the logical op already set ZF/SF/PF and cleared CF/OF) | ffs1 18→17 |
| `flag_peepholes::narrow_dead_sign_extension` | `movslq %edx, %r8` → `movl %edx, %r8d` when the upper half is never read, which unblocks the copy folds | lz 9→8 |
| `relay_and_lea` store relay | `movl %A, %D; movl %D, MEM` → `movl %A, MEM` | swapmax, several benchmark programs |

Soundness details worth keeping in mind (each has a unit test):

* **Flags are block-local, and that is now verified, not assumed.**
  `flags_are_block_local` walks the function and requires every flag reader to
  be preceded by a writer in its own block before any flag-lifetime transform
  is applied.
* **Width rules.** A 32-bit logical op zero-extends, so ZF is identical for a
  following `testq` but SF is not (bit 31 vs bit 63): with mismatched widths the
  self-test fold is only applied when every consumer tests ZF.
* **`%rdx` at `ret`.** The liveness model keeps `%rdx` live at a return unless
  the epilogue demonstrably returns a single value in `%rax` (it writes the
  accumulator and its only `%rdx` mention is a plain `movq %rax, %rdx` copy — an
  `__int128` return writes two *different* halves). That is what lets the dead
  duplicate of the return value disappear.
* **Copy coalescing** requires the copy to sit in the straight-line entry run
  (no label, branch, call or return before it), the source to be mentioned
  nowhere else, and the destination to have no implicit reader afterwards
  (`ret`, a call, a tail jump, div/mul traffic, or a shift needing `%cl`).

## 3. Results

### Compiler Explorer oracle (`scripts/godbolt.py`, `-O2`, instructions in the measured function)

| kernel | **LCCC** | GCC 16.2 | Clang 22.1 | ICX | best |
| --- | ---: | ---: | ---: | ---: | --- |
| sum8 | **12** | 57 | 41 | 42 | **LCCC** |
| cntz | **16** | 68 | 46 | 56 | **LCCC** |
| maxv | **19** | 27 | 59 | 63 | **LCCC** |
| dot | **15** | 17 | 42 | 32 | **LCCC** |
| hsh | **14** | 15 | 41 | 51 | **LCCC** |
| copy64 | **9** | 9 | 9 | 9 | tie |
| my_strlen | **10** | 10 | 1 (libcall) | 5 | tie with GCC |
| bswp32 | 12 | 11 | 45 | 62 | GCC by 1 |
| swapmax | 8 | 7 | 7 | 7 | GCC by 1 |
| lz | 8 | 6 | 4 | 7 | Clang |
| crc32k | 28 | 17 | 38 | 68 | GCC |
| ffs1 | 17 | 11 | 14 | 14 | GCC |
| scmp | 25 | 12 | 18 | 16 | GCC |
| isort | 43 | 27 | 23 | 50 | Clang |
| adler8 | 89 | 66 | 75 | 101 | GCC |

LCCC is the best of the four on **six** kernels (two at session start) and ties
on two more. Clang's `my_strlen` "1 instruction" is a `jmp strlen` libcall
substitution, not straight-line code.

### Local corpus and A/B gate

* `scripts/kernel_count.py`: **346 → 325** instructions against GCC's 264
  (session start on this base: 346).
* `scripts/peephole_ab.py` over all 38 programs in `tests/benchmark/programs`,
  same binary with the pass set on vs off: **7060 → 6705 (−5.03 %)**, every
  program equal or better, **identical stdout and exit status everywhere**.
  Largest: zlib_ng_adler32 −31, gzip_crc32 −26, spectral_norm −20, hash_table
  −16, sqlite_varint −13.

## 4. Validation

```text
cargo test --profile fastbuild --lib      1162 passed, 0 failed (36 new unit tests)
build                                      RUSTFLAGS="-D warnings" clean
tests/regression/run_regression.sh         475 passed, 2 skipped, 2 failed
                                           — identical to the unpatched base
                                           (aarch64 cross-gcc absent;
                                           check_fma_dest_coalesce_codegen is
                                           already failing on `daf3f48`)
tests/correctness/run_correctness.py       50/50 vs GCC
scripts/peephole_ab.py                     38/38 behaviour-identical
tests/fuzz/differential_fuzz.py            seeds 0:500 x {O0,O2,O3,Os}
tests/fuzz/phi_cfg_fuzz.py                 seeds 0:150 x {O0,O2,O3}
tests/benchmark/programs vs GCC            37/37 identical output
kernel corpus execution                    15 kernels at -O0/-O1/-O2/-O3/-Os
```

Two shape tests needed attention, both examined rather than silenced:

* `check_gpr_leaf_param_codegen.sh` asserted the exact three-move parallel
  shuffle in `mix6`. Coalescing retires the shuffle completely (16 → 13
  instructions, parameters consumed straight from their ABI registers). The
  script now accepts either form and, when the shuffle IS emitted, still
  requires it to be complete and correctly ordered — a *partial* shuffle would
  mean a clobbered parameter.
* `check_fma_dest_coalesce_codegen.sh` fails identically on the unpatched base;
  it is recorded as a pre-existing gap, not touched here.

A miscompile of my own was caught by the suite during development and is worth
recording: coalescing renamed the family holding a shift count, producing
`shlq %sil, %r8` (`pgo_sections` stopped assembling). `%cl` is a fixed operand;
the rule and its unit test are in `copy_coalesce.rs`.

## 5. Remaining gaps (ranked, with diagnosis)

1. **adler8 89 vs GCC 66** — RA-03: the eight unrolled byte temporaries win the
   registers and `s1`/`s2` spill. Needs next-use-aware eviction, not a peephole.
2. **isort 43 vs 23** — `(j+1)*4` is recomputed with `cltq`/`movslq` chains
   instead of being strength-reduced to a pointer walk (IR-level).
3. **scmp 25 vs 12** — the byte value is re-sign-extended three times
   (`movsbq %dil, %rsi` after `movsbq (%rbx), %rdi`). A "redundant
   re-extension" peephole (the source is already the extension of its own low
   byte) is the fix; designed, not implemented.
4. **crc32k 28 vs 17** — `leaq table(%rip), %rcx` is loop-invariant and must be
   hoisted (LICM). x86-64 cannot fold a rip-relative displacement with an index
   register, so the earlier "SIB fold" idea in the backlog was wrong.
5. **ffs1 17 vs 11**, **lz 8 vs 6** — `movq $1, %rdi` should be `movl $1, %edi`;
   the `cltq` after `lzcntl` is provably redundant (the result is ≤ 32).
6. **Loop rotation.** Every loop pays `cmp; jCC exit; body; jmp header`
   (one extra taken branch per iteration versus GCC's bottom test). Rotation
   trades one static instruction for one dynamic branch — worth doing, but it
   makes the instruction-count proxy worse, so it needs a cycle-level gate
   before it can be judged.
