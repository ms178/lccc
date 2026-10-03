# Follow-up 2026-10-03C — rebase onto PR #743, the `va_arg_pack_len` link failure, and the first end-to-end performance attribution

Session scope: rebase the accumulated patch onto the new upstream main
(`070bf71e`, PR #743), fold in all follow-up work, fix what the *real* workload
surfaces, take the first hard end-to-end performance measurement with
Valgrind/Callgrind, and audit upstream's choices as well as this session's own.
Everything below is measured on this host (2 GB RAM + 4 GB swap, Debian 13,
gcc 14.2, Rust 1.99.0, valgrind 3.24.0); no PMU, so instruction counts are
Callgrind's, wall clock is VM screening, and both are labelled as such.

---

## 1. Rebase onto upstream `070bf71e` (PR #743, "Harden CI oracle invocation contracts")

The S13 deliverable (187 885 B, base `9f6cfcf`) applied **clean** to the new
base with `git apply` — PR #743 touches `scripts/ci_local.sh` and
`.github/workflows/ci.yml`, which this patch also touches, but in disjoint
hunks. Rebase commit `8f8ab03f`; snapshot **S14** refreshed the base ref.

What #743 changes (211 → 7 files, +1890/−1234 this time — all CI/oracle
plumbing, no compiler code):

| file | change |
|---|---|
| `.github/workflows/ci.yml` | the two semantic-validation suites now run with the *pinned* tool channels (`ASMDIFF_TEST_AS`/`ASMDIFF_TEST_OBJDUMP`, `ENCDIFF_TEST_OBJDUMP`) and `LCCC_REQUIRE_PINNED_ORACLE=1` |
| `scripts/ci_local.sh` | same, and the suites are **moved after** the `ensure_gas_247.sh` provisioning gate so the pin cannot be silently absent |
| `scripts/check_ci_gate_parity.py` | +891/−534: a third layer ("execution semantics": a gate in a soft step, a hidden `env:` channel, or a workflow whose `on:` never fires is not coverage) on top of the path and invocation layers |
| `scripts/test_ci_gate_parity.py` | contract tests for the above |
| `scripts/ensure_gas_247.sh` | the binutils tarball's **bytes** are now sha256-pinned on every trust path, including a pre-placed `GAS_DL_DIR` cache; whole-digest equality, never a substring |
| `scripts/test_asmdiff.py`, `scripts/test_encdiff.py` | the round-trip leg resolves the pinned objdump explicitly; under `LCCC_REQUIRE_PINNED_ORACLE=1` a missing pin is an error, not a silent skip |

---

## 2. The defect the real workload found: `__lccc_va_arg_pack_len` never folded

### 2.1 Symptom

`tests/workloads/gzip-1.14/run.py` (lccc build leg) failed to link:

```
ccc: error: undefined symbols:
  undefined reference to `__lccc_va_arg_pack_len'
    referenced from lib/libgzip.a(libgzip_a-open-safer.o) (in function `open_safer')
```

`open_safer` is gnulib's, and its `open (file, flags, mode)` call resolves to
glibc's `_FORTIFY_SOURCE` wrapper — an `always_inline` variadic wrapper whose
body is `if (__builtin_va_arg_pack_len () > 1) __chk_fail (); return
__open_alias (__builtin_va_arg_pack ());`.

### 2.2 Root cause (fixed by instrumentation, then re-verified)

The inliner expands two sentinels the lowering emits for those builtins: the
value sentinel is deleted and spliced with the call site's forwarded
arguments, the length sentinel becomes an `I32` copy of their count. The
length rewrite matched in the wrong value space. `CCC inline traces`:

```
analyze wrap_open: vp=[Value(10)] len=[Value(4)]
rewrite: len_values=[Value(4)] extra=1 offset=10
len candidate: dest=Value(14)  shifted=Value(24)      <- never equal
```

The clone already applied `+ value_offset` (`4 + 10 = 14` is the clone's
spelling of the original `Value(4)`), and the matcher then added the offset
*again*: `14 + 10 = 24`. It matched only when `value_offset == 0`, i.e. when
the caller's value space happened to start at zero — so the sentinel call
survived in emitted code at **-O0, -O1, -O2 and -O3**, and the program failed
to link. The value sentinel was unaffected because it shifts the plan value
*once* (`10 + 10 = 20`) and compares against the clone's dest directly.

A second, independent gap: the rewrite was gated on `!extra.is_empty()`, so a
call site that forwards *nothing* also kept the sentinel - but there the count
is a well-defined zero (which is exactly what the wrapper's `!= 0` check must
decide on). Measured with the shared fixture below.

### 2.3 Fix

`src/passes/inline.rs`, one site: match in the clone's value space (shift the
plan values once, compare against the clone's dest), and drop the
`!extra.is_empty()` guard so the zero-forwarded case folds too. +21/−3.

### 2.4 Verification

* **Gate** `tests/regression/check_va_arg_pack_len_folds.sh` (9 rows, green):
  no sentinel in the emitted assembly *or* the linked binary at
  `-O0/-O1/-O2/-O3`; the fixture's runtime rows (one/two/zero forwarded
  arguments → counts 1/2/0, so the wrapper's `> 1` and `!= 0` branches must
  pick the C paths); and a **gcc reference leg** so a lccc-only reading
  cannot satisfy the gate. Registered in `ci_local.sh` and `ci.yml`
  (parity checker: PASS, 136 commands, 12 invocation contracts).
