# FOLLOWUP-2026-09-30 — Arithmetic-producer self-test elimination: reconstruct, prove, gate, measure

Session scope: the harness wiped the whole workspace between sessions (no
`/home/user/lccc`, no `artifacts/`, no toolchain), so this session started with
reconnaissance of what survived: three fragments of
`src/backend/x86/codegen/peephole/passes/flag_peepholes.rs` from a lost session,
quoted in the request. The base is `main` @ `7b3958f6` ("Volatile bitfield
wrong-code fix, align-on-definition, preheader tuning, oracle metric repair,
SIGPIPE-hardened gates"), re-based before any work began.

Everything below is stated as **measured** or **not measured**. Nothing is
claimed because it "should" be faster; where a claim rests on source-level
reasoning rather than a run, the reasoning is shown.

---

## 0. The headline

**The lost work was not recoverable — it was reconstructible, and the
reconstruction is measurably better than the fragment it replaces.**

Two independent searches establish that the code was never pushed:

```
$ git log --all --oneline -S 'saw_non_zf'                    # 2 hits, both older
$ git log --all --oneline -S 'self_test_after_counter_sub_is_removed_for_zf_consumer'
$ git log --all --oneline -S 'Arithmetic producers'          # no hits
$ git branch -r | wc -l                                      # 282 refs searched
```

So the fold was re-derived from the fragments and the surrounding pass
machinery, and three things were improved beyond what the fragments contained:

1. the fragment's `saw_whole_reader` clause is **provably redundant** (proof in
   §2.3) — and removing it lets the arithmetic guard share the *same* predicate
   the width rule already uses, so the two guards cost **one** consumer walk
   instead of two;
2. the guards are ordered cheapest-first, so the expensive flag-flow walk only
   runs for candidates that survived the free filters (compile time is a
   tracked cost, not a free good);
3. the optimisation now has an **end-to-end gate whose own matcher is
   self-tested**, and which was **mutation-tested** against a compiler built
   with the fold reverted — it fails on that compiler with the exact surviving
   instruction pairs (§4.3).

---

## 1. What the fold is

`subl $1, %R; testl %R, %R; jne` — the shape every downward counter loop
compiles to (`while (--n)`, `do { } while (--n)`, `while ((n -= k) > 0)` for
`k < 0`-style loops excluded, see below) — spends a whole instruction
re-establishing flags the subtraction already set.

The pass (`eliminate_redundant_self_test`) already handled the logical
producers (`and`/`or`/`xor`), which reproduce a `test`'s flags exactly. The
reconstruction extends it to the arithmetic producers, which do not:

| flag | `and`/`or`/`xor` then `test` | `sub`/`add`/`inc`/`dec`/`neg` then `test` |
| --- | --- | --- |
| ZF | identical (from the result) | identical (from the result) |
| SF | identical | identical **at equal widths only** (bit 31 vs bit 63) |
| PF | identical | identical |
| CF | both clear it | **diverges**: the arithmetic form takes it from the carry/borrow |
| OF | both clear it | **diverges**: from the signed overflow |
| AF | both clear it | **diverges**: the arithmetic form defines it from the borrow, `test` leaves it alone |

So an arithmetic producer may stand in for the test **only** when every
consumer of those flags reads ZF and nothing else, and that is exactly the
proof `flag_consumers_are_zf_only` supplies.

### 1.1 Why `inc`/`dec` are included rather than excluded

`inc`/`dec` do not write CF at all (SDM): after `decl %R; testl %R, %R`, a
`jb` reads CF from whichever instruction last wrote it, while the `testl` would
have cleared it. That divergence is caught by the same guard — `jb` is not a
ZF predicate, so `saw_non_zf` refuses the fold — and `decl` is precisely the
spelling the i686 backend emits for the counter loop (§6.1), which is why it
belongs in the producer list on both targets.

---

## 2. Soundness argument

### 2.1 The window has no other entry point

