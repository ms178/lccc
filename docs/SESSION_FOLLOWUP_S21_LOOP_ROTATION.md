# S21: bottom-testing loops that walk memory, and the dynamic-instruction harness

The Godbolt oracle compares *static* instruction counts at `-O2` and reported
lccc ahead of GCC 16, Clang 23 and ICX on every corpus program
(`tools/oracle/godbolt_oracle.py`, 2026-10-05: totals 1328 vs 1723 / 1913 /
1970). Static counts say nothing about how often an instruction runs, and the
first thing this stretch did was measure the runtime: **the static lead did not
survive contact with Callgrind** on five of the eight oracle programs.

| program | lccc Ir | GCC 14 Ir | lccc/GCC | previous static count (lccc/GCC) |
|---|---|---|---|---|
| bitops | 143,644 | 106,884 | 1.344 | 122 / 128 |
| control | 1,180,662 | 1,077,512 | 1.096 | 168 / 116 |
| float | 7,630 | 7,181 | 1.063 | 155 / 123 |
| int_alu | 5,905 | 4,548 | 1.298 | 92 / 68 |
| loops | 36,319 | 23,657 | 1.535 | 128 / 229 |
| memory | 29,896 | 243,815 | **0.123** | 242 / 579 |
| pixmap | 1,729,175 | 1,520,345 | 1.137 | 256 / 283 |
| strings | 1,121,903 | 880,840 | 1.274 | 165 / 197 |

Counts are executed instructions in the program's own object, libc and the
dynamic loader excluded, with stdout verified byte-identical across all
compilers before any ratio is reported. That table is the honest starting
point: lccc wins `memory` by 8x and loses the byte-walking programs badly.

## 1. The instruments (new, in `tools/oracle/`)

* `callgrind_own_ir.py` — sums Callgrind Ir for one object. `callgrind_annotate`
  alone mixes in `libc` and `ld.so`, which for these kernels is 30-50% of the
  run and hides exactly the differences being chased (its own table attributes
  a program's cost to `__libc_start_main` unless the callee records are walked
  in the right order, so this wrapper filters by object instead).
* `hot_instr.py` — per-instruction attribution. lccc emits no DWARF line
  tables, so `callgrind_annotate --auto=yes` cannot attribute to source; run
  Callgrind with `--dump-instr=yes` and this joins the profile's instruction
  addresses with `objdump -d`. It also summarises by instruction class
  (load / store / lea / control / alu / regmov), which is what turned the
  `strings` investigation from guesswork into arithmetic.
* `callgrind_compare.sh` — builds each oracle program with lccc and the local
  references, runs all of them under Callgrind, refuses to print a ratio when
  stdouts differ, and writes a JSON record (`/home/user/callgrind_s21.json`,
  `/home/user/callgrind_s21_after.json`).

## 2. Defect: the loop test stayed at the top for headers that load

`strings.c` walks three byte strings per outer iteration (`hash`,
`count_byte`, `str_cmp_words`). lccc ran **9 instructions per byte** against
GCC's 6:

```text
lccc                                      GCC
.L7c0: movsbl (%rbx),%r9d                 .L11a0: add    $1,%rcx
       test   %r9b,%r9b                          xor    %rsi,%rax
       je     .L7e0                              movzbl (%rcx),%esi
       add    $1,%rbx                            imul   %rbp,%rax
       movzbl %r9b,%eax                          test   %sil,%sil
       mov    %r15,%rsi                          jne    .L11a0
       xor    %rax,%rsi
       mov    %rsi,%r15
       imul   %r12,%r15
       jmp    .L7c0                   <-- taken, every iteration
```

`loop_invert` exists to fix exactly this and its header documents a measured
~2x on `k_memchr`, but it declined these loops:

```text
[INV] header 10 has a non-duplicable instruction
[INV] header 13 has a non-duplicable instruction
[INV] header 16 has a non-duplicable instruction
```