* **Negative control**: pre-fix, the same assertion fails by direct
  observation (`nm -u` showed `U __lccc_va_arg_pack_len`, and the assembly
  contained `call __lccc_va_arg_pack_len@PLT`); post-fix both are empty. The
  gate checks the same two facts, so it would have failed on the old code.
* **Reference compilers**: GCC 14.2 and the oracle set (gcc 16.2, clang
  23.1, icx) fold the check away for every shape tried; the fixture's verdict
  matches gcc's on all four levels.
* **Real workload**: gzip 1.14 now builds, passes its **30/30** upstream
  tests with lccc, and produces **byte-identical** compressed streams to the
  gcc build for every corpus/level combination the harness checks.

Residual risk, unchanged by this fix (documented, not hidden): a wrapper whose
sentinels cannot be spliced (`analyze_va_arg_pack` returns `None`) is not
inlined at all, and because such wrappers are `extern inline`/`gu`-semantics
they have no out-of-line body — that shape still ends in a link error. It is
the pre-existing, deliberate trade-off ("refusing to inline them is a hard
link error" is the upstream design note); this session's fix removes the far
more common *silent* variant of the same failure.

---

## 3. First hard end-to-end performance attribution (gzip 1.14)

### 3.1 Wall clock (VM screening, pinned CPU, 9 rounds, 2 warmups)

| case | lccc ms | gcc ms | ratio |
|---|---:|---:|---:|
| compress-source-l1 | 82.7 | 65.8 | 1.256 |
| compress-source-l6 | 225.4 | 186.3 | 1.210 |
| compress-source-l9 | 568.0 | 490.2 | 1.159 |
| compress-mixed-l6 | 159.7 | 118.6 | 1.346 |
| decompress-source-l6 | 26.7 | 21.7 | 1.233 |

**lccc is 1.24× slower than gcc 14.2 on this workload** (geomean). This is the
first real end-to-end number for the project; it is *not* a PMU claim.

### 3.2 Callgrind instruction counts — deterministic, same input, same output

Both binaries built `-O3 -march=x86-64-v3` by the harness; run on the same
68.5 MB input; outputs **byte-identical** (`cmp` clean), so the comparison is
apples-to-apples:

| build | Ir (executed instructions) | ratio |
|---|---:|---:|
| gzip-lccc | 13 321 404 637 | **1.87×** |
| gzip-gcc | 7 134 793 559 | 1.00 |

Per function (Callgrind, threshold 99.5 %), **with the inlining caveat that
makes naive per-function ratios wrong**: gcc inlines `fill_window` into
`gzip_deflate` (its standalone copy costs 51.5 M where lccc's out-of-line one
costs 1.096 G), so the two must be compared as a pair.

| function | lccc | gcc | ratio | comparable? |
|---|---:|---:|---:|---|
| `longest_match` | 8 385 483 768 (62.9 %) | 4 154 220 522 (58.2 %) | **2.02×** | yes — standalone in both |
| `gzip_deflate` + `fill_window` | 3 374 118 990 | 1 752 535 823 | **1.93×** | yes — as a pair |
| `send_bits` | 539 777 912 | 465 859 093 | 1.16× | yes |
| `ct_tally` | 387 533 400 | 326 214 777 | 1.19× | yes |
| `compress_block` | 461 695 151 | 322 623 132 | 1.43× | yes |
| `crc32_update_no_xor_pclmul` | 74 386 569 | 20 582 717 | **3.62×** | yes — small, crisp target |
| deflate family, summed | 13 148 610 800 | 7 021 453 347 | **1.87×** | yes |

Two secondary observations from the same profile:

* lccc emits `__memcpy_chk_avx_unaligned_erms` (glibc's *fortified* entry)
  where gcc emits `__memcpy_avx_unaligned_erms`; the counts are equal
  (68.5 M vs 68.4 M), so there is no performance consequence — it is the
  A13/A14 synthesised-`_chk` path being visible in a real profile, and worth
  one line in the record because it is the first time that path shows up in
  a workload rather than a fixture.
* lccc's `fill_window` is *not* inlined where gcc's is. That is an inlining
  policy difference on a once-per-block function — it cannot explain a 21×
  per-call cost, which is why the pair comparison above is the honest one.

### 3.3 Instruction-level mechanism (static disassembly diff)

`longest_match`'s hot loop is gzip's hand-unrolled 8-way byte scan
(`while (*++scan == *++match && ...)`). Static size: lccc 173 instructions vs
gcc 115; register-move mix: **lccc 53 `mov` vs gcc 11**.

Per matching byte:

```
gcc      movzbl 0x2(%rax),%r14d ; cmp %r14b,0x2(%rsi) ; jne <exit+len>    3 instr
lccc     lea 0x2(%r8),%rdx ; movzbl (%rdx),%r10d ; cmp 0x2(%r11),%r10b ; je  4 instr
```

lccc materialises `base + k` into a register for each unrolled step and jumps
to a **shared** mismatch exit (so the pointer must exist); gcc keeps one base
register per side, folds the displacement into the load, and uses **per-
position exit labels** whose target encodes the offset, so the pointer is
never materialised on the hot path. One extra instruction per matching byte is
exactly the 2.02× gap on a loop whose match runs are long.

`fill_window` shows the same family of cost — 158 static instructions vs 95,
53 register moves, and gcc additionally vectorises part of it (AVX2
`vpmaxuw`/`vpbroadcastd` where lccc has 8 scalar `movzwl` + 8 `cmovae`); as a
pair with its caller it is 1.93×, and unlike `longest_match` this one has not
been reduced to a single instruction pattern yet — the per-line attribution
needs a `callgrind_annotate` source mapping that lccc's DWARF does not
provide here (attempted with `--dump-instr=yes`; the instruction stream is
recorded, the *source* mapping is not), so the next session should start from
the disassembly comparison rather than from line profiles.

Architectural reading (not a guess — visible in `memory.rs`/`generation.rs`):
lccc's backend is **slot-model based** (every SSA value has an 8-byte frame
slot; `resolve_slot_addr` returns `Direct`/`Indirect`), with register
promotion and folding layered on top. The extra register moves are the price
of that model where the folding does not fire; gcc's code has no such
round-trip because values live in registers unless spilled.

### 3.4 Oracle screen (Godbolt, `scripts/godbolt.py compare`, `-O3 -march=x86-64-v3`)

Instruction counts, lccc vs the four oracles, on ten benchmark kernels:

| kernel | lccc | gcc 16.2 | clang 23.1 | icc 2021.10 | icx |
|---|---:|---:|---:|---:|---:|
| arith_loop | **198** | 424 | 236 | 494 | 251 |
| fib | **30** | 228 | 31 | 152 | 158 |
| matmul | **85** | 108 | 942 | 389 | 131 |
| sieve | **51** | 216 | 337 | 307 | 384 |
| struct_copy | 136 | **94** | 76 | 213 | 82 |
| binary_search | 71 | **51** | 91 | 60 | 90 |
| chacha20_block | 193 | 310 | 257 | 2331 | **193** |
| sha256_transform | 279 | 275 | **267** | 438 | 1145 |
| glibc_memcmp | **203** | 312 | 230 | 320 | 372 |
| qsort | 31 | **26** | 43 | 41 | 52 |

lccc wins or ties six of ten (and on `gzip_crc32`'s hot loop it is best in
field: **14 instructions**, gcc 15, icc 35, clang 49, icx 64 — verified equal
loop bodies by hand). The losses (`struct_copy`, `binary_search`, `qsort`) are
the screen's next targets, and they are the same size (in the tens of
instructions) as everything else, so the *shape* of the code, not its volume,
decides.