The backward scan from the test breaks at `LineInfo::is_barrier()`
(`types.rs`), which includes `LineKind::Label`. Therefore no label can sit
between the producer and the test, so the two lie in one straight-line window:
no branch can enter *at the test* and observe flags the producer never made.
Without that property the fold would be unsound in a case that is easy to
write by hand and impossible to see by eye — pinned by
`self_test_after_sub_is_kept_when_a_branch_enters_at_the_test`.

### 2.2 The consumer walk follows taken edges

`walk_flag_consumers` walks the flag flow, not the text: a conditional jump's
**taken edge** carries the same flags, so a consumer in a block the
fall-through never reaches still counts; an unresolvable target — a tail
branch, an indirect jump — sets `proved = false` rather than guessing. The
guard requires `proved`, so "I could not look everywhere" refuses the fold
instead of licensing it. (Pinned by
`self_test_after_sub_is_kept_when_a_later_edge_reads_carry`.)

### 2.3 Whole-EFLAGS readers are already charged to `saw_non_zf`

The fragment guarded the arithmetic case with
`!facts.saw_non_zf && !facts.saw_whole_reader`. The second clause cannot fire
when the first is satisfied, and the reason is structural:

```rust
// flag_peepholes.rs, walk_flag_consumers
let cc = condition_code_of(t);
match cc {
    Some(cc) => { if !matches!(cc, "e" | "z" | "ne" | "nz") { facts.saw_non_zf = true; } ... }
    None => {
        facts.saw_non_zf = true;                      // <-- every non-cc reader
        if !NON_SF_FLAG_READERS.iter().any(|p| t.starts_with(p)) {
            facts.saw_whole_reader = true;            // <-- implies the line above
        }
    }
}
```

`saw_whole_reader` is only assigned inside the `None` arm, and that arm sets
`saw_non_zf` unconditionally. Hence `saw_whole_reader ⇒ saw_non_zf`, and
`flag_consumers_are_zf_only` (which requires `!saw_non_zf`) already excludes
`lahf`, `pushf*`, inline asm and unknown mnemonics — the reader class that can
observe AF, the one flag a `jcc`/`setcc`/`cmovcc` predicate cannot name. This
is the reason the arithmetic guard and the mismatched-width guard are the same
predicate in the shipped code, and the reason a candidate that is both
arithmetic **and** width-mismatched pays for one walk, not two.

### 2.4 Width rule

A 32-bit producer zero-extends, so ZF survives into a 64-bit `testq`; SF does
not (bit 31 against bit 63). A 64-bit producer under a 32-bit `testl` is never
usable, because the test ignores the upper half that the flag came from. When
the widths differ the fold therefore additionally requires
`flags_are_block_local` — the whole-function check that EFLAGS does not cross a
block boundary, which the narrow `test` used to conceal.

---

## 3. What changed

| File | Change |
| --- | --- |
| `src/backend/x86/codegen/peephole/passes/flag_peepholes.rs` | `TestFlagAgreement` enum + `test_flag_agreement` classifier; the producer list extended to `add/sub/inc/dec/neg` (long and 32-bit forms); one unified ZF-only consumer proof; 8 new unit tests |
| `tests/regression/check_self_test_after_arith.sh` | **new** end-to-end gate: matcher self-test, positive fold check, in-binary negative control, runtime differential against GCC at four optimisation levels |
| `scripts/ci_local.sh` | gate `self-test-after-arith` added to the fast set |
| `.github/workflows/ci.yml` | matching hosted step (keeps `check_ci_gate_parity.py` green: 112 commands) |

### 3.1 Perfected relative to the fragment

* **Named classification.** The fragment's `(prod_wide, is_logical, is_arith)`
  triple of booleans became `Option<(bool, TestFlagAgreement)>` with
  `Exact`/`ZeroOnly` variants: the legality argument is now a name in the type
  system instead of a comment.
* **One proof, not two.** See §2.3. The fragment computed
  `walk_flag_consumers` for the arithmetic guard and `flag_consumers_are_zf_only`
  (which calls the same walk) for the width guard, so an arithmetic producer
  under a mismatched-width test walked the flag flow twice per candidate.
* **Cheapest guard first.** Classification → width → the between-instruction
  scan → the walk. All four are conjunctive, so the reordering is
  behaviour-preserving and only skips work.
