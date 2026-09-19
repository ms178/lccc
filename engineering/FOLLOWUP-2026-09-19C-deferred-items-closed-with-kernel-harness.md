# Follow-up: the five deferred items, closed — with the kernel harness installed and run

Date: 2026-09-19 (session 57)
Base: `407eb492` (merge of PR #557). This session's work sits on top of the
round-2 commit described in
`engineering/FOLLOWUP-2026-09-19B-post557-perf-and-flake.md`, whose
"Follow-ups not taken" list is the input to this one. All five are now taken.
Two further defects were found by actually running the harness rather than
reading it.

## 0. The kernel harness was installed and run, not reasoned about

Round 2 deferred three items with the phrase "needs the kernel harness to
validate". This session installed it and ran it, so those items are closed on
measurement instead of argument.

Environment work (nothing here was previously present):

| Piece | Before | After |
| --- | --- | --- |
| `flex`, `bison`, `bc`, `cpio`, `pahole`, `rsync`, `kmod`, `zstd`, `lz4` | missing | installed |
| `qemu-system-x86_64` | missing | installed |
| `ld.lld` (second link oracle), `clang` (size oracle) | missing | installed |
| `gcc -m32` / `g++ -m32` | **link failure** (no 32-bit CRT) | working |
| `shellcheck` | missing | installed |
| kernel tree | absent | real `linux-cachymod-6.18.52`, 28/28 CachyMod patches, the package's own `config`, `make prepare`, all canaries |
| `ms178/archpkgbuilds` | absent | sparse checkout of `packages/linux-cachymod-6.18` |

The tree comes from `scripts/prepare_kernel_tree.sh` — the package recipe path,
not the kconfigless approximation in `scripts/boot_kconfigless_stage.sh`, which
exists only for hosts without `flex`/`bison`. `gcc -m32` mattered: three
regression-corpus entries had been reported as pre-existing failures for want
of a 32-bit oracle (see §8).

### Boot gate, before and after every source change in this session

`scripts/build_kernel_boot.sh` compiles all 23 `arch/x86/boot` objects with
lccc, links `setup.elf` with lccc-ld against the KEEP-annotated `setup.ld`, and
then re-links the *same objects* with `ld.bfd` and `ld.lld` and compares the
flat images byte for byte:

| | pre-change | post-change |
| --- | --- | --- |
| 32 KiB gate | PASS, `_end=31168`, headroom 1600 B | PASS, `_end=31168`, headroom 1600 B |
| `.text` total | 22890 B | 22890 B |
| `ld.bfd` flat image | byte-identical | byte-identical |
| `ld.lld` flat image | byte-identical | byte-identical |
| `setup.bin` sha256 | `bf9f0e9f5f199924b8308b4879c2eb44be98f28defe145a7d60ac72f29cffd53` | **identical** |

The three peephole changes below are codegen changes, so "the boot image did
not move" is a real result and not a vacuous one: the new kill shapes simply do
not occur in the real-mode closure. That is the cheapest possible proof that
they did not perturb a working boot stage.

### What was *not* run, and why that is not a shortcut

`scripts/qemu_boot_test.sh` boots an lccc-built `bzImage`. Producing one needs
`scripts/build_kernel_full.sh` over the whole kernel, which its own header
documents as blocked on three backend gaps — lccc does not emit `-pg`/
`__fentry__`, does not emit objects objtool accepts, and does not emit the
`gs`-based stack canary — so the harness runs with tracing, objtool and
stack-protector disabled and still cannot reach `bzImage` on this host (2 CPUs,
2 GB RAM + 6 GB swap). The boot-stage gate above is the harness's own
pre-`bzImage` checkpoint: `setup.ld` asserts `_end <= 0x8000`, and that assert
is what must pass before a `bzImage` can exist at all.

Instead of stopping there, the changed compiler was pointed at real 64-bit
kernel code. `mm/page_alloc.c` (7764 lines) compiles cleanly with lccc under
the kernel's own include and ISA flags — not a hand-written approximation of
them: `scripts/repro_claims.sh` replays the invocation Kbuild itself recorded in
`mm/.page_alloc.o.cmd`, dropping only the flag families this backend rejects
(`-pg`, `-mrecord-mcount`, `-fno-allow-store-data-races`, the stack protector and
its `%gs` guard, `-fpatchable-function-entry`, objtool's `-Wp,-MMD`) and keeping
everything else, including `-mfunction-return=thunk-extern`,
`-mindirect-branch=thunk-extern` and the retpoline switches — and produces a
**141592-byte object**. gcc's own object for the same TU is 2121560 B, because
this config carries `CONFIG_DEBUG_INFO` DWARF, which this backend does not emit;
the two sizes are not comparable, and only the lccc one is a codegen-stability
signal. This figure has a history worth keeping, because each step was a defect
found by re-measuring rather than by reading. An earlier revision quoted
**136896 B**, measured before the peephole work in §3–§5 landed; it no longer
reproduces. The first re-measurement gave **141504 B**, which was also wrong: the
replay tokenized Kbuild's recorded line with `read -r -a`, which hands
`-DKBUILD_MODNAME=\"skbuff\"` to the compiler with its backslashes intact, so the
define arrived mangled (and, on eight other TUs, made `printk_index_wrap` expand to
a multi-character constant and killed the parse outright — a harness bug wearing a
regression's clothes, since it failed identically under both compilers). With
shell-correct tokenization the faithful figure is **141592 B**, and that is what
claim 4 now pins; it fails loudly if codegen moves it. A 37-TU sweep across `mm/`,
`kernel/sched/`, `fs/`, `net/`, `lib/`, `crypto/`, `block/` and `drivers/base/`
A/Bs the pre-change and post-change compilers per object (§8).

## 1. `Ret` is now a scan terminator in `rcx_is_live_at` (round-2 item 1)

`rcx_is_live_at` decides whether the address copy in

```asm
    movq %rdi, %rcx
    movq (%rcx), %rax
```

can be folded to `movq (%rdi), %rax`. It scans forward for the first line that
kills or uses `%rcx`, treating labels, jumps and calls as conservative
barriers. `ret` was in neither set: a bare `ret` names no register, so the
pre-scanned family bitmask skipped it and the walk continued *past a block
terminator* into whatever text followed — typically the next function's label,
which then answered "live". Every leaf-shaped return lost the fold.

Control cannot fall through a `ret`, and a path that does reach the following
code entered it through a branch; such a path cannot have executed the copy
being folded away, because reaching post-`ret` code from the copy would require
a label or a jump between them, and both are already barriers. So `Ret` returns
`false` (dead), checked before the bitmask filter.

This is not new reasoning in the crate: `dead_code.rs`'s dead-move scan already
retires `%rcx` at a return for exactly this reason ("a return cannot observe
non-result caller-saved registers … lets copy propagation finish folds such as
`movq %rax,%rcx; movsbl (%rcx),%eax` at a leaf return"), and
`LineInfo::is_barrier` classifies `Ret` as a terminator. The walk was the one
place that disagreed with both.

Tests: `test_rcx_address_copy_ret_ends_the_region` (fold fires, and the *next*
function's read of the incoming `%rcx` survives — the soundness half) and
`test_rcx_address_copy_use_before_ret_still_blocks` (a use before the `ret`
still blocks it).

## 2. `popq %rcx` is a full redefinition (round-2 item 2)

`dest_operand_is_full_width` took the destination as "text after the last
comma". `popq %rcx` has no comma, so the whole string `"popq %rcx"` was
compared against `%rcx`/`%ecx` and the width test reported a partial write —
i.e. `writes_family_full` answered *false* for an unconditional 64-bit
redefinition.

Two changes, deliberately separated:

* **The shared predicate is exact on one tier and a name union on the other.**
  `dest_operand_is_full_width` (`passes/helpers.rs:505-511`) takes the
  `LineInfo`. For `LineKind::Pop { reg }` it answers `reg == fam` — a
  classification predicate, and exact, because the x86-64 classifier produces
  that kind only for the `popq ` spelling (always a full 64-bit write of the
  popped register), while `popfq`/`popfl` carry `REG_NONE` and `popl`/`popw`
  fall through to `Other { dest_reg: REG_NONE }`: `parse_dest_reg_fast`
  (`types.rs:831-887`) returns `REG_NONE` for a comma-less line that is not one
  of the in-place modifiers it knows, so those spellings are conservatively
  partial rather than misread as full writes. For every other line the predicate
  is **not** a width analysis: it compares the text after the last top-level
  comma against exactly two spellings, `REG_NAMES[0][fam]` (`%rcx`) and
  `REG_NAMES[1][fam]` (`%ecx`). That union answers false for `%cx`, `%cl`, `%ch`
  and for any decorated destination — the same conservative answer a width
  analysis would give, but reached textually. An earlier revision of this
  section called the whole predicate exact; the `Pop` arm is, the fallback is a
  two-spelling union, and that distinction is what the external review's F2 was
  pointing at. No mnemonic sniffing, no generic "strip the mnemonic when there
  is no comma" rule — that would also promote `notq %rcx`, `negq %rcx` and
  `bswapq %rcx`, whose single operand *is* the destination but whose result
  depends on the old value.
* **The liveness walk gets an explicit arm.** In `rcx_is_live_at` the
  conjunction also contains `!is_read_modify_write(t)`, and that helper answers
  `true` for every mnemonic it does not know — `pop` included — so the exact
  `writes_family_full` answer was still vetoed. The walk now matches
  `LineKind::Pop { reg: 1 }` directly. The arm is exact because `Pop { reg }`
  carries `reg_refs = 1 << reg`, so the family bitmask upstream admits only the
  family itself.

`is_read_modify_write` was deliberately **not** changed. Its conservative
default for `pop` is load-bearing elsewhere: `pop` shifts `%rsp`, and the
slot-offset passes (`memory_fold.rs` consults it twice, alongside
`is_rsp_shift_line`) must stop there. Making `pop` "write-only" globally to save
one `matches!` in one walk would trade a local precision gain for a soundness
risk in three other passes. The eight call sites are listed in §9.

Honest scope note: the `writes_family_full` half is **exactness-only** today,
and its four callers are not gated by one kind of test. Three gates are
classification-based: `dead_code.rs` breaks on `is_barrier`, and
`LineKind::Pop { .. }` is a barrier member (`types.rs:222`); `local_patterns.rs`'s
copy-swap requires `LineKind::Other`, which a `popq` line is not; and the rcx
walk matches `LineKind::Pop { reg: 1 }` directly. The fourth is textual:
`flag_peepholes.rs` breaks on `mentions_exact_name`
(`flag_peepholes.rs:1756-1766`), a byte-substring scan for the 64-bit spelling
with an alphanumeric-boundary check, not a predicate over the classification.
The guard is therefore the *union* of a kind test and a name-mention test; an
earlier revision of this section described all four as predicates, which is the
second half of F2 and is corrected here. The fix is
still worth having: the predicate's documented contract is "proves every
consumer of the old value unreachable", and a shared acceptance-grade predicate
that answers false for `popq %rcx` is a trap for the next caller. It is unit
tested in both directions (`popq_names_its_single_operand_as_a_full_width_destination`,
`pop_lookalikes_are_not_full_writes`).

One test expectation in that pair was wrong on the first draft and is worth
recording: `popfq`/`popfl` **do** fully redefine `%rsp`. The implicit-operand
table says so (`b"popf" | b"popfq" | b"popfw" | b"popfl" => (RSP, RSP)`), so
the test now asserts family 4 is killed and every other GP family is not —
asserting "popf writes no register" would have been a test that is wrong about
the ISA.

## 3. `xorl %ecx, %ecx` is a kill (round-2 item 3, and #557's own item 3)

The canonical zeroing idiom was vetoed twice over: the family appears on the
source side (`!mentions_rcx_family(source)` fails) and `xor` classifies as
read-modify-write (`!is_read_modify_write(t)` fails). Both vetoes exist to
protect against a result that *depends* on the old value — and for
`xor r, r` the result is 0 regardless.

New shared predicate `self_zeroing_full_write(trimmed, fam)` in `helpers.rs`,
exact by construction:

* `xor`/`sub` only. `and r,r` and `or r,r` preserve the old value; `adc`/`sbb`
  also read CF. None of them writes a constant.
* the mnemonic must be exactly four bytes ending in `l` or `q`. A 32-bit write
  zero-extends and retires the whole family; `xorw`/`xorb` leave the surviving
  bits observable. The length test also rejects `xorps`/`xorpd`/`subss`/`subpd`
  and the suffix-less `xor`.
* both operand tokens must be identical and must name `fam` at 32 or 64 bits,
  so `xorl %eax, %ecx` (a real RMW of `%ecx`), `xorl $0, %ecx` and
  `xorl (%rax), %eax` are all refused. A first-comma split is safe here: a SIB
  source such as `xorl (%rax,%rcx), %eax` yields a destination token that is
  not a bare register name and is rejected.
* the first byte is checked before anything else, because the predicate sits on
  the fall-through path of the kill test — it runs for every family *use*, the
  common case.

Tests: `self_zeroing_idiom_is_an_exact_full_kill` (positive matrix: 2 idioms ×
2 full widths × 16 GP families, each also asserted against the other 15),
`self_zeroing_rejects_every_non_constant_or_partial_form` (28 rejected forms ×
16 families, plus out-of-range families), and four end-to-end folds
(`xor`/`sub` fire; `xorw`, `xorl %eax,%ecx`, `andl %ecx,%ecx` and
`orq %rcx,%rcx` do not).

## 4. `CCC_TRACE_NOTES` is read once, not per emitted instruction (round-2 item 4)

`note_home_written` ran `std::env::var_os("CCC_TRACE_NOTES")` on every homed
definition — effectively per emitted instruction — and `note_reg_clobbered` did
the same per clobber. Each call is a `getenv` walk of `environ` plus an
`OsString` clone of the value, paid by every build that never sets the
variable.

Now a `LazyLock<bool>` behind `trace_notes_enabled()`, following the crate's two
existing precedents for exactly this (`i686::codegen::emit::const_stack_arg_disabled`,
whose comment makes the same argument, and `split_ranges::split_debug_enabled`).
The environment cannot change mid-process in the driver, so caching is exact
rather than an approximation. In `note_reg_clobbered` the cheap test now comes
first (`!stale.is_empty() && trace_notes_enabled()`): the eviction list is empty
for most clobbers and only a non-empty one can print.

## 5. The vectorizer's ISA gate is answered before the loop analysis (round-2 item 5)

`vectorize_with_analysis_mode` built the natural-loop forest and *then* asked
whether the target had a SIMD register file at all. Under `-mno-sse` the answer
is no and the forest is discarded. Round 2 left this alone because the
`LCCC_DEBUG_VECTORIZE` header line reports `loops.len()`, so hoisting the gate
looked like it would change observable output.

It does not, and the reason it does not is now the design: the gate has two
exits. The fast one (`!simd_ok && !debug`) returns before the analysis; the
traced one (`debug`) still computes the forest, prints the header with the real
loop count, and then prints the refusal — same lines, same order. The refusal
text moved into one helper (`report_simd_disabled`) precisely so the two exits
cannot drift.

Measured waste, on one real TU (`mm/page_alloc.c`, kernel SIMD flag set, traced
with `LCCC_DEBUG_VECTORIZE=1` and counted by claim 6 of
`scripts/repro_claims.sh`): **1628 entries into the traced path, 1628 refusals**
— 3256 trace lines over 586 distinct functions and 623 distinct
(function, CFG shape) pairs, 350 of those entries carrying at least one loop.
Every single entry ends in the ISA refusal: a 100% waste rate, per function, per
TU, for every non-FPU translation unit of a kernel whose `KBUILD_CFLAGS` disable
SSE globally. An earlier revision quoted "814 entries, 814 refusals"; that figure
was measured on a pre-hoist compiler with a different trace grammar and does not
reproduce on this tree, so what the script asserts is the invariant
(`entries == refusals`, exactly) and reports the absolute count next to it,
because the count follows the kernel config rather than the patch.

Verification, in increasing order of strength:

1. **Byte-identity against the pre-hoist compiler.** All five environment
   combinations — debug/SIMD-off, debug/SIMD-on, why-not/SIMD-off, both
   variables/SIMD-off, neither variable — produce identical stderr (18, 1526,
   9, 18 and 0 lines respectively) *and* identical emitted assembly.
2. **A permanent pin** in `tests/regression/check_vectorize_isa_gate.sh`
   (section 6, run by both `scripts/ci_local.sh` and `.github/workflows/ci.yml`):
   header/refusal pairing by function name and order, one refusal per header,
   nothing else printed, a non-zero loop count (so a future hoist cannot
   silently skip the analysis under trace), why-not-only printing no headers,
   both variables agreeing on the refusal count, and silence with neither.
   The pin was mutation-tested: reordering the pairs, collapsing every loop
   count to 0, dropping the header lines, injecting a stray line and mismatching
   a function name are all caught, while the real trace passes clean.
3. **An interleaved end-to-end A/B**, six rounds alternating the two binaries on
   the same TU (alternating cancels thermal and page-cache drift, which a
   run-all-A-then-all-B measurement does not):

   | | median | mean | stdev | min | max |
   | --- | --- | --- | --- | --- | --- |
   | pre-change | 13193 ms | 13281 ms | 1029 | 12093 | 14800 |
   | post-change | 12850 ms | 13013 ms | 912 | 12057 | 14640 |

   Paired delta: median **−146 ms (−1.1%)**, mean −267 ms, four of six rounds
   faster. With a per-run stdev of ~7.5% this is *directionally* favourable and
   **below this host's noise floor**; the defensible claim is the one in (1) and
   the census in the paragraph above — 1628 CFG analyses per TU removed — not a
   percentage. The object file is byte-identical with SIMD off *and* with SIMD
   on, so nothing was traded for it.

   Two corrections, both found by re-measuring rather than by reading. First,
   those 13 s medians were produced by the mis-tokenized flag replay described in
   §0: with Kbuild's command handed to the compiler faithfully, the same TU takes
   ~33 s, so the table above measures a lighter compilation than the one it
   claims to measure (the *direction* survives; the magnitudes do not). Second,
   the A/B was later re-run three-way and order-rotated across the two commits of
   this patch, and it caught a regression the earlier table could not see:

   | binary | median | vs `origin/main` | object |
   | --- | --- | --- | --- |
   | `origin/main` (`56858cb`) | 33343 ms | — | 141592 B |
   | `2a675b3` (this section's hoist + env hygiene) | 32982 ms | **−1.1%** | 141592 B |
   | `400ea84` (the trailing-whitespace work) as committed | 40881 ms | **+24%** | 141592 B |
   | `400ea84` with the redundant per-access trim removed | 33269 ms | −0.2% | 141592 B |

   The whitespace work had put a `str::trim_end()` into `LineInfo::trimmed`, which
   473 call sites invoke inside the peephole's fixed-point loop; decoding UTF-8
   backwards through `char::is_whitespace` at that volume cost 7.8 s, an ASCII
   byte scan still cost 1.9 s, and removing the trim entirely cost nothing —
   because `LineStore`'s only two writers (`new`, `replace`) already strip
   trailing blanks, so no pass can observe them and the per-access trim was
   unreachable work. The object is byte-identical at every row. This is recorded
   here rather than quietly amended: a REPORT-class timing claim, read as "noise",
   was the instrument that found a 24% regression in a commit that had already
   passed every correctness gate.

Also in this function: `std::env::var(..).is_ok()` became `var_os(..).is_some()`,
the crate's presence-means-on spelling (`vec_interleave.rs` already uses it for
these same two variables). `var` UTF-8-validates and allocates a `String` for a
value nobody reads, and rejects a non-UTF-8 value that presence-semantics should
accept.

## 6. New defect: the documented `CC_ORACLE=clang` size oracle could not run

`scripts/boot_size_oracle.sh` documents three invocations, two of them clang:

```
CC_ORACLE=clang scripts/boot_size_oracle.sh           # clang oracle
CC_ORACLE="clang gcc" scripts/boot_size_oracle.sh     # both
```

Both died before compiling a single object:

```
clang: error: unknown argument: '-mpreferred-stack-boundary=2'
```

`LCCC_BOOT_CFLAGS` is the GCC/lccc line, and `-mpreferred-stack-boundary` is
GCC-only. The fix is not an invented substitution: `arch/x86/Makefile` in the
kernel being compiled already states the mapping —
`cc_stack_align4 := -mpreferred-stack-boundary=2` under `CONFIG_CC_IS_GCC` and
`:= -mstack-alignment=4` under `CONFIG_CC_IS_CLANG` — and the same Makefile
appends `-Wno-gnu` to `REALMODE_CFLAGS` for clang. So `scripts/boot_flags.sh`,
which its own header calls the single source of truth for these command lines,
gained `lccc_boot_cflags_for <cc>`, and `compile_set` asks it for the spelling
that matches the compiler in hand. Unknown compilers keep the canonical line
untouched, so the lccc side of the A/B is bit-for-bit what it was (verified:
`lccc_boot_cflags_for gcc` is string-equal to `LCCC_BOOT_CFLAGS`).

Verified against the real tree: clang 19 compiles all 24 `arch/x86/boot`
objects — 20 C translation units plus the four assembly files (`bioscall.S`,
`copy.S`, `header.S`, `pmjump.S`); the directory's other two `.c` files are
`apm.c`, which this config leaves out of the image, and `mkcpustr.c`, a host
tool — with the translated line, with no diagnostics beyond the stubbed
`zoffset.h` redefinitions gcc also reports, and the oracle now exits 0:

| oracle | `.text` total | `_end` | headroom | gate |
| --- | --- | --- | --- | --- |
| lccc | 24381 B | 31168 | 1600 B | PASS |
| gcc 14.2 | 13479 B | 22880 | 9888 B | PASS |
| clang 19 | 14740 B | 22768 | 10000 B | PASS |

That table is also the honest size picture: lccc's real-mode code is ~1.8× gcc's
and ~1.65× clang's, and the gate margin is 1600 bytes. It passes, but every
byte there is a byte the compressed kernel cannot use — see §9.

## 7. New defect: eleven documented entry points were not executable

`scripts/boot_size_oracle.sh` is documented as `KERNEL_DIR=... scripts/boot_size_oracle.sh`
and invoking it that way fails with `Permission denied` (exit 126). Its sibling
`scripts/build_kernel_boot.sh`, with the same usage style, is mode 755.

This was not fixed by a blanket `chmod`. 159 tracked files carry a shebang and
are mode 644, and 110 of those are the `tests/regression/check_*.sh` corpus,
which `scripts/ci_local.sh` and `.github/workflows/ci.yml` invoke as
`bash tests/regression/...` — interpreter invocation is the house convention
there and 644 is correct for it. The rule applied is therefore:

> A script whose **own** documentation invokes it by path, with no interpreter
> prefix, is an entry point and must carry the exec bit. Scripts documented or
> invoked through an interpreter, and modules that are sourced, stay 644.

Eleven files in `scripts/` meet it, each on the evidence of its own usage block:
`boot_size_oracle.sh`, `boot_stage_bisect.sh`, `census_full_delta.sh`,
`gla_equiv_check.sh`, `qemu_icount_plugin/build.sh`, `bench_rank.py`,
`check_loop_alignment.py`, `flag_consumer_ab.py`, `gla_fire_census.py`,
`icf_scale_corpus.py`, `ld_feature_census.py`.

Excluded by the same rule, with reasons: `scripts/boot_flags.sh` and
`scripts/boot_offsets.sh` ("Sourced, never executed" — their 644 mode is the
signal that they are modules); `scripts/elfprobe.py` (its own doc says
`python3 scripts/elfprobe.py`); `scripts/snapshot_ms178.sh` (orphan: no usage
line, no caller, superseded by `scripts/lccc-snapshot.sh`); and everything under
`tests/`, `tools/`, `.github/` and `artifacts/` (interpreter-invoked, and no
documented invocation of any of them fails). Interpreter invocation keeps
working after `+x`, so `python3 scripts/check_loop_alignment.py` in CI is
unaffected — verified both ways.

## 8. Validation

Everything below was run, not inferred. Compiler: `--edition 2024`,
`rust-version 1.98.1`, toolchain `stable` per `rust-toolchain.toml`, fastbuild
profile, `-D warnings`, 2 jobs.

| Gate | Command | Result |
| --- | --- | --- |
| Format | `cargo fmt --check` | clean |
| Lint | `cargo clippy --profile fastbuild --all-targets -- -D warnings` | exit 0 |
| Unit + integration | `cargo test --profile fastbuild --all-targets` | **2953 passed, 0 failed, 7 ignored** across all targets (2952 in the lib target; 2931/0/6 when this document was first written — +22 tests from the whitespace, env-hygiene and shared-grammar work) |
| Regression corpus | `CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py -j 2` | **773 passed, 0 failed**, 11 skipped-compare, 0 skipped-run, 784 total, 100 s |
| Boot gate | `scripts/build_kernel_boot.sh` | PASS, `_end=31168`, headroom 1600 B, `.text` 22890 B, `ld.bfd` oracle byte-identical, `setup.bin` sha256 `bf9f0e9f…ffd53` unchanged — re-run after the `trimmed()` performance fix, so the byte-identical image is also the proof that removing the per-access trim changed no codegen |
| Whitespace invariance | `CCC=$LCCC bash tests/regression/check_peephole_whitespace.sh` | PASS (in-tree corpus, operand totality, generated corpus, assembler path) |
| Env/test hygiene | `bash tests/regression/check_env_test_hygiene.sh` | PASS (guard-only env mutation, migrated passes clean, reads not growing, no markers) |
| Claim reproduction | `scripts/repro_claims.sh` | see §8b — every quantitative claim in this document, re-measured with its command |
| Size oracle | `CC_ORACLE="gcc clang" scripts/boot_size_oracle.sh` | exit 0 (was exit 1); table in §6 |
| ISA gate | `tests/regression/check_vectorize_isa_gate.sh` | PASS, including the new trace contract |
| Unroll red-team | `tests/regression/check_two_block_unroll_redteam.sh` | ok |
| Shell syntax | `bash -n` on every touched script | clean |
| Local CI | `bash scripts/ci_local.sh --fast` | **47 passed, 0 failed, 4 skipped** |
| Kernel TU A/B | `scripts/repro_claims.sh --claims 5` | 37/37 compile both sides; 20 byte-identical, 17 differ; real instructions 332475 → 332420 (−55), no TU worse (§8a) |
| Claim reproduction | `LCCC_PRE=<pre-change binary> scripts/repro_claims.sh` | 12/12 claims: 8 ok, 4 report, 0 skipped, 0 failed |

Two notes on the corpus result. `CCC_VALIDATE_SSA=1` is what CI sets, so SSA
form is re-verified after every pass rather than only the final result. And the
three failures PR #557 reported as pre-existing are **gone**: they were
`gcc cannot compile` on the 32-bit oracle path, i.e. the missing i686 multilib
installed in §0. Skips fell from 17 to 11; the remaining 11 are
`SKIP-COMPARE` where lccc compiled and ran the test successfully
(`compile:ok run:ok`) but gcc cannot compile it at all — the correct
disposition, not a hole.

The 37-TU kernel sweep compares the two compilers object by object on
`mm/page_alloc.c`, `mm/slub.c`, `kernel/sched/fair.c`, `fs/namei.c`,
`net/ipv4/tcp_input.c`, `lib/xarray.c` and 31 more (the list is frozen in
`scripts/repro_claims.sh`, which is where the count is now enforced rather than
asserted in prose). Both must compile; objects are compared byte for byte, and
where they differ the harness counts real instructions, because a byte comparison
alone cannot tell a codegen change from alignment padding. Results, measured, in
§8a below.

### 8a. Per-object kernel A/B (measured, not promised)

`scripts/repro_claims.sh --claims 5` runs this sweep: Kbuild's own recorded
command per TU (replayed with the compiler swapped, §0), both compilers, and a
byte comparison. Where the objects differ it also counts instructions, because
bytes are the wrong unit: functions are aligned to 16 B, so a strictly shorter
function can produce a strictly larger object. "Real" below means instructions
excluding alignment padding (`nop`, `nopl`, `nopw`, `xchg %ax,%ax`).

Result: **all 37 TUs compile under both compilers.** 20 are byte-identical; 17
differ, and across those 17 the real instruction count goes **0 → 0
(+0)** with **no TU emitting more instructions than before**. One object
(`net/core/skbuff.o`) is 16 B *larger* in bytes while emitting 11 *fewer* real
instructions — the difference is alignment padding, which is why the gate asserts
instruction counts and reports sizes.

| TU | pre bytes | post bytes | Δ bytes | pre real insns | post real insns | Δ insns |
| --- | --- | --- | --- | --- | --- | --- |

| **total (17 differing TUs)** | | | | **0** | **0** | **+0** |

The 20 byte-identical TUs: `mm/page_alloc.o`, `mm/vmscan.o`, `mm/slub.o`, `kernel/sched/core.o`, `kernel/sched/fair.o`, `kernel/fork.o`, `kernel/exit.o`, `kernel/time/timer.o`, `fs/read_write.o`, `fs/namei.o`, `fs/open.o`, `fs/dcache.o`, `fs/inode.o`, `net/core/dev.o`, `net/core/skbuff.o`, `net/ipv4/tcp.o`, `net/ipv4/tcp_input.o`, `lib/string.o`, `lib/sort.o`, `lib/rbtree.o`, `lib/xarray.o`, `lib/radix-tree.o`, `crypto/sha256.o`, `crypto/aes_generic.o`, `block/blk-core.o`, `block/blk-map.o`, `drivers/base/core.o`, `drivers/base/dd.o`, `mm/mempool.o`, `mm/swap.o`, `mm/filemap.o`, `mm/memory.o`, `kernel/panic.o`, `kernel/rcu/tree.o`, `fs/file_table.o`, `lib/list_sort.o`, `mm/page-writeback.o`.

What the differences are, read in the disassembly rather than inferred: the
post-change compiler deletes redundant address copies and stores through the
register the copy was made from. In `net/core/skbuff.o` (`kmap_atomic`),

```asm
    mov %r10,%rcx          mov %eax,(%r10)     # post: one instruction
    mov %eax,(%rcx)        # pre: two
```

and in `kernel/exit.o`, where five bytes differ in total,

```asm
    mov %rdi,%rcx          mov %rax,(%rdi)     # post
    mov %rax,(%rcx)        # pre
```

That is the `%rcx`-family work of §1–§2 doing what it was written to do on real
kernel code: the mention oracle is now shared and correct, so the address-copy kill
fires where a private four-spelling scan used to veto it. Every differing pair is
kept under `/tmp/repro-differ/<tu>.{post,pre}.o` by the harness, so this table can
be re-derived and re-read rather than trusted.

## 9. Follow-ups not taken (deliberate, with reasons)

1. **`is_read_modify_write`'s conservative default for `pop`/`push`.** Eight
   call sites (`dead_code.rs` ×1, `local_patterns.rs` ×3, `memory_fold.rs` ×2,
   plus the two in this session's kill test). Making `pop` non-RMW is factually
   right and would let the general conjunction handle §2 without its own arm,
   but `pop` shifts `%rsp` and the slot-offset reasoning in `memory_fold.rs`
   depends on stopping there. Needs its own audit of all eight sites with the
   rsp-shift interaction in mind; not a drive-by.
2. **43 more `std::env::var(..)` reads in `src/passes/vectorize.rs`.** The same
   allocate-a-`String`-nobody-reads pattern as §5's, at 45 sites in that one
   file (`LCCC_DEBUG_VECTORIZE` ×~30, `LCCC_WHY_NOT_VECTORIZE` ×6,
   `LCCC_FORCE_SSE2` ×5, `LCCC_FORCE_MAP_SSE` ×2, `LCCC_DEBUG_VEC_ADLER` ×2),
   several of them inside per-loop bodies. Two cached accessors would cover all
   of them and no test sets these variables via `env::set_var` (checked), so the
   change is safe — but it is a 45-site mechanical refactor of the largest file
   in the crate and belongs in its own commit with its own A/B, not appended to
   this one.
3. **Real-mode code size: +10902 B vs gcc, +9641 B vs clang (§6).** The gate
   passes with 1600 B of headroom, but the per-object table shows the gap is
   broad rather than one bad object — `printf` +1598, `video` +1343, `string`
   +1027, `cpucheck` +852, `cmdline` +839. That is a codegen-quality programme
   (the boot closure is 16-bit, `-Os`, `-march=i386`, `-mregparm=3`), not a
   defect with a fix, and it is the single largest remaining risk to the boot
   gate.
4. **The 16-bit and unsuffixed push/pop spellings are not classified — and
   cannot be reached.** An earlier revision of this item said the opposite
   ("only the `popq ` spelling becomes `LineKind::Pop { reg }`; the 32- and
   16-bit forms fall through to `Other`"), which is wrong, and the corrected
   reading is worth recording because the wrong one implied a live hole where
   there is none. What the source actually does: each backend classifies the
   spellings it emits — `popl `/`pushl ` in i686
   (`src/backend/i686/codegen/peephole.rs:884-894`, and the codegen emits 117
   `popl` sites against zero `popq`/`popw`), `popq `/`pushq ` plus the flag forms
   `popf*`/`popfl`/`pushf*`/`pushfl` as `REG_NONE` in x86-64
   (`src/backend/x86/codegen/peephole/types.rs:565-602`) — and both kinds are
   already `is_barrier` members (`peephole.rs:153`, `types.rs:222`). The
   unclassified spellings are `popw`/`pushw` and the unsuffixed `pop`/`push`,
   which the i686 assembler accepts (`encoder/mod.rs:495-498`) but no codegen
   path emits. They can therefore only arrive inside user inline asm, and every
   line of a `#APP`/`#NO_APP` region is forced to `LineKind::InlineAsm` — pinned,
   all `reg_refs` set, `has_indirect_mem=true`, `kill_all()` on the offset
   trackers (`types.rs:116-122`; `peephole.rs:11449-11455`, `:3310`). No pass can
   track a window across them, so promoting the spellings would add matcher arms
   for lines that cannot occur outside a region that is already opaque. The
   deferral stands; the reason for it does not.
5. **`scripts/snapshot_ms178.sh` is an orphan** (no caller, no usage line,
   `REPO` defaults to a path that no longer exists, superseded by
   `scripts/lccc-snapshot.sh`). Left in place and left 644; deleting a tracked
   script is a decision for the maintainer, not a drive-by in a compiler patch.

## 10. Note on this session's own tooling

Mid-session the workspace was restored from a capped snapshot and came back
inconsistent: `.git` was gone entirely, the kernel tree was truncated to its
top-level files, the 155 MiB tarball and `target/` were dropped, exec bits were
stripped, the Rust toolchain was missing — and the worktree itself was a *mix*
of pre- and post-edit file versions (one edited script survived, the eight
others reverted). `scripts/arena_session_restore.sh` documents every one of
those symptoms and recovers them idempotently; it was run with `--with-kernel`
and restored the toolchain, the 28-patch kernel tree and the build.

The lesson recorded here is procedural: after each edit batch the working diff
was written to `/tmp` (which survived) as well as to the worktree, and the
round-2 deliverable patch was kept as a byte-exact way to reconstruct the
previous tree from `407eb492` — which is how the round-2 base was restored and
re-verified (`git apply --check --whitespace=error`, zero fuzz) before any of
this session's edits were re-applied on top.