### 3.5 Method note (a trap this session fell into, and out of)

A first oracle comparison of `gzip_crc32_update_no_xor` seemed to show lccc
emitting a call where gcc inlined the loop — a 2× gap. It was an **extraction
artifact**: the fixture had been copied without the `static` keyword, which
changed the compiler's own admission rule (`fits_single_call_site_static`
requires `func.is_static`). Re-run with faithful linkage, lccc inlines and
*beats* gcc's count (14 vs 15). Lesson: never compare compiler output on a
mutation of the original source, and use the compiler's own diagnostics
(`CCC_INLINE_DEBUG`) rather than reading tea leaves in assembly.

---

## 4. Red-team audit

### 4.1 Of upstream PR #743

**Agree, and the design is the right shape:**

1. **Pinning the tarball's bytes on every trust path** — a pre-placed
   `GAS_DL_DIR` cache entry was the one channel that could bypass verification,
   and it is exactly the channel a persistent workspace (this one) uses. The
   comparison is whole-digest, not a substring, and their self-test exercises
   prefix lookalikes of the hash itself. This is the correct threat model for
   a solver whose *verdicts* are byte-exact assembly comparisons: an
   unpinned toolchain is an unpinned verdict.
2. **`LCCC_REQUIRE_PINNED_ORACLE=1` turning a silent skip into an error**, with
   the *test* legs (not just the corpus legs) resolving the pinned pair. A
   skipped round-trip test used to look like a pass; now both mirrors pass the
   channel explicitly and the checker enforces the invocation shape.