* **`saw_consumer` documented as a deliberate choice.** A flags-dead test is
  removed by the dead-flag passes; requiring a consumer here keeps the fold's
  proof obligation explicit.

---

## 4. Evidence

### 4.1 Unit tests (measured)

```
$ cargo test --profile fastbuild --lib -- flag_peephole
test result: ok. 60 passed; 0 failed; 0 ignored; 0 measured; 3839 filtered out
```

Eight are new: `self_test_after_counter_sub_is_removed_for_zf_consumer`,
`..._add_dec_neg_is_removed_for_zf_consumer`,
`..._sub_is_kept_for_carry_overflow_or_sign_consumers` (10 predicates),
`..._sub_is_kept_when_a_later_edge_reads_carry`,
`..._wide_sub_is_kept_under_a_narrow_test`,
`..._sub_is_kept_for_a_whole_flags_reader`,
`..._sub_is_kept_when_the_register_is_touched_between`,
`..._sub_is_kept_when_a_branch_enters_at_the_test`.

### 4.2 Emitted code (measured)

`int countdown(int n) { int c = 0; while (--n) { c++; } return c; }` at `-O2`:

```diff
 .LBB1:
     subl $1, %edi
-    testl %edi, %edi
 je .LBB3
```

One instruction per iteration, in the hot path, on the shape that is the
canonical countdown loop. The same transformation fires for `do { } while
(--n)` and for the unsigned counter.

### 4.3 Mutation test of the gate (measured)

A second compiler was built with the pass reverted (`git stash` of the single
file, `scripts/build_lccc_fast.sh`), and the new gate run against it:

```
$ CCC=…/lccc-fold-OFF bash tests/regression/check_self_test_after_arith.sh
FAIL structural:
  -O2 c_while: redundant self-test survived a ZF-only consumer: subl $1, %edi ;; testl %edi, %edi
  -O2 c_do: redundant self-test survived a ZF-only consumer: subl $1, %edi ;; testl %edi, %edi
  -O2 c_unsigned: redundant self-test survived a ZF-only consumer: subl $1, %edi ;; testl %edi, %edi
```

and against the shipped compiler it passes. A gate that cannot fail is not a
gate; this one was shown to fail on exactly the regression it exists for.

### 4.4 Build reproducibility (measured)

Two independent `scripts/build_lccc_fast.sh` runs of the same tree produced
byte-identical `target/fastbuild/lccc` (`cmp` clean), which is what makes the
A/B in §4.5 an A/B of the *change* and not of the build.

### 4.5 Corpus A/B under Callgrind (measured)

`scripts/callgrind_ab.py` compiles the benchmark corpus twice — once with the
pre-change compiler (`lccc-base`) and once with the shipped one (`lccc-new`) —
checks that both binaries print identical output, then runs both under
Callgrind (`--cache-sim=yes --branch-sim=yes`) and reports simulated
instruction counts, which are deterministic, unlike wall time on a shared
2-vCPU VM with no PMU.

Full table: `engineering/evidence/SELFTEST-ARITH-1/callgrind-corpus-ab.md`
(31 programs, every pair of binaries checked for identical output first).

**Result: one row improves, nothing regresses.**

| benchmark | Ir base | Ir new | new/base |
| --- | ---: | ---: | ---: |
| `zlib_ng_adler32` | 396,349,526 | 383,790,936 | **0.96831** |
| 30 others | — | — | 1.00000 (±1.5e-5 layout noise on five of them) |
| **geomean (all 31)** | | | **0.99896** |

The single improving row is not a coincidence of the metric: it is the
`zlib-ng` adler32 kernel, a workload the request named explicitly, and the
entire difference between the two compilers on that program is **one
instruction**:

```diff
     subl $1, %ebx
     movq %r8, %rdi
     movq %r10, %r9
-    testl %ebx, %ebx
 jne .LBB13
```

One instruction removed from a loop body, executed 12,558,590 times —
12,558,590 / 396,349,526 = 3.17%. That is the whole optimisation, measured
end-to-end on a real extracted workload: **−3.17% executed instructions on
zlib-ng's adler32**, and a zero-delta everywhere else in the corpus.