The refusal is rule 2 ("loads are refused outright") plus rule 3 ("every value
defined in `H` is used only inside `H`"): the header's `Load %b = *%p` *is* the
loop's payload, and the body consumes it, so a **renamed** copy in the latch
would leave the body reading the guard's byte forever. The renaming was never
the point, though — it is only what made rule 3 necessary.

### 2.1 Fix: verbatim duplication when the header walks memory

`src/passes/loop_invert.rs` now has two cloning modes. A header that only
computes keeps the existing fresh-name path (separate live ranges, rule 3
enforced). A header containing non-volatile loads/stores is duplicated
**verbatim** — same destinations, same operands — and rule 3 is skipped
because the copy re-defines the values the body reads.

The correctness argument is that the rotation is not a motion at all, it is a
re-labelling: the header's instruction sequence executes at the same points in
the same order, `n+1` times per entry, whether it sits at the top of the
iteration or at the bottom of it.

```text
original :  H1 B1 H2 B2 ... Hn Bn H(n+1)   exit
rotated  :  H1 B1 H2 B2 ... Hn Bn H(n+1)   exit
            ^guard                 ^latch
```

Every load and store in `H` therefore reads and writes the same addresses, in
the same order, the same number of times; nothing is hoisted, sunk or
speculated. The duplicates are appended *after* phi elimination's edge copies,
so they observe the next iteration's induction variable, and the header's own
terminator becomes the once-executed guard. Volatile accesses, calls, atomics,
inline asm and stack allocations are still refused — the argument above does
not extend to them and no measured loop needs them.

## 3. Measurements after the fix

Oracle programs, Callgrind, same harness (`callgrind_s21_after.json`):

| program | before | after | Δ | vs GCC |
|---|---|---|---|---|
| **strings** | 1,121,903 | **1,012,019** | **−9.79%** | 1.274 → 1.149 |
| bitops, control, float, int_alu, loops, memory, pixmap | — | unchanged | 0% | — |
| total (8 programs) | 4,255,134 | 4,145,250 | −2.58% | |

Real bench harness (`tests/bench/driver.c` + kernel, best-of-5):

| kernel | base Ir | new Ir | Δ | base time | new time |
|---|---|---|---|---|---|
| `k_strcmp_signed` | 87,534,567 | 77,890,022 | **−11.0%** | 5.16 ms | 3.49 ms |
| `k_memchr` | 8,556,061 | 8,556,061 | 0 | 0.580 ms | 0.575 ms |
| `k_adler32` | 5,151,148 | 5,151,148 | 0 | 0.306 ms | 0.296 ms |

Blast radius: of the first 80 TUs under `tests/bench`, `tests/corpus`,
`tests/benchmark`, exactly 5 change assembly (`k_strcmp_signed`,
`k04_strlen`, `k13_strcmp`, `fannkuch`, `linux_rbtree`) — all byte/pointer
walks. The whole change in `k04_strlen` is one line:

```diff
-    jmp .LBB1
+    cmpb $0, (%rsi)
+ jne .LBB2
```

## 4. Verification evidence

* `cargo test --profile fastbuild --lib`: **4135 passed / 0 failed / 7 ignored**
  (4131 before; +5 new pass tests, −1 superseded negative test).
* New/updated pass tests: `a_header_load_is_inverted_verbatim`,
  `a_verbatim_copy_lands_after_the_edge_copies`,
  `a_header_load_escaping_into_the_body_is_inverted_for_memory_but_not_for_pure_values`,
  `a_volatile_header_load_is_not_inverted`,
  `a_header_with_a_call_is_not_inverted`,
  `a_header_with_a_stack_allocation_is_not_inverted`; the pure-header path's
  tests are unchanged and still pass.
* Differential fuzzing against GCC/Clang
  (`scripts/fuzz_diff.py`, all output-verified): `differential` 40/40,
  `phi_cfg` 40/40, `synthetic` 30/30, `stress_suite` 91 cases;
  `unroll_stress.py` 12 configurations, 0 failures.
* `cargo fmt --all -- --check` clean; `scripts/ci_local.sh --fast` — see the
  snapshot commit message for the gate summary.
* Pristine-HEAD A/B: `/tmp/lccc_base` (built from a stashed tree) was used to
  attribute every difference above to this change and nothing else.

## 5. Harness defects fixed on the way (both produced false compiler reports)

1. `scripts/fuzz_diff.py` `stress_suite`: `gen_fp_stress` cases were reported
   as **miscompiles**. They are not: lccc's documented default code-generation
   baseline is x86-64-v3, which includes FMA3, so `u*0.999 + 0.001` is
   contracted into `vfmadd231sd` (even at `-O0`); the reference `/usr/bin/gcc`
   targets baseline x86-64, has no FMA, and cannot contract. Both answers are
   conforming C (`-ffp-contract=fast` is the C default) and differ by 1 ulp.
   Reproducer: `double step(double u){ return u*0.999 + 0.001; }` from
   `u = 0x1.af9e818f741bfp+37` — lccc `0x1.af3002f7808b8p+37`, GCC/Clang
   `0x1.af3002f7808b9p+37`, and `-ffp-contract=off` makes lccc bit-identical.
   The fp_stress generator now pins `-ffp-contract=off` for both compilers so
   the harness measures arithmetic instead of contraction policy.
2. `scripts/unroll_stress.py` reported a **reference** arm that could not run
   (`[gcc-O0] RUN TIMEOUT`) as an lccc crash — `scripts/fuzz_diff.py` surfaced
   it as `[LCCC_CRASH] unroll_stress`. A reference arm that produces no output
   says nothing about the compiler under test; those are now printed as
   `[SKIP] ... reference arm inconclusive` and do not fail the batch. A driver
   that exits 0 without producing an executable (`NO EXECUTABLE`) used to
   abort with a traceback; it is now an unusable arm. `--run-timeout` is
   configurable.

## 6. Follow-ups, with the data that motivates each

1. **Count-bits loop idiom** (`bitops`, lccc/GCC = 1.344). The loop
   `while (x) { x &= x-1; c++; }` is kept as a five-instruction loop
   (`lea / and / add / test / jne`, 16,196 iterations = 56% of that program's
   instructions) although lccc can already lower `IrUnaryOp::Popcount` to the
   `popcnt` instruction on its default target (`__builtin_popcount` compiles to
   a single `popcntl`) and `bit_idioms` recognizes the straight-line SWAR
   form. Folding the loop into `Popcount` would take that program from 1.344 to
   ≈ 0.7 against GCC. Needs a new loop-level folder; `loop_idiom` is
   copy-specific (`loop_invert` is the wrong home: this one needs SSA phis).
2. **Register-allocator copies and separate zero-extension in byte loops**
   (`strings`, now 1.149). The remaining gap to GCC is three instructions per
   byte: `mov %r15,%rsi` + `mov %rsi,%r15` around the xor/multiply chain, and
   `Cast(I8→U8)` materialised as `movzbl %r9b,%eax` next to a `movsbl` load
   instead of one zero-extending `movzbl (%rbx),%eax`. Both are instruction
   selection / coalescing, not loop structure, and would show up in every
   byte- or char-oriented workload.
3. **`-O0` and FMA contraction** (see §5.1): legal, but it makes `-O0` output
   differ from GCC's `-O0` and from lccc's own `-O2` reference flags in
   differential harnesses. Decide explicitly: either document it next to the
   x86-64-v3 default policy, or gate contraction on `-O1`+.