3. **Ordering**: the semantic-validation suites moved *after* provisioning, so
   the "cannot rot" argument is structural rather than a comment.
4. **The parity checker's third layer (execution semantics)** is the part I
   would single out: path parity and invocation parity can both be satisfied
   by a gate that a soft step, a hidden env channel, or an unfired workflow
   trigger makes unreachable. Enforcing "an active, non-soft step in a
   workflow whose `on:` fires" is what makes the other two layers meaningful.
   The docstring is also honest about its own limits (command substitution,
   `python -c` payloads and encoded strings are out of scope for static
   parity) — that honesty is worth more than a broader regex.

**Where I went looking for a hole and did not find one:** the *integration*
gates pass `--as`/`--objdump` explicitly to the provisioned pair
(`ci_local.sh` 710–750), so the corpus differential's authority is pinned too,
in both mirrors.

**Where I would still go further (minor, measured, not a defect):** `objcopy`
remains PATH-resolved (`LCCC_OBJCOPY`, default `objcopy`). I checked whether it
is verdict-material: it is used as `objcopy -O binary --only-section=.text`
(`encdiff.py:148`, `insndiff.py:146`) — a raw section dump applied *to both
sides* of every differential, so a version difference cancels by construction
and cannot skew a verdict. Recommendation: leave it, or pin it for uniformity
with the same env-channel discipline; either is defensible, and the reasoning
belongs in a comment rather than in a silent default.

### 4.2 Of this session's own work

1. **Is the `va_arg_pack_len` fix complete?** The plan collects the dests of
   *all* length sentinel calls and the rewrite visits every instruction in
   every inlined block, so one wrapper with several `__va_arg_pack_len()`
   uses (one per return path) is covered; `value_offset` is per-call-site, so
   a wrapper inlined at two sites gets two independent shifts. The count is
   static per call site, which is exactly the semantics: the wrapper's
   variadic arguments *are* the call site's extra arguments.