The ±1.5e-5 rows are *not* the fold: they move by 6–15 instructions out of
117 k–2.9 G (`.p2align` padding shifts the text and changes a downstream
branch-distance decision), which is exactly the noise floor of an
instruction-count metric that includes alignment.

### 4.6 Runtime differential (measured)

The new gate's runtime half builds one program with lccc at `-O0`, `-O1`,
`-O2`, `-O3` and with GCC at `-O2`, and requires identical output over the
counter values that a wrong proof would break: the last iteration, the 32-bit
sign flip at `0x80000000`, and the unsigned wrap points:

```
PASS runtime: lccc -O0/-O1/-O2/-O3 agree with gcc -O2 (2147553620)
```

### 4.7 Oracle survey (measured)

`scripts/codegen_oracle.py --rank` (Godbolt: GCC 16.2, Clang 23.1, ICC 2021.10,
ICX latest) over the workload kernels named in the request, plus
`--all-functions` for the worst row:

```
  gap benchmark         function                  insns  loads store spill brnch call  best
   58 expat_xml_scan    main                        152     35     7    17    34    4  gcc16.2=94  <-- CALLS DIFFER
   -8 expat_xml_scan    expat_utf8_name_length       70      8     0     0    24    0  gcc16.2=78
   43 sqlite_varint    main                        181     35     7    22    30    5  clang=138   <-- CALLS DIFFER
    6 sqlite_varint    sqlite_get_varint           121     10     9     0    17    0  clang=115
   41 zstd_count       main                        132     20     4     8    22    1  gcc16.2=91
```

Read honestly, and with the tool's own warning applied (3 of 7 rows compare
functions with different call counts, so their "gap" is an inlining decision,
not code generation):

* the **extracted kernel** of the expat scan is 8 instructions *ahead* of GCC
  16.2 and 70 instructions where GCC needs 78 — the 58-instruction "gap" in
  that program is entirely in its `main` driver;
* `sqlite_get_varint` is 6 instructions behind Clang (121 vs 115) with the same
  call count — a real, comparable gap and a candidate for the next session;
* nothing here is a claim about speed: instruction count is the weaker signal
  and the project treats it as such (`docs/GODBOLT_ORACLE.md`).

### 4.8 Pre-existing defect cross-check (measured)

The lost session's saved `work/regression.json` recorded a *miscompile*:
`fp_liveness_ptr_deref_alias_negative`, `lccc: '3 1'` against `gcc: '3 3'`.
That probe now lives inside `tests/regression/memcpy_unaligned_load_fwd.c`
(its comment names the case), and on this base it is green at every level:

```
-O0: OK memcpy_unaligned_load_fwd 1d6b8ba3cbf662d5
-O1: OK memcpy_unaligned_load_fwd 1d6b8ba3cbf662d5
-O2: OK memcpy_unaligned_load_fwd 1d6b8ba3cbf662d5
-O3: OK memcpy_unaligned_load_fwd 1d6b8ba3cbf662d5
gcc -O2: OK memcpy_unaligned_load_fwd 1d6b8ba3cbf662d5
```

i.e. the defect recorded in that artifact was fixed upstream between sessions;
it is verified here rather than assumed.

### 4.9 Gate status (measured)

`scripts/ci_local.sh` is the contract, not "the tests I remember":

```
$ CI_LOCAL_JOBS=1 ./scripts/ci_local.sh --fast     # 125 passed, 0 failed, 5 skipped
$ CI_LOCAL_JOBS=1 ./scripts/ci_local.sh --slow     #   6 passed, 0 failed, 0 skipped
pass stamp: target/ci_local.pass (tree 916da294…, full, debian-13)
```

Every gate GitHub runs is green on this tree, the two halves having been run
on the **same content hash**:

