# Follow-up — 2026-11-13 · code-generation oracle baseline + full code-debt census

Baseline: `ms178/lccc` `main` @ `4c3c081731291900b3966037dab48228b5cd329a`
("Merge pull request #1182").  Deliverable: `ms178-1.patch` (this session's delta)
plus the snapshot ledger under `$LCCC_ARTIFACTS` (default `<workspace>/artifacts`).  Compiler built with
`scripts/build_lccc_fast.sh` (fastbuild profile: `-O1`, no LTO, incremental,
`-j2`).  Host: 2-vCPU Xeon VM, 1.9 GiB RAM + 4 GiB `scripts/ensure_swap.sh`
swap, **no PMU** (`perf` absent) — so every runtime number below is a *screening*
measurement with bootstrap CIs, never a cycle claim.  The i7-14700KF was **not**
measured by anything in this document; that gate (§11 of the programme) stays open.

Toolchains installed from the project's own verified cache
(`scripts/restore_agent_toolchain_cache.py --only rust,clang,gcc,mold,binutils,gdb`,
tag `harness-cache-8`): Rust 1.99.1, Clang 23.1.5, GCC 16.2.0, binutils 2.47,
mold 3.0.0, GDB 18.1.  The Compiler Explorer oracles resolve to `cg162` (GCC
16.2), `cclang2310` (Clang 23.1.0), `cicc2021100` (ICC 2021.10.0) and
`cicxlatest` (ICX channel).

---

## 1. What landed

### L1 — the code-generation oracle was run over the whole corpus (no code change)

`scripts/codegen_oracle.py --rank` (the survey mode derived from `godbolt.py`)
over all 87 kernels in `tests/benchmark/programs`, plus a paired runtime matrix
(`tests/benchmark/run_benchmarks.py`, 11 rounds, 2 warm-ups excluded,
`taskset`-pinned) over the ten workload-derived kernels that stand for
gzip/zlib-ng/expat/SQLite/glibc/Linux.  Evidence pack:
[`engineering/evidence/2026-11-13-codegen-oracle-baseline/`](../engineering/evidence/2026-11-13-codegen-oracle-baseline/README.md).

Headline numbers, recorded with their CIs and their caveats:

| Metric | Value |
|---|---|
| Static: aggregate instruction gap vs best-of-oracle | **2389** insns over 101 functions (72 behind, 9 tied, **20 ahead**) |
| Runtime: LCCC / GCC geometric mean | **1.0029** (arithmetic 1.0060) — a tie, decided per kernel |
| Runtime: LCCC / fastest reference | **1.0537** — Clang is the reference to beat on 8 of 10 kernels |
| Best rows | `expat_utf8_content` **0.873**, `gzip_crc32` **0.920** |
| Worst rows | `zlib_ng_compare256` **1.205**, `gzip_longest_match` **1.185** (both vs Clang) |

**The important result is the root-cause method, not the numbers.** Three of the
five largest static rows dissolved on inspection:

- `libm_round_family:trunc` (15 vs ICX 1): LCCC emits **13** instructions, GCC
  **18**, ICX's "1" is an artifact of that kernel's own `trunc` definition — LCCC
  is the best of the three.  A rank row is a question, never a verdict.
- `libm_round_family:round_family_pass` (82 vs 27): LCCC unrolls 4×, GCC does not;
  per element both emit exactly one `vroundsd` per builtin.  Instruction count is
  not runtime.
- `expat_siphash24:sip24_final` (158 vs 81): real, and the mechanism is a
  `movq %rsi,%rN; jmp` trampoline per switch arm (16 wasted instructions), not a
  switch-lowering failure — the jump table is correct.

### L2 — code-debt census: 45 markers adjudicated, 2 stale claims corrected

`backlog.md` §15 is new and is the session's main written artefact: it
classifies all 45 markers (44 comment `TODO`/`FIXME` + one `unimplemented!`) in `src/` as
STALE / REAL-BUG / REAL-GAP / HAZARD / HISTORY / BY-DESIGN, and gives the
important ones full item format (Why / Where / Plan / Gates / Accept / Risks /
Effort-ROI).

Two markers were **verifiably wrong about the current tree** and are corrected:

- **CD-01** (`src/passes/mod.rs`, the `div_by_const` i686 gate).  The comment
  asked for "a 32-bit-aware variant that uses single-operand `imull` for mulhi".
  That capability exists: `src/backend/i686/codegen/magic_div.rs` +
  `alu.rs`'s `mull` sequences.  The rewritten comment warns explicitly that
  re-enabling the IR pass on i686 would re-introduce the I64-widening sequence
  the backend truncates — i.e. the "fix" is a **miscompile**.
- **CD-02** (`src/ir/lowering/lvalue.rs`).  The comment claimed writes to
  `register … asm("r12")` variables are silently dropped and asked for an
  empty inline-asm input constraint.  That is exactly what
  `expr_assign.rs::lower_assign` has been doing for several sessions; the
  `lvalue` site is the *address-of* path, which is correctly refused.

### L3 — a ratchet so the census cannot decay (`scripts/check_todo_census.py`)

A census re-derived by hand every session rots.  The new gate:

- requires every marker to name its item (`TODO(CD-21):`) or declare
  `TODO(no-item): <reason>`;
- fails when an id is absent from `backlog.md`, or when the item is
  adjudicated closed/fixed/stale (that is how CD-01/CD-02 would have been
  caught);
- pins the 45 historical unqualified markers as per-file **counts** in
  `scripts/todo_census_baseline.txt`, which may only shrink — so line-moving
  refactors do not invalidate it while any new unqualified marker fails;
- counts a marker only when it is *comment text*: the generated Unicode-name
  blob in `frontend/lexer/ucn_name_table.rs` contains real character names with
  the substring `TODO` inside a byte-string literal, and a naive grep measures
  the data instead of the debt;
- ships `--self-test` with 12 mutation cases (each rule is deleted in a fixture
  and the corresponding assertion must fail), which is the convention this repo
  requires of every gate.

Wired into `scripts/ci_local.sh` (fast lane, self-test first) **and** into
`.github/workflows/ci.yml`, because `check_ci_gate_parity.py` refuses a
standalone local gate that no hosted workflow runs.  Both parity gates were
re-run green after the wiring (`CI/local standalone gate parity: PASS (588
commands…)`, `check_ci_workflow_shell: ok`), as were `check_doc_links.py` and
`check_script_imports.py`.

**Not claimed:** `ci_local.sh --fast` was **not** run in this session (standing
instruction from the owner: no local CI).  The gates that were run individually
are named above; the rest of the fast lane is unverified for this commit, and the
snapshot ledger records the tree as `UNGATED` for exactly that reason.

---

## 2. What the measurements say to do next, in order

### CD-21 · unroll constant-trip-count loops that have a data-dependent early exit — **P0**

`zlib_ng_compare256` is 1.205× behind Clang and the mechanism is fully
understood: Clang unrolls the 32-iteration early-exit loop into 32 straight-line
blocks (4 insns per 8 bytes); LCCC keeps it rolled (6 insns per 8 bytes plus a
loop-carried `cmpl`/`jb`).  The same mechanism explains `gzip_longest_match`
(1.185×).  This is the inner-loop shape of `memcmp`/`ZSTD_count`/`compare256`/
`longest_match` — the hottest code in the compression family the programme ranks
golden.

First step is a *census*, not a patch: add an unroller-declined-reason report
(the §57 explainability convention) and sweep the 87 kernels, so the heuristic
change is justified by "how many loops and which" rather than by one benchmark.
Then: constant trip count ≤ N, single data-dependent exit, body below a size
threshold, remainder loop retained, and a register-pressure veto (§25 — never buy
instructions with spills).

### CD-22 · FP/vector constants are re-materialised instead of hoisted — **P0**

Measured: in `libm_round_family`, LCCC issues two constant loads per `copysign`
per element (8 per unrolled iteration); GCC hoists the mask into an XMM register
and issues none inside the loop.  This is load-port pressure on Raptor Lake in
exactly the FP loops (`fir_filter`, `fp_product_map`, `matmul`, `spectral_norm`).

The tempting one-line "fix" is wrong and this session verified why: switching
`copysign` to the `vandnpd` single-mask form saves a rodata line but *costs* a
`vmovsd` in straight-line code, because `vandnpd`'s mask must be in a register.
The win only exists once the constant has a register home.  So the item is a
rematerialisation policy for the allocator ("fold into the memory operand" vs
"hold in register", priced by §25), with the `copysign` form as its visible
symptom — not a patch to `emit_fp_copysign`.

### CD-20 · phi-copy trampolines after jump-table dispatch — **P1**

16 of 158 instructions in `sip24_final`.  Coalesce copy-only blocks into their
predecessor, or let the dispatcher materialise the phi in the dispatch register.

### CD-03 · a call to an `error`-attributed function is accepted silently — **P0 trust**

Four-line reproducer in §15.3: GCC errors, LCCC exits 0 and lowers the call to
`Const(0)`.  The reason the current behaviour exists is sound (the kernel's
`cpucap_is_possible` puts `__compiletime_error` inside `always_inline`), so the
fix is a two-step: mark the call site during lowering, then diagnose only what
survives inlining and DCE.  CD-27 (guarantee `always_inline` chains) is the
prerequisite that makes step 2 compatible with the kernel.

---

## 3. Open issues this session identified but did not fix

| ID | Severity | One line |
|---|---|---|
| CD-04 | P1 | `__builtin_frame_address(n>0)` returns NULL (GCC walks the frame chain). |
| CD-05 | P2 | `operand_to_imm_i64` fails open: a non-constant intrinsic immediate silently becomes 0. |
| CD-06 | P2 | `ExprId` is a pointer address with an ABA workaround on every type-cache hit. |
| CD-07 | P2 | `expand_rept_blocks` still duplicated in the x86 and ARM parsers although a shared, growth-guarded implementation exists. |
| CD-08 | P2 | `FP_*` constants assume glibc/Linux numbering. |
| CD-09 | P2 | `-mgeneral-regs-only` gates only the AArch64 variadic ABI path. |
| CD-24–CD-26 | P2 | `#pragma redefine_extname` external case; `const_eval_global_addr` MemberAccess/Cast bases; `expr_sizeof` nested-deref pointer width. |
| CD-28 | L | Full type checking in `sema/analysis.rs` — the largest item in the census; needs its own register, not a session. |
| CD-29 | P2 | Two independent builtin→type tables (sema vs lowering) that can drift. |
| CD-30–CD-38 | — | Non-x86 backend accuracy items (silent zeros for x86-only intrinsics, ARM W/X `.long`, RISCV `v0.t`, `auipc` label relocation, multi-alternative asm constraints). |
| CD-42 | L | `-m16` still delegates the whole compilation to GCC (accurate marker; the fix is i686 code size). |

---

## 4. Notes for the next agent (hard-won, please keep)

1. **`ensure_swap.sh` before any build.**  `/swapfile` is wiped with the root
   filesystem; the helper recreates exactly 4 GiB and is a no-op when a
   correctly-sized area is active.  Two rustc processes do not fit in 1.9 GiB.
2. **The toolchain cache is the fastest path to a working compiler.**  `python3
   scripts/restore_agent_toolchain_cache.py --only rust,clang,gcc,mold,binutils,gdb`
   then `source <cache-prefix>/env.sh` (the tool prints the prefix it used).  Rust lives at 1.99.1;
   `scripts/build_lccc_fast.sh` finds it through `scripts/rust_toolchain.sh`.
   Symlinking `~/.cargo/bin/{cargo,rustc,…}` to the cache's `bin/` makes every
   other project script work unchanged.
3. **`codegen_oracle.py --rank` needs the `-j4` flag** or the CE requests
   serialize; it took ~2 min for 87 kernels with 4 oracles.  Its `Comparable: no`
   rows must not be ranked against each other.
4. **Wall-clock runs on this host need ≥ 11 rounds and the printed CV.**  Short
   kernels (10–60 ms) showed 5–25 % CV even pinned; only rows whose CI excludes
   1.0 are worth quoting.
5. **Do not "fix" CD-01 by re-enabling `divconst` on i686** (miscompile) and do
   not "fix" CD-22 at `emit_fp_copysign` (costs an instruction).  Both traps are
   documented at the site.
6. **Snapshot after every validated change**: `./scripts/lccc-snapshot.sh
   "<slug>" "<description>"`.  It refuses a tree without a `ci_local.sh` stamp
   unless `LCCC_SNAPSHOT_UNGATED=1` is set — which is what this session used,
   because the owner forbade running local CI.  The ledger records `UNGATED`
   honestly; a session that *may* run CI should produce a gated snapshot.

---

## 5. Addendum — what was also done, and two honest corrections

### L4 — a semantics fixture for the CD-21 unroll, validated on three compilers

`tests/regression/early_exit_compare_bounds.c` (new) pins the exact shape CD-21
proposes to transform: an 8-byte word loop with a data-dependent early return,
checked against a byte-at-a-time reference over **65 589 cases** — every length
0..64, every first-difference position, three difference classes (bit 0, bit 63,
0xff — a wide XOR reports the same byte for all three, a byte-order or ctz/clz
mistake does not), and misaligned operand pairs because an unrolled load may
assume alignment the source does not guarantee.

Validated: LCCC, GCC 16.2 and Clang 23.1.5 each print `ok (65589 cases)`, and the
file passes through `scripts/run_regression_suite.sh` with its A/B differential
against GCC (`PASS=1 FAIL=0`, AB-diff failures 0).  This is the gate CD-21 needs
*before* the transform exists, so the transform can be judged on correctness
rather than on compare256's own fixture alphabet.

### Correction 1 — there is no preprocessor hang (a false alarm, recorded so it is not repeated)

While writing L4, an early draft contained a nested `/* … */` inside a block
comment.  `lccc -E` appeared to hang (> 60 s, three times), and a bisection was
started.  It was **host thrash**, not the compiler: a `cargo check --all-targets
--locked -D warnings` left running in the background had exhausted the 1.9 GiB
of RAM, so *every* process on the box stalled (even `free -h` and `echo` timed
out).  Re-tested on a healthy host, the malformed draft finishes in
milliseconds and diagnoses itself
(`error: expected ';' after expression before 'by'`), exactly as GCC reports its
own parse error.  **No defect claim is made, and none should be derived from this
episode.**

Two operational rules fall out, and they belong in every future session's plan:

1. **Never run `cargo check/test --all-targets` concurrently with anything else
   on this host.**  One such job is enough to stall the whole sandbox; the
   `-j2` bound limits CPU, not the peak RSS of an `--all-targets` build.
2. **Always wrap compiler invocations in `timeout -s KILL`.**  A stalled process
   does not necessarily die on `SIGTERM`, and a foreground `timeout` without
   `-s KILL` can hang a session's tool call for the full harness timeout.

### Correction 2 — the corpus extension did not happen

The instruction was to extend the integrated benchmark section with further
workload excerpts from `ms178/archpkgbuilds`.  A `--filter=blob:none --depth 1`
clone of that repository did not complete within a 282 s budget on this sandbox
(two `raw.githubusercontent.com` fetches by path returned 404 for the assumed
layout, and the GitHub API call died on an SSL EOF), so no source was extracted
and **no kernel was added**.  Fabricating a kernel "inspired by" a project
without its pinned source file and SHA-256 would violate
`tests/benchmark/WORKLOAD_PROVENANCE.md`, which is the one thing that makes the
existing 24 workload-derived kernels usable as evidence.

The existing corpus already covers the projects named in the brief —
gzip (`gzip_crc32`, `gzip_crc32_slice8`, `gzip_longest_match`), zlib-ng
(`adler32`, `compare256`, `slide_hash`), expat (`xml_scan`, `utf8_content`,
`siphash24`), SQLite (`varint`, `vdbe_serial`), Linux (`find_bit`, `list_sort`,
`rbtree`, `xxhash64`, `chacha20`, `sha256`) and glibc (`memcmp`, `strstr`,
`dl_hash`) — 87 kernels in total.  The concrete next candidates, in priority
order for whoever has network budget: `libpng`'s Paeth/Sub row filters
(`pngrutil.c`), `xz`'s range-coder/`lzma_mf` match finder, `openssl`'s
ChaCha20-Poly1305 and AES-CTR blocks, and `curl`'s HTTP header tokenizer.  The
first two are chosen because they extend CD-21's family (early-exit wide loads)
rather than adding another instance of a shape already covered.