2. **Does folding to a constant lose a dynamic value?** No: the count cannot
   vary at run time for a fixed call site — unless the wrapper's variadic
   arguments were forwarded from *its* caller, which is the non-inlined case
   (`analyze_va_arg_pack` is per function body, and a sentinel value passed
   as an argument to another call is rejected by the use validation).
3. **Did I weaken the "multi-use" validation by touching this code?** No — the
   change is inside the `!plan.len_values.is_empty()` arm; the value-sentinel
   validation above it is untouched.
4. **Is the new gate strong enough to be a gate?** It asserts three
   independent things (codegen, runtime, gcc differential) at four
   optimization levels, and its failure mode is a build error rather than a
   wrong number. Its weak point, stated plainly: the runtime fixture checks
   *counts*, not register-level ABI of the spliced arguments — that is the
   existing synthetic-glibc coverage in the corpus's hands, not this gate's.
5. **The perf story**: the 1.87× instruction ratio is *deterministic* and the
   attribution is measured, not inferred; but Ir is not cycles (a single
   instruction can be a cache miss). The wall-clock ratio (1.24×) is the
   honest bottom line; the instruction ratio is the actionable lead.

---

### 3.6 The repo's own Callgrind A/B harness (`scripts/callgrind_ab.py`)

lccc vs gcc, `-O3 -march=x86-64-v3`, correctness-gated by the harness:

| benchmark | Ir gcc | Ir lccc | lccc/gcc |
|---|---:|---:|---:|
| arith_loop | 1 310 122 025 | 1 020 119 132 | **0.779** |
| fib | 2 814 991 201 | 119 356 | **0.00004 (degenerate — see below)** |
| matmul | 17 380 678 | 17 428 484 | 1.003 |
| sieve | 97 721 568 | 152 099 114 | **1.556** |
| struct_copy | 152 124 091 | 226 121 226 | **1.486** |
| binary_search | 399 723 | 575 469 | **1.440** |
| qsort | 445 382 980 | 464 196 544 | 1.042 |
| glibc_memcmp | 83 918 932 | 88 237 493 | 1.051 |

Two readings that matter:

* **Two independent methods agree on the losers.** `struct_copy` and
  `binary_search` are the same two kernels the Godbolt screen flagged
  (§3.4: 136 vs 94/76 and 71 vs 51/60 static). Different flags, different
  metric, same verdict — the screen is trustworthy as a *triage* tool.