| gate | time | note |
| --- | ---: | --- |
| `cargo-test` | 589 s | the required-check unit suite (3839 + 60 tests) |
| `cargo-test-debug-assertions` | 593 s | `debug_assert!` invariants live: shadow-epoch, GVN span locks |
| `rustfmt` / `clippy` | 7 s / 139 s | `-D warnings` clean |
| `regression-corpus-ssa` | 128 s | corpus with `CCC_VALIDATE_SSA=1` |
| `regression-corpus-debug-assertions` | 141 s | corpus on the assertions-on compiler |
| `benchmark-output-oracle` | 366 s | every corpus kernel matches GCC's output |
| `peephole-whitespace-invariance` | 316 s | exhaustive operand matrix |
| `self-test-after-arith` (new) | 26 s | this session's gate |
| `ci-gate-parity` | — | 112 mirrored commands, new gate included |

---

## 5. Not measured (stated, not implied)

* **No hardware counters.** The host has no PMU and no `perf`; there are no
  cycles/IPC numbers in this document and none are implied. Callgrind's `Ir` is
  a deterministic *simulated* instruction count, which is why it is used for
  A/B decisions here, and it is not a wall-clock claim.
* **No end-to-end package A/B in this session.** The `zlib-ng`/`gzip`/`expat`
  workload harness exists (`tests/workloads/`), and the *extracted kernels*
  (`zlib_ng_adler32`, `gzip_crc32`, `expat_xml_scan`, `sqlite_varint`,
  `glibc_memcmp`, …) are part of the corpus A/B in §4.5; the full-project A/B
  was not run in this session and is listed in §6.
* **i686 is not covered by the new fold.** See §6.1 — the i686 peephole is a
  separate implementation that deliberately refuses arithmetic producers, and
  giving it the same proof needs a flag-flow walk it does not have yet.

---

## 6. To-do, in priority order

### 6.1 i686 mirror of the fold (high value, medium effort)

`src/backend/i686/codegen/peephole.rs:7234` (`eliminate_redundant_test_i686`)
does the logical-producer fold with a **purely local** proof (producer adjacent
to the test, same size, same destination text) and its doc comment states the
reason it stops there: *"ADD/SUB/NEG set CF/OF from carry — neither matches
TEST's flag contract."* That is correct as far as it goes — what is missing is
the consumer walk that turns "matches exactly" into "matches on ZF, and only ZF
is read".

Concretely, the counter loop on i686 is still

```
.LBB1:
    decl %edx
    testl %edx, %edx        <-- removable: the `decl` already set ZF
    je .LBB3
```

Design: hoist the x86-64 walk into a shared, backend-neutral module (the
walker only needs `LineStore`/`LineInfo`, a label table, `flags_effect` and
`condition_code_of`) and call it from the i686 pass with the same
`flag_consumers_are_zf_only` predicate. The i686 backend has boot-size gates
(`check_i686_*`, the kernel boot corpus) that will quantify it; the same
`check_self_test_after_arith.sh` structure ports directly (`-m32`).
Do **not** copy the walk — two implementations of a soundness proof is two
places to drift.

### 6.2 Shift producers (low effort, same proof)

`shl`/`shr`/`sar` write ZF/SF/PF from the result and leak CF/OF from the bits
shifted out — the same ZF-only situation as the arithmetic forms, so
`shll $1, %esi; testl %esi, %esi; je` is foldable with the identical guard.
One entry in the `ARITHMETIC`-style table plus a test; the walk is already
there. Check `shl $0`-style immediates and count-in-`%cl` forms against the SDM
before enabling.

### 6.3 Counted-loop closing (highest ceiling, highest effort)

The §4.2 reproducer still emits a rolled loop, while GCC 16.2 closes it
completely:

```
GCC:    leal -1(%rdi), %eax ; ret
LCCC:   .LBB1: subl $1,%edi ; je .LBB3 ; .LBB2: addl $1,%esi ; jmp .LBB1
```

GCC recognises `c++` over the countdown as an affine function of `n` and
deletes the loop. That is a ScalarEvolution-style trip-count/induction
analysis, not a peephole, and it is the largest *visible* codegen gap in this
session's reproducer. This is where the next big win is.

### 6.4 Loop rotation (medium value, low risk)

Every lccc loop in this session's output ends in an unconditional `jmp` back to
the header. Rotating the loop (test at the bottom, entry guard in the
preheader) removes one taken branch per iteration on every loop in the corpus.
`peephole/passes/loop_trampoline.rs` and the loop-preheader pass exist; the
question to answer first is *why* rotation does not fire on these shapes.

### 6.5 Oracle survey for the next gap

`scripts/codegen_oracle.py --rank tests/benchmark/programs/*.c` (worst
instruction-count gaps against GCC 16.2 / Clang 23.1 / ICC / ICX) is the
standing "find the next big win" instrument; run it at the start of the next
session and take the top rows in order, rather than starting from this list.

### 6.6 A corpus kernel for the counter-loop class (small, undone on purpose)

The fold fires in one of the 31 `callgrind_ab.py` fast-corpus programs
(`zlib_ng_adler32`, §4.5), and in none of the 51 `tests/benchmark/programs`
at the *asm-diff* level — the corpus simply does not contain the countdown
idiom outside adler32. A dedicated kernel (`self_test_counter.c`: signed and
unsigned downward counters, a do-while form, and the sign-carrying control)
would put the class in the standing A/B instrument instead of relying on
adler32 happening to contain it. It is **deliberately not** in this patch:
`check_benchmark_outputs.sh` globs the whole directory and the slow half had
already been run to a `mode=full` stamp, so adding a program afterwards would
have invalidated the stamp that makes this patch delivery-grade. Do it as the
first commit of the next session, together with its `DEFAULT_FAST` entry in
`scripts/callgrind_ab.py`.

### 6.7 Workload-level A/B

`tests/workloads/{gzip-1.14,zlib-ng-2.3.3}` and the `expat`/`sqlite` kernels
already exist with provenance and pinned digests. The open item is a *paired*
A/B of a full package build between two compiler revisions (`lccc-base` vs
`lccc-new` in this session's naming), with the same deterministic inputs, so
that peephole-level wins can be seen — or honestly reported as lost — at
package granularity.

### 6.8 Already done, do not redo

* **Linker oracles.** `tools/linker/setup_oracles.sh` already pins exactly the
  requested set — bfd 2.47, mold 2.42.1, LLVM lld 23.1.x, `wild` at git HEAD —
  and already uses the mold preset I went looking for independently:
  `-DMOLD_TARGETS='X86_64;I386'` (mold's `CMakeLists.txt` instantiates every
  template source once per listed target;
  the comment marks it "strongly discouraged … unless you build mold
  frequently for your personal use", which is this host's case, and `X86_64`
  must stay first because `MOLD_FIRST_TARGET` is the default target of the
  built binary). No change needed; verified, not assumed.
* **The workload corpus.** `gzip`, `zlib-ng`, `expat`, `sqlite`, the Linux
  find-bit path and `glibc`'s `memcmp` are already extracted with source
  digests and licences (`tests/benchmark/WORKLOAD_PROVENANCE.md`), so the
  "extend the corpus" item from earlier sessions is closed for those projects;
  §6.6 and §6.7 are the remaining, different work.

### 6.9 Harness continuity (process, keep doing it)

This session began with a full workspace wipe and recovered with
`scripts/arena_session_restore.sh`, which is the supported path: it recreates
the swap file (8 GiB — now active, `vm.swappiness=20`), resolves the stable
Rust channel from `rust-toolchain.toml` into the persisted
`/home/user/.cargo`, recreates any missing rustup shims, restores the
executable bits and the `.git` directory from the published bundle, and then
builds `target/fastbuild/lccc`.

Snapshot naming: `scripts/lccc-snapshot.sh "<slug>" "<description>"` with
`LCCC_BASE_REF=<upstream main>` and `LCCC_SNAPSHOT_ALLOW_PARTIAL=1` until both
CI halves are green, then again without it — the first (S01) is a safety copy,
the second is the delivery candidate. The canonical deliverable stays
`/home/user/ms178-1.patch`; the dated copy in the workspace root
(`ms178-1.S01-<slug>.patch`) exists so a human can tell two saves apart
without opening them. The ledger records `ci_gate` per snapshot, so a partially
validated tree can never be mistaken for a delivery candidate later.