* **Static and dynamic counts can disagree, and the dynamic one wins.**
  `sieve` looks *better* statically (51 instructions vs gcc's 216) but
  executes 1.556× gcc's instructions. Screening cannot replace measurement;
  use the oracle for code shape and Callgrind for cost.
* **`fib` is degenerate and must not be averaged in**: lccc folds the whole
  benchmark at compile time (119 K Ir for the entire program), so the ratio is
  meaningless. Whether the harness should exclude it or the benchmark should
  take a runtime argument is a follow-up for the harness owner; the honest
  statement is that the geomean printed by the harness is not usable as long
  as this row is in it.

---

## 5. Verification status

* `ci_local.sh --fast` on the frozen tree (`d00680ee`): **154 passed, 0
  failed, 5 skipped, ALL GATES GREEN, `CI_EXIT=0`, pass stamp written**
  (`results/ci-fast-final-211648.log`). The five skipped rows are the declared
  slow class, which GitHub CI runs.
* `rustfmt` PASS, `clippy` PASS inside that run, and `clippy` also green in an
  isolated `--only clippy` run.
* The new gate `va-arg-pack-len-folds` PASS inside the full run; a first full
  run went red on `pipefail-sigpipe` with a *defect in this session's own
  gate* (`nm | grep -q` can SIGPIPE the producer under `pipefail`, which would
  have made the check pass exactly when the sentinel is present); fixed in
  `d00680ee`, then re-verified by `--only pipefail-sigpipe` and by the full
  run above. That the repo's own gate caught the new gate is the system
  working as designed.
* `adduser`-free environment repairs this session (after the harness wipe):
  swap 4 GiB, rust 1.99.0 + rustfmt + clippy, `gcc-multilib`,
  `libc6-dev-i386`, `g++-multilib`, `valgrind` 3.24.0.
* New gate green; corpus runner green for the fixture; parity checker PASS.
* gzip 1.14: lccc build completes, 30/30 upstream tests, byte-identical
  streams vs gcc for every case the harness checks.
* Snapshot S15 (`artifacts/`, ledger) carries the rebase + fix + gate.

---

## 6. Follow-ups, priority order

1. **`longest_match` address materialisation (perf, biggest single item).**
   Measured: 2.02× Ir vs gcc, 63 % of lccc's instructions on gzip -6, one
   avoidable `lea` per matching byte in an 8-way unrolled scan, because the
   pointer is materialised for a *shared* mismatch exit. Where to work, with
   the pieces already in the tree: `x86/codegen/memory.rs` has indexed-load
   folding (`try_emit_indexed_load`, `try_emit_ivsr_indexed_load` — IVSR
   already peels a `Cast`+`Add` for `k[idx + 1]`) and
   `backend/generation.rs::build_rematerializable_global_addr_set_for` is the
   in-tree *precedent* for the missing half: a value that is recomputed at its
   use site (an LEA straight into the consumer) instead of being kept live.
   The pointer chain here (`p_k = p_{k-1} + 1`, each `p_k` used by the load
   and by the mismatch exit) is exactly the case IVSR does not recognise
   (the base is another pointer value, not a canonical induction variable)
   and that remat could answer. Order of attack, cheapest first: (a)
   per-position exit labels for the unrolled compare chain — the offsets are
   compile-time constants, so the exit no longer needs the pointer at all
   (this is what gcc's `jne 550d/5513/5519/...` bombs do); (b) extend the
   remat set to `base + const` address values whose only hot use is a load;
   (c) a pointer-IV canonicalisation pass (gcc's `ivopts` equivalent) — the
   largest change and the one that fixes the family at once.
   Arithmetic for the expected win, so the next session can falsify it: the
   scan loop is the majority of `longest_match`'s 8.39 G Ir; removing 1 of 4
   instructions per matching byte is ≈1.2–1.9 G Ir ≈ 9–14 % of lccc's total
   13.3 G, i.e. 1.87× → ≈1.6–1.7× on this workload. Instrument: the
   retained binaries in `results/gzip-artifacts/` +
   `valgrind --tool=callgrind`, exactly as in §3.2.
2. **`fill_window` (1.93× as a caller pair)**: same family; check whether the
   vectorisable max/broadcast shapes are vectoriser gaps or deliberately
   scalar, and whether the non-inlining of a once-per-block function costs
   anything measurable (it cannot explain the per-call gap, but it removes
   the call and the frame).
3. **`crc32_update_no_xor_pclmul` is 3.62× gcc** (74.4 M vs 20.6 M Ir) — small
   in absolute terms (0.56 %) but a self-contained function, so it is the
   cheapest of the perf items to reduce with the oracle: extract the function,
   compare against gcc/clang/icx on Godbolt, and see what the references do
   differently (the earlier screen showed lccc *ahead* on the scalar CRC table
   loop, 14 vs 15 instructions, so this is the PCLMUL path specifically).
4. **Anonymous nested records are super-quadratic in `lowering`** (carried
   over from FOLLOWUP-2026-10-03B §4.2.7; reproducer
   `audit/nested_records_scale.py`) — unchanged, still unfixed.
5. **The four measured losses** - `sieve` (1.556), `struct_copy` (1.486),
   `binary_search` (1.440) and `qsort` (1.042), per §3.6, of which the first
   three are confirmed by two independent methods. Drill per function with
   the oracle's `--function` and Callgrind's per-line annotation (the latter
   needs a compiler whose DWARF `callgrind_annotate` can map to source; gcc
   can serve as the *host* for lccc-generated assembly only after a
   re-assemble, so start from the disassembly diff).
7. **Harness hygiene**: `tests/benchmark/programs/fib.c` (or the A/B
   harness's handling of it) makes the printed geomean meaningless because
   lccc constant-folds the whole benchmark; either exclude the row or give
   the benchmark a runtime input.
6. **Slot-model spill economics**: the 53-vs-11 register-move ratio deserves
   its own investigation once (1) is done; it is the same root cause seen from
   a different angle.
