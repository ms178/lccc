# LCCC Linker — Follow-Up Plan (Sessions 2–13)

**Date:** 2026-08-17 (second session of the day)
**Base:** `origin/main` @ `1706984` (post-PR #98)
**Deliverable:** `ms178-1.patch` — cumulative, applies to `origin/main` @ `1706984`
**Session 3 additions:** see §2.7–§2.10 and the revised §4
**Supersedes:** `FOLLOWUP_2026-08-17.md` for the items marked DONE below.

---

## 0. Oracle environment (read this before trusting any number)

The previous session's oracles were **stale release binaries** and produced two
false failures. Both "bugs" were in the oracle, not in lccc:

| Oracle | Old (this repo's prior sessions) | Now | Effect |
|---|---|---|---|
| mold | 2.37 / 2.40.1 release | **2.42.0-git `a718dbb`, built `-march=native`** | — |
| wild | 0.7.0 / 0.10 release | **0.10.0-git `c545fc5`, built `-march=native`** | fixed 2 false FAILs |
| bfd | 2.44 (Debian) | 2.44 | — |

With wild 0.7.0 the suite reported:

```
[FAIL] ctor_priority_order      lccc (0,'123') != wild (0,'213')
[FAIL] relro_write_protection   lccc (0,'PROTECTED') != wild (1,'WRITABLE')
```

**lccc was right in both cases** — constructor priority order and RELRO
enforcement — and wild-git agrees. Anyone re-running this suite must build the
oracles from git; a stale oracle silently inverts the verdict.

> **Action for future sessions:** always `git clone` mold and wild and build
> them before drawing conclusions. Budget ~12 min for wild, ~25 min for mold at
> `-j2`.

---

## 1. Measurement methodology (no PMU available)

The research VM is a 2-core Xeon with **no hardware PMU**
(`/sys/bus/event_source/devices` has only software events, `perf` absent) and
run-to-run wall-clock spread of 20–55%. Wall clock alone cannot resolve the
effects we care about.

**Solution: `valgrind --tool=callgrind` instruction counts as the primary
metric.** Verified deterministic — three consecutive runs of the same binary on
the same input returned *bit-identical* `486,425`. This is the technique
rustc-perf and wild's own benchmark suite use.

Stated limitation, so nobody over-reads it: instruction count is **not** time.
It ignores memory-level parallelism, branch misprediction and cache misses, and
it serialises threads — a parallel linker's wall time can fall while its
instruction count rises. It is therefore used for **single-threaded algorithmic
work** only, and always reported next to best-of-N wall clock.

`tests/linker/bench_linker.py` implements this: best-of-N wall (minimum, not
mean — noise is additive, so the minimum is the best estimator), peak RSS via
`wait4`/`getrusage`, output size, and callgrind instructions. Every
configuration is **validated by executing its output** before it is timed.

---

## 2. DONE this session

### 2.1 ELF parser robustness — two fuzzer-found crashes fixed

Built a structure-aware mutation fuzzer (`/home/user/fuzz/fuzz_ld.py`) whose
seed links to a *complete, runnable* freestanding binary, so mutants exercise
the whole pipeline (resolution → layout → relocation → emit) rather than dying
at symbol resolution. It found two distinct real defects:

**(a) Integer overflow in bounds checks → panic.**
`if off + size <= data.len()` wraps in release builds, so a section header
claiming `sh_offset = 2^64 − 0x2000` passed the guard and panicked in the slice:
`range start index 18446744073692774483 out of range for slice of length 2064`.

This was a **systemic class**, not one site: 4 sites in `parse_object.rs` and
10 in `parse_shared.rs`. Fixed at the root by adding overflow-safe helpers to
`backend/elf/io.rs` (`slice_at`, `range_ok`, `table_entry`, `try_read_u*`) and
converting every call site. `table_entry` also guards the
`base + idx * entsize` multiplication, itself an overflow hazard since
`e_shnum` and `e_shentsize` are attacker-controlled.

**(b) Non-power-of-two `sh_addralign` → 1 TiB allocation → SIGABRT.**
`sh_addralign = 0xffffffffff` violates the gABI (must be 0 or 2^n). The layout
engine rounded the section address up to it and the process died in the
allocator: `memory allocation of 1099511627796 bytes failed`. `catch_unwind`
**cannot** intercept an allocator abort, so this needed a real fix, not a
handler. Rejected at parse time with the offending value in the message, plus a
64 GiB defence-in-depth cap on the output buffer.

**Comparison on the same malformed input:**

| Linker | Behaviour |
|---|---|
| **lccc (after fix)** | `error: section '.text._start' data out of bounds`, **exit 1** |
| bfd | exit 1 |
| mold 2.42-git | **SIGSEGV (exit 139)** |
| wild 0.10-git | exit 255 |

lccc is now strictly more robust than mold on this input class.

**(c) `catch_unwind` at the `lccc-ld` driver** with a quiet panic hook: a
residual panic becomes `lccc-ld: internal error: …` + exit 1 (what build
systems expect) instead of exit 101 + a backtrace note.
`LCCC_LD_PANIC_TRACE=1` restores the full hook for debugging.

**Validation:** 15 new surgical malformed-input tests (each mutates exactly one
field and documents which invariant it probes); 6000-mutant fuzz campaign →
**0 defects**.

### 2.2 `parallel_reloc.rs` — undefined behaviour fixed

The opt-in parallel store path had a **soundness bug**. Its doc comment claimed
"partitions are contiguous segments of a list that has been sorted by offset;
therefore the byte ranges cannot overlap" — but nothing sorted the list or
checked the precondition. With an unsorted list, offsets `[0, 1000, 8, 1008]`
and 2 threads produce spans `[0,1008)` and `[8,1016)`: **two `&mut [u8]`
aliasing the same bytes**, i.e. UB. Additionally the buffer bounds check was a
`debug_assert!`, which compiles out in release — an out-of-range offset became
an out-of-bounds *write* rather than a skip.

Rewritten so disjointness is **established, not assumed**: partition boundaries
are placed only where a running high-water mark has been reached, so mutually
overlapping writes always stay in one partition; every write is bounds-checked
before any pointer is formed.

**A randomised differential test caught a bug in the fix itself** — sorting by
offset reorders writes that overlap at *different* offsets, silently inverting
last-writer-wins (a stable sort only protects equal offsets). Order is now
restored within each partition after the split points are chosen.

11 tests: unsorted-equals-serial, last-writer-wins under 5 thread counts,
out-of-range skipping, partition/no-partition assertions, and a 300-round
randomised differential across densities, widths, overlaps and thread counts.

### 2.3 Congruent segment packing — **59.5% smaller binaries**

Root-caused by comparing program headers against bfd and wild. lccc rounded
each PT_LOAD's **file offset** up to a page, because `addr` was hardwired to
`BASE_ADDR + offset`. The gABI only requires
`p_offset ≡ p_vaddr (mod p_align)` — `mmap` maps `p_offset & ~(pgsz−1)` onto
`p_vaddr & ~(pgsz−1)`. bfd and wild emit congruent, *unaligned* offsets
(`off=0x2d70/vaddr=0x3d70`); lccc was inserting a full page of zeros per
segment.

Fixed with a `vaddr_bias` that is always a multiple of `PAGE_SIZE`: file
offsets stay dense, virtual addresses advance one page per segment.
Congruence then holds by construction.

**Subtle correctness point that this change forced:** RELRO's boundary is an
*address* boundary (ld.so mprotects rounded-out page bounds). Once offsets and
addresses diverge, aligning the file offset would leave RELRO ending mid-page
and write-protect the start of the next section. Now aligned in address space
and verified: a probe that writes to `_DYNAMIC` reports `PROTECTED` for both
lccc and bfd.

**Measured (zlib-ng test binary, same objects, only the linker varies):**

| Linker | Before | After |
|---|---:|---:|
| **lccc** | 20 640 B | **8 352 B (−59.5%)** |
| bfd | 16 400 B | 16 400 B |
| mold | 9 808 B | 9 808 B |
| wild | 6 773 B | 6 773 B |

lccc went from **worst to second-best**, beating bfd by 2.0× and mold by 1.2×.
Largest inter-LOAD file gap fell from ≥4096 B to 384 B.

**`emit_shared.rs` had the identical defect** and was fixed the same way. It is
a *separate* layout implementation, which is itself the finding: the two paths
duplicate layout logic, so a fix to one silently leaves the other broken (see
§4.6). Shared libraries:

| Linker | Before | After |
|---|---:|---:|
| **lccc** | 19 568 B | **7 280 B (−62.8%)** |
| bfd | 15 240 B | 15 240 B |
| mold | 7 792 B | 7 792 B |
| wild | 5 974 B | 5 974 B |

Here lccc beats **both** bfd (2.1×) and mold, and is within 22% of wild.

7 new invariant tests covering both paths: PT_LOAD congruence, no page-sized
padding, RELRO ends on a page boundary, output not larger than bfd, and the
same congruence/padding/loadability checks for a `.so`.

`emit_script.rs` (kernel/linker-script mode) still has the unfixed variant —
see §4.6.

### 2.4 `-Map=` / `-Map FILE` — implemented end-to-end

Previously parsed and **discarded** (`let _ = v;`). Now wired through
`parse_linker_args` → `link_builtin` → `emit_exec`, written after address
assignment from the same layout state the ELF is emitted from.

Output follows GNU ld's column layout so existing scrapers work. Two bugs found
and fixed while wiring: bare `-Map=` was only recognised inside `-Wl,` groups,
and the two-argument `-Map FILE` form was swallowed.

**Verified authoritative, not merely plausible:** every symbol address in the
map is compared against `st_value` read back from the produced ELF —
**20 003 / 20 003 exact matches**. That property is the regression test.

### 2.5 Symbol-table sort — −6.8% instructions, byte-identical output

Callgrind attributed **12.4%** of a 20k-symbol link to the symtab sort and
**7.0%** to `__memcmp_avx2_movbe`. Cause: the cached sort prefix was 8 bytes,
but real symbol names share longer prefixes (`g_sym_12345`, `_ZNSt3__1`,
`__pthread_`), so the `u64` key tied and every comparison fell through to a
byte compare. Widened to 16 bytes (`u128`): memcmp disappeared from the profile
entirely. **57.65M → 53.74M instructions**, output byte-identical.

Also: `read_cstr` now uses a SWAR word-at-a-time NUL scan (Mycroft's test, no
`unsafe`, no new dependency) and an ASCII fast path that avoids the
`Utf8Chunks` machinery. Exhaustively tested against the naive reference across
every alignment × NUL position (640 combinations).

**Negative result, recorded so nobody repeats it:** an *index* sort (sort `u32`
indices into a separate key array; 4-byte swaps instead of 32) was **worse** —
57.8M vs 53.7M — because the double indirection on every comparison costs more
than the wider swap saves. The comment in the code says so.

### 2.6 Test infrastructure

* `tests/linker/bench_linker.py` (new) — 4 profiles, callgrind + best-of-N
  wall + peak RSS + size, correctness-gated.
* `tests/linker/real_workloads.py` (new) — links **real project objects**
  (expat/xmlwf, gzip, zlib-ng) with lccc/bfd/mold/wild and compares *runtime
  behaviour*, not just exit status: gzip round-trips and compares compressed
  size, zlib-ng round-trips 256 KiB and prints a CRC, xmlwf must accept a valid
  document, reject a malformed one with a diagnostic, and produce a matching
  SHA-256 of its echoed output.
  A harness bug was found and fixed here: `xmlwf -d DIR` rewrites its input in
  place, which destroyed the fixture and made every linker look equally broken.
* `run_linker_tests.py`: **89 → 112** tests.

### 2.7 Build-time policy for constrained hardware (session 3)

The research VM has 2 cores and 2 GB RAM, so build time dominates the
edit-measure loop. Three changes, all now encoded in scripts rather than
remembered:

| Change | Before | After |
|---|---:|---:|
| lccc: `--profile fastbuild` (`scripts/build_lccc_fast.sh`) | ~4–5 min | **2m52s cold, ~20 s incremental** |
| mold: `-DMOLD_TARGETS='X86_64;I386'` | ~25 min | a few minutes |
| wild: `-p wild-linker --bin wild` (not the workspace) | ~12 min | ~6 min |

`fastbuild` keeps the project-policy `-O1` optimisation level but turns LTO
off, enables incremental compilation and uses 256 codegen units. The
`build_lccc_o1_j2.sh` release build is still what ship-quality/reproducible
measurements use; `fastbuild` is for the REACT loop.

mold's build cost comes from instantiating the entire linker as a template
over ~18 target architectures, recompiling every `.cc` per target.
`MOLD_TARGETS` is the exact knob (it is documented in mold's own CMakeLists as
"strongly discouraged" for distribution builds — irrelevant here, we only ever
compare x86-64 and i386).

**`tests/linker/setup_oracles.sh`** (new) encodes all of this plus the
git-HEAD-only policy, and is idempotent so it is cheap to run at session
start. **`tests/linker/setup_workloads.sh`** (new) does the same for the
expat/gzip/zlib-ng object trees, with `--clean` as the recovery path — after a
harness wipe the autotools trees have clobbered timestamps and try to re-run
`autoreconf`, which fails on a box without autoconf/m4/perl.

### 2.8 Technical debt: one `SegmentPacker`, not two macro copies

The congruent-packing logic from §2.3 existed as `macro_rules!` in
`emit_exec.rs` *and* an identical copy in `emit_shared.rs`. That duplication is
exactly why the shared-library path stayed broken after the executable path was
fixed — nobody re-measured the third caller.

Extracted into `layout_plan::SegmentPacker`: `vaddr()`, `new_segment()`,
`padding_to_page()`, `is_congruent()`. The invariant (`bias` is always a
multiple of the page size, so congruence holds *by construction* and cannot be
forgotten by a caller) is now stated and tested in one place — 6 property
tests including congruence across all bases/offsets/segment counts, both
4 KiB and 2 MiB pages, and base 0 for ET_DYN.

**Output is byte-identical** after the refactor (exe 8 352 B, `.so` 7 280 B),
which is the point: it is a pure de-duplication.

### 2.9 Driver hygiene: 12 spurious warnings per link, and a silent LTO trap

`gcc -fuse-ld=lccc` printed **twelve** `warning: ignoring unknown option`
lines on every single link, for flags gcc's driver always passes
(`-plugin`, three `-plugin-opt=`, `--push-state`/`--pop-state` pairs). bfd and
mold accept these silently. Noise on that scale trains users to ignore the
linker's output, which defeats the diagnostics that matter.

Fixed by classifying the flags rather than blanket-silencing them:

* **benign driver artefacts** → accepted silently, via an explicit allow-list
  (`is_benign_ignorable`) so a future unknown flag still warns;
* **`-plugin` → handled deliberately.** It is *not* benign: gcc passes it
  unconditionally, but it only matters when an input is LTO bytecode. Ignoring
  it silently is correct for real objects and quietly wrong for `-flto` builds.
  lccc-ld now detects GCC slim-LTO objects (`.gnu.lto_*` sections) and LLVM
  bitcode (`BC\xc0\xde`, plus the wrapper magic) and refuses with:

  ```
  lccc-ld: error: 'x.o' is LTO bytecode, which requires a linker plugin that
  lccc-ld does not implement; rebuild that input without -flto
  ```

  bfd/mold/wild "succeed" here only because they load the plugin. Producing a
  binary with silently-missing code would be the worse failure.

### 2.10 `--exclude-libs=LIST|ALL`

Implemented across `args.rs` (all four spellings), the `lccc-ld` driver and
`emit_shared.rs`. Verified against bfd: helper archive symbols vanish from
`.dynsym`, the real API stays exported.

**A bug in the first implementation is the interesting part.** Filtering the
export list alone passed both obvious checks — symbols hidden, API present —
and still produced an unloadable library:

```
symbol lookup error: ./lib.so: undefined symbol:
```

(note the empty name). The excluded symbol kept its **PLT entry and
`R_X86_64_JUMP_SLOT` relocation**, but the `.dynsym` entry that relocation
referenced was gone, so `r_info`'s symbol index degenerated to 0. An excluded
symbol is by definition not interposable, so the correct fix is to suppress
the PLT as well — the same treatment version-script-locals already get.

The regression test asserts all three properties (hidden / API present /
**loads and returns 43**) plus "no JUMP_SLOT against symbol index 0", and was
**verified to fail on a deliberately reverted build** (2 of 5 checks fail),
so it is not a vacuous test. Writing the naive symbol-table-only assertion
would have shipped the broken library.


---

## 2b. DONE in session 4 (kernel + high-ROI linker items)

### 2b.1 `--emit-relocs` — the KASLR blocker

**Was accepted and ignored.** The link reported success and produced an image
with *zero* `.rela` sections, so a `CONFIG_RELOCATABLE` / `CONFIG_RANDOMIZE_BASE`
kernel would build cleanly and then fail to boot. That is strictly worse than
rejecting the flag, and it is why this ranked above every cosmetic gap.

Implemented in `emit_script.rs` (the kernel's `-T` path):

* one `.rela.<section>` per output section that has relocations;
* `r_offset` rebased from section-relative to the final **virtual address**
  (what bfd emits for a linked image);
* `r_info` symbol indices remapped from each input object's symtab into the
  merged output `.symtab`;
* `STT_SECTION` symbols emitted for every output section when the flag is on,
  so references print as `.text + 0x9` exactly like GNU ld, instead of being
  folded into the addend — `objtool` and the `relocs` pass both expect that
  form;
* `sh_link` → `.symtab`, `sh_info` → target section;
* relocations are still *applied*: the image bytes are unchanged, verified by
  comparing section contents with and without the flag.

**Validation — the strongest available short of booting a kernel.**
`tests/linker/setup_kernel_tools.sh` builds the *real*
`arch/x86/tools/relocs` from kernel.org (v6.12) and the suite runs it over
both images:

| Check | Result |
|---|---|
| relocations vs GNU ld (section-relative, entry for entry) | **9/9 agree** |
| kernel `relocs` tool accepts lccc's image | **yes** |
| relocation set derived by `relocs`, lccc vs bfd | **identical** |
| realistic 5-object kernel-style link | **23 relocs → 12 KASLR entries, identical set** |

Absolute addresses legitimately differ (bfd pads sections differently), so the
comparison is section-relative throughout; comparing raw addresses would report
a false mismatch.

### 2b.2 `gcc -rdynamic` exported nothing

`gcc -rdynamic` produced an executable with **no exported symbols**: dlopen'd
plugins could not call back into the host and `backtrace_symbols` lost every
name.

Cause: gcc's driver spells the flag with a **single dash**
(`gcc -rdynamic` → `collect2 … -export-dynamic`), and `lccc-ld` matched only
`--export-dynamic`. The flag fell through to the unknown-option arm and was
dropped.

It survived this long because `lccc-ld --export-dynamic` invoked *directly*
worked perfectly — so any test driving the linker directly passed. The new
test deliberately goes through `gcc` and finishes with an **end-to-end dlopen
round trip** (plugin calls back into the host, must print 41) rather than a
symbol-table inspection.

### 2b.3 Anonymous version scripts were silently ignored

`VersionScript::parse` required a version name before `{`, so the most common
form —

```
{ global: exported_api; main; local: *; };
```

— returned `None` and the entire script was discarded: `local: *;` exported
everything. This affected **shared objects too**, not just the new executable
path; it had simply never been tested with an unnamed node.

### 2b.4 `--version-script` now applies to executables

Previously honoured only for `.so`. The parser was private to
`emit_shared.rs`, which is *why* the executable path could not use it — the
same "two copies drift apart" failure mode as the segment packer. Extracted to
`linker_common::version_script` and shared, with 6 unit tests (anonymous node,
named node, comment stripping, glob semantics, no-`local:*` case, missing
file).

### 2b.5 Fuzzer moved into the repo and widened

Previously it lived outside the tree and every mutant took the **same** code
path, so `emit_script`, `emit_rel` and `emit_shared` were never fuzzed at all —
a 6000-mutant campaign proved almost nothing about them. Now at
`tests/linker/fuzz_ld.py` with a committed seed corpus + generator, rotating
mutants across four emit paths (userspace, `-r`, `-T --emit-relocs`,
`-shared`). **4000 mutants across all four paths: 0 defects.**

The seed README records *why* the seed is freestanding: an earlier `printf`
seed made every mutant die at symbol resolution, so 250 mutants found nothing;
the freestanding seed found two real crashes within minutes.


---

## 2c. DONE in session 5 (allocation traffic on symbol-heavy links)

Single theme: **the linker allocated roughly two heap blocks per input symbol**,
and that allocator traffic — not algorithmic work — was the top cost on
`many_syms`, the one benchmark where lccc lost to every oracle.

| Snapshot | Change | Ir | allocations |
|---|---|---:|---:|
| S20 | baseline | 66,698,225 | 40,240 |
| **S21** | `SymStr` SSO symbol names + `read_cstr_ref` | 61,213,008 (−8.2%) | 20,234 (−49.7%) |
| **S22** | inherent single-allocation `to_string` | **58,903,810 (−11.7%)** | 20,234 |

Measured with callgrind (Ir) and DHAT (allocations) on the 20 000-symbol bench
under the **fastbuild** profile; wall clock best-of-7 **12.0 → 11.6 ms**.
Verified byte-identical output against a separately-built baseline binary on
both a dynamic libc link and the 20k-symbol link, plus 740 unit and 136
differential tests (bfd + mold + wild) green. Full analysis in §4.1.

New tests: 10 in `linker_common/symstr.rs`, including an exhaustive
length-0..64 round-trip across the inline/heap boundary, a
`size_of::<SymStr>() == size_of::<String>()` guard (which caught a real 32-byte
layout bug), hash-agreement with `str` in both storage classes (unsoundness
here would silently break `&str` map probes), a clone/drop storm over the heap
variant, and the mutation-verified `to_string_allocates_exactly_once`.

New tooling: `scripts/symstr_migrate.py`, a span-driven mass-migration helper
built on rustc's JSON diagnostics — see §4.1 for the two correctness guards it
needs, which apply directly to the interning work.

## 2d. DONE in session 6 (upstream rebase, .eh_frame, C++ EH correctness)

Rebased onto **`04c1432`** (ms178/lccc main). The whole v3 series (S01–S23) is
merged upstream; verified by `git apply --reverse --check`, which succeeds
against `04c1432`, and by `git diff 1706984b..04c1432 --stat` matching the v3
deliverable exactly (55 files, +5813/−450). Old snapshots retired to
`artifacts/merged-2026-08-17.tar.gz`; the v4 series restarts at S01.

### S01 — `.eh_frame_hdr` construction, −10.9% Ir

| Step | Ir | Δ |
|---|---:|---:|
| v4 baseline on `04c1432` | 84,023,409 | — |
| drop the `.eh_frame` `to_vec()` | 83,491,193 | −0.63% |
| memoise CIE encodings | **74,891,960** | **−10.9%** |

1. `parse_eh_frame_fdes` called `parse_cie_fde_encoding` **once per FDE**.
   Real inputs share a handful of CIEs, so a 20 000-function link re-parsed
   the same CIE 20 000 times; `build_eh_frame_hdr` was **5.38%** of the entire
   link. A linear-scan cache keyed on `cie_pos` removes it (a `Vec` beats a
   `HashMap`: the distinct-CIE count is single digits).
2. `emit_exec` copied the whole relocated `.eh_frame` (400 KB here) with
   `to_vec()` only to satisfy the borrow checker before a later `write_bytes`.
   NLL ends the borrow at the call, so the copy was pure waste.

Output byte-identical. `eh_frame.rs` had **no tests at all**; it now has five,
including `distinct_cies_keep_distinct_encodings`, which decodes every FDE
against its own CIE using an independent in-test oracle.

**Rejected experiment (do not retry):** pre-sizing the FDE `Vec` with
`count_eh_frame_fdes` costs a second full scan of the section and made the link
**1.7% slower** (85.5M Ir). The incremental growth allocations are cheaper than
one extra pass. Measured, reverted.

**Testing lesson, learned the hard way here.** The first version of the CIE-cache
test asserted only that the decoded FDE locations were *distinct*, and it
**survived** a mutation that made the cache ignore its key — a wrong encoding
still produces distinct-but-wrong numbers. It only became a real test once it
compared against an independent decode of each FDE via its own CIE. A test that
"passes under the bug it was written for" is worse than no test, because it
buys false confidence. Always mutate a new test before trusting it, and if the
mutation survives, the *assertion* is wrong, not the mutation.

### S02 — C++ exceptions: SIGSEGV throwing any locally-defined class type

**Every C++ program that throws a user-defined exception class crashed**, after
printing the correct output. Reduced to three lines: `struct E{int v;};` plus
`throw E{1}` across one frame.

A user-defined exception type makes the compiler emit a typeinfo object
(`_ZTI1E`) into `.data.rel.ro`. Its first word is an absolute 64-bit reference
to `_ZTVN10__cxxabiv117__class_type_infoE + 0x10` — a vtable **inside
libstdc++**. In an ET_EXEC link that address is unknowable until `ld.so` maps
the library, so GNU ld emits a **dynamic `R_X86_64_64`** and leaves the storage
zero. lccc routed the symbol to a GOT slot — useless, because the storage being
initialised *is* the typeinfo object, not a GOT entry — and statically wrote
only the addend. `__gxx_personality_v0` then dereferenced `0x10` while matching
the LSDA type table.

Fix: `create_plt_got` now returns the absolute-64 sites against dynamic **data**
symbols (`AbsDynReloc`); `emit_exec` adds their targets to `.dynsym`, sizes
`.rela.dyn` for them, emits one `R_X86_64_64` each, and leaves the storage zero.
Copy-relocated symbols are excluded — `R_X86_64_COPY` already fills a local BSS
copy, and a second relocation would double-apply. Byte-compared against
`ld.bfd -no-pie`: identical relocation type, symbol and addend, identical
zeroed slot.

**Why the existing C++ EH test missed it.** `cxx_exceptions_unwind` throws
`std::runtime_error`, whose typeinfo lives in libstdc++ — so no local typeinfo
object is ever emitted and the broken path is never taken. The new
`cxx_exceptions_local_typeinfo` throws two *locally defined* class types, and
checks the **exit code** as well as stdout, because the failure printed the
right answer and *then* died. Mutation-verified: reverting the fix yields exit
`-11`.

Two generalisable lessons: **a differential test that only compares stdout will
miss any crash that happens after the last write** — always compare exit codes;
and **testing with library types instead of user-defined ones exercises a
different linker path entirely**.

### CI coverage gap (workflow files could not be merged)

The v3 patch touched **no** `.github/` files, so nothing was lost in the failed
workflow merge. But it is worth stating what upstream CI does *not* cover, since
none of this session's work would have been caught by it:

`ci.yml` runs `cargo build --release` and **`cargo test --lib`** only. That
means the 745 unit tests run in CI, but the entire linker differential suite
(`tests/linker/run_linker_tests.py`, 137 cases against bfd/mold/wild), the real
workloads (15 checks), and the fuzzer are **never run by CI** — they need
`ld.bfd`/`mold`/`wild` and the workload tarballs, which the workflow does not
install. Both bugs fixed in this session were invisible to `cargo test --lib`.
Adding a CI job that runs `tests/linker/setup_oracles.sh` and then the
differential suite would be high-value; it is deliberately *not* attempted here
because workflow files cannot currently be merged through the integration.

## 2e. DONE in session 7 (byte-level hot paths: −15.0% Ir)

Rebased onto **`3a88d08`**; the v4 series (S01–S03) is merged upstream
(verified by `git apply --reverse --check`). One theme this session: the
linker's innermost byte-handling primitives were written in the obvious way,
and the obvious way costs a call or a bounds check per byte.

| Snapshot | Change | Ir | Δ |
|---|---|---:|---:|
| — | v5 baseline on `3a88d08` | 74,962,782 | — |
| **S01** | single-copy string-table append | 74,624,929 | −0.45% |
| **S02** | single-slice LE reads | 68,579,185 | −8.1% |
| **S03** | delete `eh_frame`'s private LE readers | 65,980,569 | −3.8% |
| **S04** | ASCII fast path in `read_cstr_ref` | 65,046,237 | −1.4% |
| **S05** | `Elf64_Sym` entries without slice copies | **63,757,288** | −2.0% |
| | **cumulative** | | **−15.0%** |

Every step verified **byte-identical** against the pre-change linker on the
20 000-symbol link, with 751 unit + 137 differential (bfd/mold/wild) + 15 real
workloads green, plus fuzzing and Valgrind on the steps that add `unsafe`.

**What the wins actually were.** `read_u64` and friends built their arrays by
indexing each byte — eight bounds checks and eight loads per 64-bit read, 80 150
calls just in `parse_elf64_object` (S02, the single biggest win). `eh_frame.rs`
then carried its *own* private copies of the same helpers, un-`#[inline]`d, so
deleting them in favour of the shared ones was a net **code removal** worth
−3.8% (S03). Symbol entries were assembled with four `copy_from_slice` calls
each — 200 256 calls, 5.94% of the link (S05). `read_cstr_ref` re-walked every
name through the general UTF-8 validator after `memchr0` had already scanned it
(S04).

Wall clock on `many_syms`, best-of-9: lccc **15.2 ms**, mold 12.2, wild 5.5,
bfd 21.8. lccc now clearly beats bfd on this benchmark; mold and wild remain
ahead and the remaining gap is structural (below), not micro-optimisable.

**Rejected experiment (measured, reverted, do not retry):** writing
`push_strtab_name` as the tidy safe `extend(name.iter().copied().chain(once(0)))`
is **79.30 M Ir — 5.8 % *worse* than the baseline it replaced**, because the
byte-at-a-time iterator defeats the vectorised copy. Only the explicit
`copy_nonoverlapping` version is faster, which is the sole reason that function
carries `unsafe`. This is a good illustration of why every one of these
"obviously faster" rewrites was measured rather than assumed.

**Where the remaining time goes** (post-S05 profile): `memcpy` 10.9%, `malloc`
4.4%, `emit_executable` 3.7%, symtab quicksort ~6%. The `memcpy` is no longer
the per-symbol traffic — it is now dominated by 16 large `to_vec()` calls, i.e.
the whole-section copies of §4.1b. That remains the top structural item.

## 2f. DONE in session 8 (structural: one argument parser)

Rebased onto **`82356bf`**; the v5 series (S01–S06) is merged upstream.
This session went after **structural** debt rather than instruction counts.

### S01 — `link_shared`'s duplicate argument parser is gone

This was flagged in §4.6 as *"highest-value structural item"*. It was not a
tidiness complaint: **21 flags that `args.rs` understood were silently dropped
on the shared-library path**, because `link_shared` re-implemented parsing in
~90 private lines. Two were confirmed as real differential failures against
`ld.bfd` before touching anything:

| flag | executables | shared libraries (before) | bfd |
|---|---|---|---|
| `-Map=FILE` | writes a map | **writes nothing** | writes a map |
| `--defsym=A=B` | emits alias | **emits nothing** | emits alias |

`--gc-sections` and `--wrap` were *also* in the dropped set but turned out to
behave identically to bfd on the `.so` path anyway — worth stating, because it
shows the audit was done by measurement, not by listing flag names.

**The documented blocker was real and is now fixed at the root.** `LinkerArgs`
could not express positional inputs, so it could not represent
`--whole-archive`, which applies only to archives *following* it. It now has
`inputs: Vec<InputItem>` — an ordered list of `(name, is_lib, whole_archive)`
recording the flag state in effect at each input's position. The old
order-insensitive views (`extra_object_files`, `libs_to_load`) are retained, so
no existing caller changed. `soname` / `bsymbolic` / `no_undefined` moved into
`LinkerArgs` too; only the private parser had understood them.

`link_shared`: **233 → 141 lines**, one parser. `--defsym` now applies on the
`.so` path; `-Map` is threaded into `emit_shared_library` and written from the
same `output_sections`/`section_map` the ELF is emitted from — the new test
verifies the map's addresses **against the emitted ELF**, section by section,
rather than just asserting the file is non-empty.

**A regression this caused, and why that is the interesting part.** Merging the
parsers immediately broke `-z defs`. The canonical `-z` arm in `args.rs`
matched first and threw the keyword away — its comment claimed `defs` was
*"accepted elsewhere"*, which had been true only because the private parser
handled it. The later `-z defs` branch I added was therefore unreachable dead
code. The existing `so_z_defs_rejects_undefined` differential test caught it
within one run. Two lessons: **a comment asserting that something is handled
elsewhere becomes a lie the moment "elsewhere" is deleted**, and **when
merging two implementations, the first matching arm wins — a duplicate arm
added later is silently dead**.

New tests: 4 unit tests pinning positional semantics (`--whole-archive` in both
bare and `-Wl,` spellings, command-line ordering, legacy views still
populated), plus differential `so_shared_flag_parity`, which guards the whole
flag-parity class against bfd. **Mutation-verified**: it fails on the
pre-refactor linker with *"-Map wrote no map file for a shared library"*.

### S02 — exact symbol-count reservation

DHAT showed the `Elf64Symbol` vector growing one element at a time: 15
reallocations, 3.1 MB of memcpy per 20 000-symbol object. `sym_count` was
already computed exactly two lines earlier.

  allocated **22.03 MB → 19.85 MB (−9.9 %)**, Ir 63,764,826 → 63,661,670.

Byte-identical output. 755 unit + 138 differential + 15 workloads + 800 fuzz
mutants pass throughout.

### Still open (unchanged priority order)

1. **§4.1b borrowed / mmap section data** — `to_vec` is now only **16 calls but
   2.68 %** of the link: the whole-section copies. This is the last large
   structural item and needs the staged, span-driven approach; `section_data`
   is read at 56 sites in 18 files but **mutated at none of them** after
   construction, which is what makes a copy-on-write or borrowed representation
   viable.
2. §4.2 parallel store path — still blocked on hardware (2 vCPU here).
3. §4.3 ICF apply — the main remaining *size* lever.

## 2g. DONE in session 9 (§4.1b closed: zero-copy section data)

Rebased onto **`09b2fba`**. This closes the last large structural item.

| Step | Ir | Δ |
|---|---:|---:|
| v7 baseline on `09b2fba` | 63,661,653 | — |
| **S01** share the input buffer | 62,244,479 | **−2.2%** |
| **S02** share the archive buffer (300-member archive) | 71,972,444 (from 72,189,607) | −0.3% |

`to_vec` in the parse path: **1,709,063 → 34 Ir**. Total `memcpy`: **9.78 % →
7.17 %** of the link.

**`SectionData` is an `Arc<[u8]>` window, not a borrow.** The obvious fix —
`Vec<&'a [u8]>` into the file buffer — cannot work here, recorded so it is not
re-litigated: the buffer is a *local* `Vec` inside `load_file` while the
`Elf64Object` outlives it; archive members are read into temporaries with no
single buffer to borrow; and some sections are *synthesised* (`strmerge` pools,
`linker_entry`), so no lifetime describes every case. `Deref<Target = [u8]>`
made it a genuine drop-in: `section_data` is read at **56 sites in 18 files**
and the type change produced **four** compile errors, all at synthesising sites.

### The detour that matters most

The first working version was **1.4 M Ir *slower* than baseline** even though it
removed the copy. Indexing through `SectionData`'s `Deref` re-derives a
bounds-checked subslice on *every* access, and the symbol loop touches its
section ~6 times per symbol. Binding `as_slice()` once per section recovered all
of it and more. **Removing a copy is not automatically a win if the replacement
adds per-access work** — this would have shipped as a regression had the profile
not been re-read after the change rather than only before it.

### Two negative results (do not retry)

* **`Vec<u8> → Arc<[u8]>` always copies.** An "exact-size read"
  (`File::open` + `metadata` + `read_to_end` + `shrink_to_fit`) to stop
  `fs::read`'s over-allocation forcing a realloc changed *nothing*. A
  standalone experiment showed why: an `Arc` stores refcounts in a header ahead
  of the payload, so it can never adopt a plain `Vec`'s allocation. Removing
  that final copy needs **`mmap`**, not a smarter read.
* `--whole-archive` on a standard link is **rejected with an explicit
  diagnostic** (only supported with `-T`/`-r`), so archive benchmarks must use a
  normal link.

### A flaky test, fixed

The four `ordered_input_tests` from session 8 shared one temp directory keyed
on the PID. `cargo test` runs them concurrently in one binary, so whichever
finished first deleted the others' input files:
`whole_archive_positional_via_wl` passed in isolation and failed in a full run.
Each call now gets a unique directory; verified stable over five consecutive
full runs. **A flaky test is worse than a failing one — it trains you to rerun
instead of investigate.**

### Verification

762 unit · 138 differential (bfd/mold/wild) · 15 real workloads · 800 fuzz
mutants · Valgrind clean. Output byte-identical against a **separately built
baseline binary** on three distinct parse routes: the 20 k-symbol link, a
dynamic libc link (which runs correctly), and an archive link.
`parsed_sections_alias_the_input_buffer` asserts section bytes live *inside* the
input allocation — the structural form of "no copy happened", since Ir counts
would be flaky in a unit test. **Mutation-verified**: restoring `to_vec()` fails
it with *"it was COPIED"*.

### Remaining structural work

1. **`mmap` input files** — now the only way to remove the last whole-file copy.
2. §4.2 parallel store path — still blocked on hardware (2 vCPU here).
3. §4.3 ICF apply — the main remaining *size* lever.

## 3. Current scoreboard

**Correctness (end of session 4):** **730 unit tests, 136 differential tests**
(bfd 2.44 + mold 2.42-git + wild 0.10-git), 15 real-workload checks,
4000 fuzz mutants across four emit paths, and the real Linux
`arch/x86/tools/relocs` as a KASLR oracle — **all green**.

**Link speed** (best-of-5, 2-core VM; spread is high, treat as ordinal):

| Profile | lccc | wild | bfd | mold | Winner |
|---|---:|---:|---:|---:|---|
| many_objects | **2.8** | 4.4 | 6.8 | 7.2 | **lccc** |
| reloc_heavy | **2.5** | 3.0 | 3.7 | 5.2 | **lccc** |
| archive | **2.2** | 3.1 | 4.9 | 5.3 | **lccc** |
| many_syms | 14.4 | **6.1** | 19.3 | 11.4 | wild |

**lccc wins 3 of 4 synthetic profiles and beats bfd everywhere.**

**Output size** (real workloads, lower is better):

| Workload | lccc | bfd | mold | wild |
|---|---:|---:|---:|---:|
| expat/xmlwf | **201.2 K** | 207.5 K | 207.0 K | 194.5 K |
| gzip | **106.7 K** | 114.8 K | 111.8 K | 100.7 K |
| zlib-ng | **128.9 K** | 129.0 K | 132.1 K | 118.1 K |
| zlib-ng minimal exe | **8 352** | 16 400 | 9 808 | 6 773 |
| trivial `.so` | **7 280** | 15 240 | 7 792 | 5 974 |

lccc now beats bfd and mold on size in all of these; wild remains ahead on the
small cases (it packs the ELF header region harder still — worth a look).

**Deliverable verification:** `ms178-1.patch` was applied with `git apply
--check` to a *pristine* `git clone` of `ms178/lccc` at `1706984` — clean.
14 files, +2368 / −150.

---

## 4. Highest-ROI remaining work (ordered by expected-gain × breadth ÷ cost)

### 4.1 `many_syms`: close the gap to wild — **partially done in session 5**

**Status: step 1 of 2 landed (S21+S22). `SymStr` shipped; `SymbolId(u32)`
interning deliberately deferred — rationale below.**

Measured on the session-5 bench (`/tmp/ms`: 20 000 globals + freestanding
`_start`, **fastbuild profile**, callgrind Ir + DHAT allocations). Note the
fastbuild baseline is 66.7M Ir, *not* the 53.7M quoted earlier in this document
— that figure was the release/`-O1` profile. Never compare Ir across profiles.

| Step | Ir | Δ | allocations | Δ |
|---|---:|---:|---:|---:|
| baseline (S20) | 66,698,225 | — | 40,240 | — |
| S21 `SymStr` + `read_cstr_ref` | 61,213,008 | −8.2% | 20,234 | −49.7% |
| S22 inherent `to_string` | **58,903,810** | **−11.7%** | 20,234 | −49.7% |

Wall clock on the same link, best-of-7: lccc **12.0 → 11.6 ms**, vs mold 9.1,
wild 5.7, bfd 17.2. Output **byte-identical** to the pre-change linker on both
a dynamic libc link and the 20k-symbol link; 740 unit + 136 differential green.

**What `SymStr` is.** `Elf64Symbol::name` was a `String`: one allocation per
symbol, plus a second in `parse_object` because `read_cstr` returned an owned
`String` that was immediately copied. But symbol names are *tiny* — measured
mean length 10.4 bytes on the synthetic corpus, 12.9 in `libc.so.6`, 14.7 in
expat's `xmlparse.o`, with **77–100% at or under 23 bytes**. `SymStr` stores
≤23 bytes inline (24 bytes total, exactly `size_of::<String>()`, so the symbol
array does not grow) and heap-allocates only beyond that. `Deref<Target=str>`,
`Borrow<str>` and a `str`-compatible `Hash` make it a drop-in: `FxHashMap`
probes by `&str` still work, so no map type changed. `read_cstr_ref` borrows
straight out of the string table so parsing allocates nothing per symbol.

**Two traps worth remembering.**

1. *The tag byte must live at a fixed offset shared by both union variants.*
   The first attempt put the discriminant first, and the `Box<str>` alignment
   padded the struct to 32 bytes — bigger than `String`, which would have
   traded allocations for cache misses. `is_no_larger_than_string` caught it.
   The fix is the classic SSO layout: payload first, tag at byte 23 in *both*
   `#[repr(C)]` variants, with the heap variant's padding explicitly zeroed so
   reading the tag through either field is defined.
2. *`Display` gives you a `ToString` that reallocates.* After S21, DHAT still
   showed 20 001 allocations, all from `RawVec::grow_amortized` under
   `dynamic.rs:264`'s `sym.name.to_string()`. The blanket `ToString` formats
   through `fmt::Write::write_str`, growing from empty — one realloc per
   symbol. `ToString` cannot be overridden (specialization is unstable), but an
   **inherent** `to_string` shadows it, because method resolution prefers
   inherent methods — so every call site picked it up with zero edits, for
   −3.8% Ir. Pinned by `to_string_allocates_exactly_once` (asserts
   `capacity == len`), mutation-verified.

**Why `SymbolId(u32)` interning was deferred, not abandoned.** Interning is
still the better endpoint: it removes name *hashing and comparison* from the
hot maps, which SSO does not (`FxHasher::hash_one::<&String>` and
`HashMap::insert` were 1.7% + 1.5% self). But it forces
`FxHashMap<String, GlobalSymbol>` to change shape in **10 files across 4
backends** (7 ARM + 3 x86, plus RISC-V/i686 fallout), and every emitter that
formats a name. That is not validatable in one step against a linker whose
correctness rests on differential testing. `SymStr` captures the allocation
half of the win at a fraction of the risk and is a strict prerequisite: once
names are a single owned type behind an interface, swapping the *storage* for
an arena index is a change to `SymStr`'s internals plus the map key type,
rather than a rewrite of every call site. Do interning next, on top of it.

**Tooling note.** `scripts/symstr_migrate.py` drove the 150-error migration
from rustc's JSON diagnostics, rewriting exact byte spans. Two hard-won
guards make it safe and are worth keeping for the interning change:
*one file per round* (rustc's byte offsets are computed against the file as
parsed; after you edit one file, spans reported for it in the same batch are
stale and splicing at them corrupts string literals), and a *stale-span check*
that verifies the bytes at the reported offsets actually appear in the source
line rustc echoed. Spans crossing a newline or a comment are refused outright
and left for hand-fixing — about 10 sites, all in struct literals whose next
field followed a comment.

### 4.1b Borrowed / mmap section data — **DONE in session 9 (§2g)**

Section data now shares the input buffer via `SectionData` (`Arc<[u8]>`
windows). What remains of this item is only `mmap`, to remove the single
remaining whole-file copy; see §2g for why no `Vec`-side trick can.

### 4.2 Wire the parallel store path into `emit_exec`

`parallel_reloc.rs` is now **sound and well-tested but still unused** —
`emit_exec` writes relocations inline. Wiring requires splitting the reloc loop
into (a) resolution, which mutates instruction bytes for TLS/GOTPCRELX
relaxation and must stay serial, and (b) a pure `RelocWrite` list. Expected
1.3–1.8× on reloc-heavy links at ≥4 cores; **no gain on this 2-core VM**, which
is why it was not attempted here — measure on the 14700KF.

### 4.3 ICF apply

`icf.rs` analyses and classifies but never folds. Applying it needs symbol and
relocation redirection plus dropping folded sections. This is the main
remaining *size* lever against wild.

### 4.4 Remaining feature gaps

Remaining: positional `--as-needed` (currently global, not position-sensitive)
· ELF32 script path (`setup.elf`, needed for the kernel's real-mode boot stub)
· `--print-map` as an alias for `-Map=/dev/stdout` · symbol versioning with
multiple version nodes (`@`/`@@`).
Done: ~~`--exclude-libs`~~ (s3 §2.10) · ~~`--emit-relocs`~~ (s4 §2b.1) ·
~~`--version-script` on ET_EXEC~~ (s4 §2b.4).

### 4.5 Fuzzing depth

The current fuzzer is a byte mutator over 4 seeds. Next: a *grammar-aware* ELF
generator (valid-but-hostile structures: overlapping sections, cyclic
`sh_link`, 0-size `SHT_GROUP`, INCLUDE cycles in scripts) and `cargo-fuzz`
targets for `parse_elf64_object`, `parse_archive_members` and
`parse_linker_script`. Also fuzz the **archive** and **linker-script** seeds
harder — this session's campaign was dominated by `.o` mutants.

### 4.6 Structural debt

* `emit_exec.rs` is ~2000 lines with layout, symtab, relocation and header
  emission interleaved. Segment *packing* now lives in
  `layout_plan::SegmentPacker` (§2.8), but `LayoutPlan` itself — the
  section-header index/count computation — is **still not wired**; the
  duplicated `out_sec_to_hdr` / `sh_count` walk remains inline in
  `emit_exec.rs`. That is the next slice of the same debt.
* ~~`emit_script.rs` still has the page-padding defect.~~ **CORRECTION
  (session 2): this claim was wrong.** `emit_script.rs` already packs
  congruently — it aligns each output section's file offset to
  `lma mod 2 MiB` (see the `const PAGE: u64 = 0x200000;` loop). The session-1
  doc asserted the defect without measuring the script path; it was inferred
  from "the fix was applied twice, so the third engine must need it too."
  Recorded here because an unverified claim in a follow-up doc is worse than
  no claim: the next agent would have "fixed" working code.
  The genuine issue was the *duplication*, addressed below.
* **`link_shared`'s hand-written argument parser still duplicates `args.rs`.**
  Session 3 hit this directly: `--exclude-libs` had to be implemented *twice*,
  once in `parse_linker_args` and again in `link.rs`'s private loop, and the
  driver needed a third forwarding arm in `lccc_ld.rs`. Every new flag costs
  three edits and silently misses two of them if you forget. Replacing the
  private parser with `parse_linker_args` is blocked on one thing:
  `link_shared` also drives *positional* archive handling
  (`--whole-archive` regions, `--start-group`), which `LinkerArgs` does not
  model. Fix that first — give `LinkerArgs` an ordered input list — then delete
  the duplicate. **DONE in session 8 (§2f S01):** `LinkerArgs::inputs` models
  the ordered, positional input list and the private parser is deleted; this
  fixed `-Map` and `--defsym` on the `.so` path.

---

## 5. Reproduction

```bash
# oracles (must be git builds)
git clone --depth 1 https://github.com/rui314/mold           && cmake -B build -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_CXX_FLAGS="-O2 -march=native" && cmake --build build -j2
git clone --depth 1 https://github.com/davidlattimore/wild   && RUSTFLAGS="-C target-cpu=native" \
    cargo build --release -p wild-linker --bin wild

# lccc (project policy: opt-level 1, two jobs, swap enabled)
./scripts/build_lccc_o1_j2.sh

# validation
cargo test --release --locked -j2 --lib          # 718 pass
tests/linker/run_linker_tests.py                 # 112 pass
tests/linker/real_workloads.py                   # 15 ok
tests/linker/bench_linker.py                     # perf + size matrix
```

**Environment:** 2-core Intel Xeon @ 2.60 GHz, 2 GB RAM + **4 GiB swapfile**
(required; the build OOMs without it), Debian 13, gcc 14.2.0, rustc 1.97.1,
valgrind 3.24.0, no PMU.

---

## 6. Working rules that paid off this session

1. **Fuzz before claiming robustness.** Both crashes were found in minutes by a
   ~120-line mutator, after the suite was already "green".
2. **Write the randomised differential test before trusting a concurrency
   fix.** It caught a real ordering bug in the very fix that was removing UB.
3. **Verify the oracle.** Two "lccc failures" were wild-0.7 bugs.
4. **Record negative results in the code.** The index-sort experiment is
   documented at the call site so it is not retried.
5. **Snapshot after every validated change.** `lccc-snapshot.sh` writes an
   atomically-renamed patch + tarball + git bundle and *verifies the patch
   applies to a clean worktree* before reporting success.

---

# Session 10 — Agent C's vDSO patch, audited; and ICF apply

**Base:** `ms178/lccc` main @ `b38d86a` (already contains the session-9 work).
**Snapshots:** `S01-vdso-et-dyn-script` (`a48fbb7`), `S02-icf-apply` (`cf2c637`).
**Totals at end of session:** 771 unit pass / 0 fail / 6 ignored, 145
differential pass / 0 fail (bfd, mold, wild), 400 fuzz mutants clean.

## 10.1 Audit of Agent C's `S05-vdso-elf64-dynamic`

**Verdict: adopt the design, fix two defects.**

### The design is right

C synthesises a `<script-dynamic>` input object carrying `.hash`, `.gnu.hash`,
`.dynsym`, `.dynstr`, `.gnu.version`, `.gnu.version_d` and `.dynamic`, lets the
*script's own* `*(...)` wildcards place those sections, and patches the values
once addresses have settled.

This is the correct choice and I am keeping it. The obvious alternative — a
second, script-specific layout path that emits dynamic metadata directly — is
what GNU ld deliberately does *not* do, and it would have meant two independent
implementations of segment assignment drifting apart. Feeding synthetic
sections through the ordinary placement machinery means the script author's
`SECTIONS` block governs the result, exactly as with `ld.bfd`.

**Verified necessary.** On pristine `b38d86a` the reduced Linux v6.12 vDSO
script fails outright:

```
lccc-ld: error: linker script: cannot resolve assignment . = ... (__SIZEOF_HEADERS)
```

With the patch the vDSO links to a 1952-byte image whose exported dynamic
symbol set matches `ld.bfd` exactly: `clock_gettime@@LINUX_2.6`,
`getcpu@@LINUX_2.6`, `time@@LINUX_2.6`, plus the `LINUX_2.6` version node.

**Verified correct, not merely plausible.** `readelf -d` prints a broken hash
table just as happily as a working one, so structural inspection proves
nothing. I reimplemented the *consumers* — the gABI `.hash` walk and glibc
`dl-lookup.c`'s `.gnu.hash` bloom-filter/bucket/chain walk — over the raw file
bytes, and confirmed every exported symbol resolves through **both** tables,
and that an absent symbol resolves through neither. That reimplementation is
now `tests/linker/check_dyn_hash.py`. C's SysV hash, GNU hash, bucket, chain
and bloom constructions are all provably correct. Two bugs I initially
suspected were falsified by Python reproduction before I touched the code.

### Why I had not proposed this myself

My sessions optimised the **executable** path for instruction count and
allocation traffic. Nothing in the benchmark corpus (zlib-ng, gzip, expat) or
in the differential suite links an `ET_DYN` image *through a linker script*, so
this entire code path had zero coverage and never appeared in a profile. It is
a genuine blind spot in my own prioritisation, not merely a lower-priority
item: the kernel vDSO is exactly the case the corpus cannot reach.

### Defect 1 — the bucket count was clamped to 64

```rust
let nbuckets = (...).next_power_of_two().max(1).min(64);   // before
```

`.gnu.hash` lookup cost is the chain length in the selected bucket, walked on
every dynamic symbol resolution for the life of the process. Clamping buckets
caps the table at 256 bytes and pays for it forever on the consumer side.
Measured chain lengths:

| exports | capped avg / max | uncapped avg / max |
|---------|------------------|--------------------|
| 500     | 7.8 / 18         | 1.9 / 4            |
| 5000    | 78.1 / 151       | 1.5 / 4            |

At 5000 exports the cap makes the average lookup **52x** longer to save 19 KB
of buckets. Removed, with the measurement recorded at the site. Cost: +1.8 KB
on the 501-symbol case (53,688 -> 55,480 bytes). Correct trade.

### Defect 2 — `VersionScript::parse_text` mis-parsed full linker scripts

`parse_text` scanned for the first `{`. Given a complete linker script with
**no** `VERSION` node, it found the brace belonging to `SECTIONS`, parsed that
block as a version node named `SECTIONS`, and exported a bogus dynamic symbol
of that name. Reproduced with `nover.lds`, fixed with a word-boundary
`contains_keyword()` helper that returns `None` when the text looks like a full
script (`SECTIONS`/`PHDRS`/`MEMORY`/`ENTRY`/`OUTPUT_FORMAT`) but contains no
`VERSION`. Verified gone; the vDSO still exports precisely its four symbols.

### Integration defect

The patch `git apply --check`ed clean and then failed to **build**: E0308 at
`emit_script.rs:214`, because it predates the session-9 `SectionData`
migration and pushed a `Vec<u8>` where a `SectionData` is now required. Fixed
with `SectionData::owned(data)`. *A clean apply is not integration proof.*

## 10.2 §4.3 — ICF apply

`icf.rs` analysed and classified but **never folded**; nothing called it. The
lever was inert. Wiring it up first required fixing the analysis, which was
unsound.

### The identity test was unsound

`collect_candidates` hashed section **bytes only**. On x86-64 a call is
`e8 <rel32>` with the displacement supplied by a relocation, so

```c
int wrap_a(void) { return alpha(); }   /* e8 00000000 c3 */
int wrap_b(void) { return beta();  }   /* e8 00000000 c3 */
```

emit byte-identical sections differing only in relocation target. The old
`group_is_safe` accepted the pair — it rejected only *absolute* relocations,
and `R_X86_64_PLT32` is relative — so folding them would make `wrap_b()` return
`alpha()`'s value, silently, with no diagnostic anywhere. Confirmed on
gcc 14.2 `-O1 -ffunction-sections`: both sections are literally `e800000000c3`.

Two sections are now identical only when bytes, length, flags, entsize,
**alignment** and the entire relocation list agree — offset, type, addend, and
resolved target identity (globals by name, locals by target section + value).
Hash buckets are split into exact-equality classes before folding, so a
collision can never cause a fold.

### Safety classes

`--icf=safe` additionally refuses to fold any section whose address is
observable. The scan walks absolute relocations across the **whole link**, not
just those inside candidate sections — the address of a function is virtually
always taken from somewhere else, so a local scan would miss it — and maps each
back to the defining section. This preserves C's guarantee that distinct
functions compare unequal. `--icf=all` folds regardless, matching gold/lld.

### Application

`plan()` returns a redirect map. Folded sections are added to `dead_sections`
so the existing `--gc-sections` machinery keeps them out of the layout;
afterwards each folded section's `section_map` entry is aliased to the
representative's placement, so every symbol address and relocation resolves to
the survivor and nothing dangles. Representatives are the lexicographically
smallest `(object, section)` and buckets are sorted before iteration, so the
plan does not depend on hash order — **a linker that folds differently between
runs is not reproducible**, and that would be a defect in its own right.

### Results

C++ template corpus, 8 same-layout instantiations, `g++ -O2
-ffunction-sections`:

| linker | plain | with ICF |
|--------|-------|----------|
| bfd    | 21,800 | (no ICF support) |
| mold   | 17,320 | 14,824 |
| **lccc** | 17,576 | **13,480** |

**-23.3%** versus our own no-ICF baseline, and **9.1% smaller than mold's**
`--icf=all` on identical input, with identical program output.

Honest negative result: on the C corpus the win is small — expat's `xmlwf`
folds 11 sections for 49 bytes, and gzip/zlib-ng/expat binary sizes are
unchanged to the byte. Idiomatic C simply has few exactly-identical functions.
ICF is a C++ lever, and it should be presented as one; the corpus does not
show it because the corpus is C.

### Tests

9 unit tests plus 6 differential cases. Mutation-verified in both directions:
restoring byte-only identity fails
`identical_bytes_different_call_targets_do_not_fold` with an explicit
"this is a miscompilation" message, and makes the differential case print
`22 22` instead of `11 22`.

Also fixed in the harness: `run_linker_tests.py` compiled *every* fixture with
`gcc`, so any `.cpp` fixture could only ever `SKIP`. It now selects the driver
by file extension, which is what let the C++ exception-unwinding case
(`icf_folded_code_still_unwinds`) actually run — folding must not disturb
`.eh_frame`, since the unwinder looks up landing pads by return address.

## 10.3 Lessons

1. **A clean `git apply --check` is not integration proof.** Build it.
2. **Correct algorithms can still be wrong at the seams.** C's hash maths was
   flawless; the defects were a hardcoded clamp and an unrelated parser
   fallthrough. Test the *edges* — no VERSION node, zero exports, 500 exports —
   not just the motivating case.
3. **Structural inspection of a hash table proves nothing.** Reimplement the
   consumer's walk over raw bytes.
4. **An unapplied optimisation hides its own bugs.** ICF's byte-only comparison
   sat in the tree looking reasonable precisely because nothing ever ran it.
5. **Report the negative result.** ICF does almost nothing for C. Saying so is
   more useful than quoting the C++ number alone.

## 10.4 Remaining backlog (unchanged priority)

1. **§4.2** wire `parallel_reloc.rs` into `emit_exec` — still blocked on
   hardware (2 vCPU); measure on the 14700KF.
2. **§4.4** positional `--as-needed`, ELF32 script path (`setup.elf`),
   `--print-map` alias, multi-node `@`/`@@` versioning.
3. **§4.5** grammar-aware ELF fuzzing (current fuzzer is a byte mutator).
4. `mmap` input I/O — the last whole-file copy; no `Vec`-side trick can remove
   it (proven in session 9).
5. COMDAT/`SHT_GROUP` dedup in the exe path; `SymbolId(u32)` interning;
   `LayoutPlan::compute` unwired.

---

# Session 11 — Blindspot sweep of the linker-script path

**Base:** `ms178/lccc` main @ `16410927` (PR #105 merged the session-10 vDSO +
ICF work).
**Snapshots:** `S01-script-blindspots` (`b5f7044b`), `S02-script-tls-pt-tls`
(`db041625`), `S03-script-overlay` (`46f0f905`).
**Totals:** 780 unit pass / 0 fail / 6 ignored, 152 differential pass / 0 fail
(bfd, mold, wild), 400 fuzz mutants clean, expat/gzip/zlib-ng differential PASS.

## 11.1 Agent C's patch: already merged, corrections intact

C's `S05-vdso-elf64-dynamic` is the same patch audited in session 10 and is now
upstream. Re-verified on `16410927` that all three of my corrections survived
the merge: the `.gnu.hash` bucket clamp is gone
(`names.len().next_power_of_two().max(1)`, no `.min(64)`), the
`contains_keyword()` full-script guard is in `version_script.rs`, and the
synthesised sections use `SectionData::owned`. The verdict stands: **the design
is right, and it is kept.** Nothing further to fold in.

Since re-auditing a merged patch produces no new value, the session went after
the more useful question: **what else is missing that no test would ever tell
us about?**

## 11.2 Method: probe for rejection, not for wrong answers

Unit tests only cover features you know you have. A linker's real failure mode
is a construct it rejects outright, and nobody notices until a real script hits
it. So I wrote a differential *feature probe*: 18 minimal scripts, each
exercising one construct that kernel, firmware or `ld --verbose` scripts depend
on, each linked with both `lccc-ld` and `ld.bfd`, classified as
`[both-ok]` / `[GAP]` / `[lccc-ok]` / `[both-no]`.

The first run found **four** rejections; following them up found **two** more
bugs behind them. The probe is now `tests/linker/probe_script_features.sh`,
exits non-zero on any gap, and currently reports **18 ok, 0 gaps**.

## 11.3 What was broken

### (a) The script path handled 7 relocation types; the exe path handles 24

Any `-fPIC` object linked with `-T` failed:

```
lccc-ld: error: script link: unsupported relocation type 41
```

Type 41 is `R_X86_64_REX_GOTPCRELX` — what GCC emits for ordinary external data
access under `-fPIC`. This blocked precisely the use case the vDSO work exists
for: objects compiled `-fPIC` and linked to fixed addresses.

A `-T` link has no GOT, so GOT references must be *relaxed into direct
addressing*, per the x86-64 psABI:

| before | after |
|---|---|
| `mov sym@GOTPCREL(%rip), %reg` `8b` | `lea sym(%rip), %reg` `8d` |
| `cmp %reg, sym@GOTPCREL(%rip)` `3b` | `cmp $sym, %reg` `81 /7 imm32` |
| `call *sym@GOTPCREL(%rip)` `ff /2` | `nop; call sym` `e8` |
| `jmp *sym@GOTPCREL(%rip)` `ff /4` | `nop; jmp sym` `e9` |

The `mov`→`lea` rewrite is the load-bearing one: the original instruction
*dereferences* the slot, so pointing it at the symbol would load the bytes
stored there and treat them as a pointer — silently wrong code with no
diagnostic. Forms that cannot be relaxed are a hard error instead. Also added
16/PC16/8/PC8, SIZE32/64, GOTPC32, GOTOFF64, and range checks on every 32-bit
displacement (a truncated PC32 is how an over-large image acquires a wild jump
that only manifests at runtime).

### (b) STV_HIDDEN symbols were exported from `.dynsym`

The export filter consulted only the version script, so under `global: *` a
hidden symbol was published. Visibility is an ABI property fixed by the
compiler and a glob cannot override it; GNU ld applies visibility *first*.
Verified against bfd on identical input — bfd exports `public_fn` and
`protected_fn` only, and now so do we. `STV_PROTECTED` stays exported, since it
only forbids preemption. This leaked the vDSO's internal helpers into the
dynamic symbol table.

### (c) `PROVIDE_HIDDEN` parsed but threw the hidden bit away

`Assignment` had no visibility field, making `PROVIDE_HIDDEN` a synonym for
`PROVIDE`. Plumbed through to `st_other` (STV_HIDDEN). `HIDDEN(sym = expr)` was
not accepted at all; it is now, and unlike `PROVIDE*` it defines
unconditionally.

### (d) `MEMORY` regions unsupported

Added `MEMORY { name (attrs) : ORIGIN = a, LENGTH = n }` with `> region` and
`AT> region`, including the `org`/`len` abbreviations firmware scripts use.
Regions carry an allocation cursor and — the real payoff — an **overflow
check**. Overrunning a region previously produced silently overlapping
sections; the diagnostic now names the section, the region, the overflow amount
and both ranges, which is strictly more informative than bfd's
``region `tiny' overflowed by 83 bytes``.

### (e) `SEGMENT_START` / `DATA_SEGMENT_ALIGN` / `DATA_SEGMENT_END` rejected

All three appear in **ld.bfd's own default script** (`SEGMENT_START` four
times), so any script derived from `ld --verbose` failed to parse.
`SEGMENT_START` now yields its declared default and is overridable through
`EvalCtx` (wired for `-Ttext`/`-z`). `DATA_SEGMENT_ALIGN` evaluates as
`ALIGN(maxpagesize)`: the segment-overlap trick is an optional few-KiB file-size
win, whereas rejecting the script is fatal. A duplicate `CONSTANT()` handler I
introduced was removed in favour of the pre-existing one, which already had the
correct x86-64 `MAXPAGESIZE` of `0x200000` — a reminder to read the file before
adding to it.

### (f) TLS unsupported in the script path

`__thread` under `-fno-pic` emits `R_X86_64_TPOFF32`, rejected as type 23.
Added TPOFF32/64 and DTPOFF32/64. TPOFF is measured from the **end** of the TLS
block (`%fs:0` points past it on x86-64); DTPOFF from the start. Getting the
base wrong still yields a plausible negative offset, so the test executes the
program. The emitted `mov %fs:-0x4,%eax` is identical to bfd's.

Correct TPOFF values are not enough: without a **PT_TLS** header the loader
never allocates the thread block. One is now synthesised when the script places
`SHF_TLS` sections but declares no PT_TLS, with `p_filesz` covering `.tdata`
only and `p_memsz` covering `.tdata` + `.tbss`.

### (g) `OVERLAY` unsupported

Desugared into ordinary output sections: each member pinned to the overlay's
VMA, each with its own `AT()` slot, plus GNU ld's `__load_start_<n>` /
`__load_stop_<n>` symbols (an overlay without them is unusable — the copy
routine cannot find the bytes). The output-section body grammar is now a shared
`parse_output_section_body()` so overlay members and normal sections cannot
drift apart.

## 11.4 Two bugs found *by* the new tests

These are the interesting ones, because both were introduced or exposed by the
work itself and neither was reachable from any pre-existing test.

**`SIZEOF_HEADERS` undercount → SIGILL.** The first PT_TLS implementation
crashed instantly. `SIZEOF_HEADERS` is computed from `script.phdrs.len()`
*before* layout runs, so the synthesised header went uncounted: the reservation
was 56 bytes short, the first section landed on top of the last program header,
and the entry point pointed into header bytes — `_start` disassembled to
`(bad)`. Fixed with `script_header_size_with(script, extra_phdrs)` backing both
`SIZEOF_HEADERS` and the file-offset cursor, plus a `debug_assert` that the
pre-layout prediction matches the final layout, because a silent disagreement
between those two numbers reproduces exactly this corruption.

**`filesz > memsz` in the single-PT_LOAD fallback.** That path derived `p_memsz`
from the VMA span but `p_filesz` from the file span — fine while LMA == VMA,
wrong the moment an overlay or any `AT()` separates them, since several sections
share a VMA while occupying distinct LMAs. `readelf` reported *"the segment's
file size is larger than its memory size"* and a loader would treat the image as
corrupt. The span is now taken over load addresses with `memsz >= filesz`. Only
reachable from scripts that declare no `PHDRS`, which is why nothing caught it
before.

## 11.5 Tests added

13 unit tests (MEMORY parsing incl. abbreviations/`AT>`/missing-LENGTH
rejection, SEGMENT_START default and override, DATA_SEGMENT_ALIGN,
HIDDEN/PROVIDE_HIDDEN flags, overlay VMA/LMA and load symbols) and 6
differential tests (region placement, SEGMENT_START/DATA_SEGMENT, HIDDEN
definitions, hidden-not-exported, GOTPCREL relaxation, Initial-Exec TLS,
OVERLAY). Every script test **runs the linked program** and cross-checks
against GNU ld; structural inspection would have passed several of the broken
versions.

Mutation-verified: disabling the region overflow check lets the overflow
through; dropping the visibility filter fails `script_hidden_not_exported` with
the leaked symbol's name.

## 11.6 Lessons

1. **Probe for rejection, not just for wrong answers.** A construct the linker
   refuses is invisible to a test suite that was written against the features
   it already has. Eighteen four-line scripts found six gaps in an hour.
2. **`ld --verbose` is a specification.** Two of the missing constructs appear
   in bfd's own default script; anyone customising it hit a parse error.
3. **A correct value is not a working feature.** TPOFF was right while TLS was
   still broken, because PT_TLS was missing. Ship the whole mechanism.
4. **Derived quantities must have one source.** `SIZEOF_HEADERS` and the file
   cursor disagreed by one program header and produced a SIGILL. One helper,
   one assert.
5. **Read the file before adding to it** — the duplicate `CONSTANT()` handler
   would have silently downgraded `MAXPAGESIZE` from 0x200000 to 0x1000.

## 11.7 Remaining backlog

1. **§4.2** wire `parallel_reloc.rs` into `emit_exec` — still blocked on this
   2-vCPU box; measure on the 14700KF.
2. **§4.4** positional `--as-needed`, ELF32 script path (`setup.elf`),
   `--print-map` alias, multi-node `@`/`@@` versioning.
3. **§4.5** grammar-aware ELF fuzzing (the current fuzzer is a byte mutator);
   the feature probe is a step toward this for scripts specifically.
4. `mmap` input I/O — the last whole-file copy (no `Vec`-side trick can remove
   it; proven in session 9).
5. COMDAT/`SHT_GROUP` dedup in the exe path; `SymbolId(u32)` interning;
   `LayoutPlan::compute` unwired.
6. General-Dynamic/Local-Dynamic TLS relaxation in the script path (only
   Initial/Local-Exec is handled; GD/LD would need the `__tls_get_addr`
   sequence rewrites the exe path already has).

---

# Session 12 — §4.4 feature gaps, mmap I/O, COMDAT, GD/LD TLS, grammar fuzzing

**Base:** `ms178/lccc` main @ `1abc5f1a` (PR #107 merged the session-11 sweep).
**Snapshots:** `S01-as-needed-printmap-versions` (`7c5a07e4`),
`S02-mmap-input-io` (`aad5e94c`), `S03-comdat-dedup-exe` (`1b9cbe40`),
`S04-script-tls-gd-ld` (`3488d85f`), `S05-grammar-fuzzer` (`61306c68`),
`S06-header-numbering-single-pass` (`eff7c8a0`).

**Totals:** 797 unit pass / 0 fail / 6 ignored, 158 differential pass / 0 fail
(bfd, mold, wild), 18/18 script probes, 400 byte-mutator + ~4,000
grammar-fuzzer links clean, expat/gzip/zlib-ng differential PASS.

## 12.1 §4.4 — three features that were accepted and ignored

Each of these linked successfully and produced a subtly different binary from
the one the user asked for, which is the worst failure mode a linker has.

**`--as-needed` / `--no-as-needed` were discarded.** Both were in `lccc_ld.rs`'s
ignore list, and the loader hardcoded as-needed semantics. GNU ld defaults to
`--no-as-needed`, so lccc dropped `DT_NEEDED` entries the user had explicitly
requested — and linking a library purely for its ELF constructors is a normal
pattern whose behaviour changes silently when the entry disappears.

Like `--whole-archive`, these are **positional**. `InputItem` gained an
`as_needed` field recorded at parse time and carried to the `DT_NEEDED`
decision. The last mile was subtle: `x86/linker/input.rs` has its *own*
GROUP-script handler, separate from the one in `linker_common/dynamic.rs`, and
its recursion called plain `load_file`, resetting the flag. `libm.so` is
exactly such a script — `GROUP ( libm.so.6 AS_NEEDED ( libmvec.so.1 ) )` — so
the flag was correct at every layer except the one that mattered. DT_NEEDED
sets now match bfd on all three positional cases.

**`--print-map` / `-M` produced nothing.** Also swallowed by the ignore list.
They now set `map_path` to `"-"`, and `write_to_path` treats `-` as stdout, so
one code path serves both spellings and the map can be piped.

**Version scripts stopped after the first node.** Given
`LIBV_2.0 { global: new_fn; } LIBV_1.0;`, `new_fn` was not exported at all and
the inheritance edge was discarded — a library that looks versioned but cannot
satisfy an older dependency. Added `VersionNode` with a `parent`, made
`matches_global` span *all* nodes (export is a property of the script, not of
its first node), and taught the verdef builder to emit the second
`Elf64_Verdaux` with a correct `vd_cnt`. `readelf -V` output now matches bfd
exactly, including `Parent 1: LIBV_1.0`.

## 12.2 mmap input I/O — the last whole-file copy

Session 9 proved no `Vec`-side trick could remove it. mmap can.

`SectionData` already avoided the *second* copy, but `std::fs::read` still
copied every input into the heap. Callgrind put that at **3.46 M Ir, 6.6 % of
the gzip link**, essentially all `__memcpy_avx_unaligned_erms`.

`FileMap::open` maps `PROT_READ/MAP_PRIVATE` and hands out a `FileBacking`
every section clones. No new crate dependency — this repo has none and that is
worth keeping, so `mmap`/`munmap`/`madvise` are three `extern "C"`
declarations. Every failure mode falls back to `fs::read`, and `LCCC_NO_MMAP=1`
forces it for A/B.

| workload | read | mmap | Δ |
|---|---|---|---|
| many_syms | 62,197,711 | 60,272,432 | **−3.1 %** |
| gzip | 52,386,561 | 48,767,354 | **−6.9 %** |
| expat | 53,685,755 | 49,443,714 | **−7.9 %** |

memcpy share on gzip fell 11.23 % → 4.95 %; output byte-identical on all three
(`cmp`). Archives benefit twice: members are windows into one mapping, and
pages belonging to members the linker never pulls in are never faulted in.

Wall clock also moved — lccc now beats bfd on all three workloads
(gzip 16.2 vs 26.7 ms, zlib-ng 15.9 vs 21.6, expat 20.2 vs 26.9) — though that
spread is noisy here and the Ir numbers are the ones to trust.

## 12.3 COMDAT dedup in the executable path

`emit_rel.rs` has done this since it was written; the executable path never
did. **Symbol resolution hid it**: only one definition ever wins, so the
program behaved correctly and `nm` matched GNU ld exactly, while the losing
section *bodies* were still laid out as unreachable dead bytes.

Symbol counts therefore cannot detect this. Searching the linked image for the
literal byte pattern of `Widget<long>::twice` across three TUs:

```
before   3 occurrences, 780 executable bytes
ld.bfd   1 occurrence,  579
after    1 occurrence,  567   (smaller than bfd)
```

Factored into `linker_common::comdat` so the `-r` and executable paths cannot
drift. The winner is the first group with a given signature in link order, so
the result depends only on input order, never on hash iteration. It feeds the
existing `dead_sections` set and runs **before** ICF: the compiler already
declared these copies interchangeable, so no content comparison is needed.
Plain non-`GRP_COMDAT` groups are left alone — discarding one would delete live
`.debug_*` data.

## 12.4 GD/LD TLS relaxation in the script path

Session 11 left this as known-missing. General-Dynamic is the **default** model
for `-fPIC`, so any PIC object with a `__thread` variable still failed with
"unsupported relocation type 19". A `-T` link is static with a fixed TLS block,
so `__tls_get_addr` is unreachable and GNU ld *relaxes* to Local-Exec.
Implemented GD→LE, LD→LE, TLSDESC→LE and GOTTPOFF→LE, recording the consumed
byte ranges so the `__tls_get_addr` call's own PLT32 relocation is not applied
over the rewritten bytes.

**DTPOFF had to be TP-relative** (bug found by the new test). After LD→LE the
sequence leaves the *thread pointer* in `%rax`, not the module base. My first
version used the block-relative offset and produced `cmpl $0xb,0x4(%rax)` where
bfd emits `cmpl $0xb,-0x4(%rax)` — every Local-Dynamic access reading the wrong
side of the thread pointer. The link succeeds and the values are garbage, so
only a runtime check catches it.

**GOTPCREL relaxation generalised.** The TLS test's TCB setup compiles to
`sub sym@GOTPCREL(%rip),%rsi` (opcode `2b`), and the relaxer handled only `cmp`
(`3b`). The psABI relaxes the whole `op r64, m64` group to the group-1
immediate form, so all eight (add/or/adc/sbb/and/sub/xor/cmp) are now handled
via an explicit opcode → `/ext` table.

## 12.5 §4.5 — grammar-aware ELF fuzzing

`fuzz_ld.py` mutates bytes, so almost every mutant dies at the first validity
check and the machinery *behind* the parser is rarely reached.
`fuzz_elf_grammar.py` inverts this: it **builds** a structurally valid ELF64
relocatable (sections, locals-first symtab, `.rela` with patched `sh_link`,
string tables) and then makes one semantically hostile choice — a relocation
past the end of its section, a COMDAT group listing itself, `SHF_STRINGS`
without a terminator, alignment 2^40, a COMMON symbol of size 2^62, and 13
more.

Because the file parses, the linker gets far enough for the interesting code to
run: **every** generated object is accepted by `readelf -h`, and about half the
links reach the emitter. Inputs rotate across seven modes (exec, shared, `-r`,
`-T`, `--gc-sections`, `--icf=all`, `--emit-relocs`).

**The fuzzer was validated against a planted bug** — a `panic!` on an
out-of-range relocation offset — and correctly reported
`[PANIC] mode=exec / mutation: reloc offset past end of section` with a working
reproducer. A fuzzer that finds nothing is indistinguishable from one that
tests nothing; the procedure is documented so it is repeated whenever the
generator changes. ~4,000 links across six seeds: clean.

## 12.6 Structural: one section-header walk instead of two

The header numbering in `emit_exec.rs` was spelled out **twice** — once to
record each section's index, once to recount and derive `symtab_shidx`. Drift
between them does not fail the link; it produces symbols whose `st_shndx` names
the wrong section, silently breaking debuggers and `perf`. Now one walk whose
running count yields both. Byte-identical output on gzip, as a pure
de-duplication should be.

`LayoutPlan::compute` stays unwired and now **says why**: it models a simpler
layout than reality (fifteen linker-created headers interleaved between four
ordered groups of output sections, depending on `is_static` and on which tables
are non-empty), so switching to it would silently renumber every header. A
plausible-looking function that must never be called is a trap; the doc comment
now states the constraint and points at the invariant test.

## 12.7 Negative and deferred results

**`SymbolId(u32)` interning: measured, not attempted.** Callgrind attributes
~7 % of the many_syms link to `String`-keyed hashbrown traffic (insert 1.63 % +
1.27 %, iteration 1.30 % + 1.19 %, `hash_one` 0.96 %), so the lever is real. It
is also 25 map declarations and 149 call sites. A rushed conversion risks
correctness for a single-digit gain, so it is recorded with its measurement
rather than half-done.

**ELF32 `setup.elf`: diagnosed, not implemented.** `lccc-ld` accepted
`-m elf_i386` and silently assumed x86-64, failing later with the opaque
"not 64-bit ELF". A real ELF32 script emitter is a large piece of work; in the
meantime `-m` is validated and unsupported emulations are refused up front with
a message naming the alternative. Silently ignoring an option that changes the
output format is itself a defect, and that part is fixed.

## 12.8 Lessons

1. **A flag can be correct at every layer except the one that matters.**
   `--as-needed` parsed right, threaded right, and was reset by a second
   GROUP-script handler nobody remembered existed. Trace the value to the
   decision, not to the API boundary.
2. **Symbol-level checks cannot see body-level duplication.** COMDAT dedup was
   missing for the whole life of the executable path and every symbol tool
   agreed with GNU ld. The test had to search for raw bytes.
3. **Relaxation changes what a register holds.** DTPOFF after LD→LE is
   TP-relative, not block-relative; the wrong choice links cleanly and computes
   garbage.
4. **Validate the fuzzer.** Plant a bug and confirm it is found, or "no defects"
   means nothing.
5. **Prefer one walk to two copies.** The header numbering had survived
   duplicated because both copies were always edited together — until they
   would not have been.

## 12.9 Remaining backlog

1. **§4.2** wire `parallel_reloc.rs` into `emit_exec` — still blocked on this
   2-vCPU box; measure on the 14700KF.
2. `SymbolId(u32)` interning (~7 % measured; 149 call sites).
3. ELF32 output for the kernel's `setup.elf` (now diagnosed rather than
   silently wrong).
4. Thin-archive shared buffers — the mmap work covers regular archives; thin
   archives still read each member separately.
5. `--icf=safe` currently rejects any address-taken function; mold folds those
   whose address is only compared, not called through.

---

# Session 13 (hotfix) — a shipped compiler warning, and the process hole that let it ship

**Base:** `ms178/lccc` main @ `a48de95b`. **Snapshot:** `S01-fix-private-interfaces-warning` (`b73563ec`).

## 13.1 The defect

The session-12 mmap work shipped a warning:

```
warning: type `Mapping` is more private than the item `FileBacking::Mapped::0`
 --> src/backend/linker_common/filemap.rs:211:12
    field `FileBacking::Mapped::0` is reachable at visibility `pub`
note: but type `Mapping` is only usable at visibility `pub(self)`
```

`FileBacking` was a `pub enum` whose `Mapped` variant carried `Arc<Mapping>`,
and `Mapping` is a private raw-pointer RAII type. A private type was reachable
through the crate's public API.

The fix is the shape the file already used ten lines earlier: `FileBacking` is
now an opaque struct wrapping a private `FileBackingInner`, mirroring
`FileMap`/`FileMapInner`. That is not merely warning-silencing — `Mapping`'s
only invariant is *unmapped exactly once*, so no caller should be able to
construct or destructure it. The public surface is `FileBacking::owned(..)`
(8 call sites converted from the variant constructor) plus `as_slice()`.

## 13.2 Why it was not caught — the part that actually matters

Every build check I ran across sessions 9–12 was some form of

```
cargo build ... 2>&1 | grep -E "^error" | head
```

which is blind to warnings by construction, and a zero exit status was then
read as success. The compiler *did* report the problem on every single build;
the process that was supposed to validate the work threw the message away.

Grepping harder is not a fix, because it depends on remembering. So
`scripts/build_lccc_fast.sh` now exports `RUSTFLAGS=-D warnings`, making a
warning a build failure; `LCCC_ALLOW_WARNINGS=1` opts out for a scratch build
mid-refactor.

**Mutation-verified**, like any other guard: reintroducing the exact defect now
yields `error: type Mapping is more private than the item
FileBackingInner::Mapped::0` and a failed build, instead of a warning nobody
reads. Warnings are also now checked on **all three surfaces** — lib, bins and
tests — where previously only the lib was inspected, and only for errors.

## 13.3 Verification

Zero warnings on lib, bins and tests. 802 unit pass, 158 differential pass
(bfd/mold/wild), 18/18 script probes, 400 byte-mutator + 500 grammar-fuzzer
links clean, expat/gzip/zlib-ng differential PASS.

Because the change touches the zero-copy path, that was measured rather than
assumed: gzip output is byte-identical with and without `LCCC_NO_MMAP`, and the
mmap win is intact at **48,786,539 vs 52,406,946 Ir (−6.9 %)**.

## 13.4 Lesson

**A check that cannot fail is not a check.** Filtering build output for
`^error` silently redefined "clean build" as "compiles at all", and it stayed
wrong for four sessions because nothing ever contradicted it. When a class of
defect escapes, fix the gate, not just the instance — and then prove the gate
fails on the reintroduced defect.

---

# Session 14 — focused Linux i686 setup reproduction and code-size work

**Rebased base:** `ms178/lccc` main `208bbfae` (PR #114).
**Kernel:** Linux 6.18.44 with all 26 selected `linux-cachymod-6.18` package
patches applied. **Scope:** exactly the 24 objects in `arch/x86/boot/setup-y`;
no full kernel build. C objects are compiled by LCCC and the final setup ELF is
linked by `lccc-ld`, with GNU BFD used as a file-level oracle.

## 14.1 Reproducing the wall

A guarded top-level make target (active only at `MAKELEVEL=1`) drives the
subdirectory with the kernel's real setup flags. This avoids two invalid
shortcuts: asking kbuild for individual objects selects generic `-m64`, while
an unguarded injected target is recursively redefined. The focused build now
reliably produces all 24 setup objects without attempting a full kernel.

The current relaxed-assertion link still demonstrates that code generation,
not just the historical assertion, is the blocker. After rebasing S21 onto
main `208bbfae`, `.text` alone is `0x9303` (37,635 bytes), and its end is
`0x97e5`, 6,117 bytes beyond 32 KiB. The improved placement now gives
`setup_size=0xb000`, `setup_sects=0x58`, and `_end=0xba00`. Relaxing the
assertion therefore diagnoses the overrun; it does not solve it.

## 14.2 Correctness fixes required before optimization

The i686 path first needed ABI and optimizer repairs: regparm callees capture
incoming `%eax/%edx/%ecx` correctly; dead epilogue writes and genuinely unused
callee-save pairs can be removed without changing body `%esp`; wide integer
returns keep `%edx` live only in marked functions; and whole-function stack-slot
DSE no longer compares unstable raw `N(%esp)` offsets across pushes or stack
adjustments.

The final dead-move pass now solves backward liveness over the function CFG,
including branches and loop backedges. Unknown jump targets remain
conservative. Architectural operands omitted from AT&T syntax are explicit in
the liveness model: one-operand multiply/divide, `cltd`, `xchg`, and string
operations cannot lose live inputs. A red-team test uses the high half of
`mull` specifically because the old explicit-text-only model could otherwise
consider the `%eax` dividend dead.

`movsbl`/`movswl` also must not be identified as implicit `movs` string
instructions merely because their mnemonic starts with `movs`; string opcodes
are now matched exactly, including optional `rep` prefixes.

## 14.3 S21: source-clobber propagation and accumulator updates

Two frequent accumulator-backend shapes survived S20:

```asm
movl %eax,%ecx
movsbl (%ecx),%eax
```

and

```asm
movl %ebx,%eax
incl %eax
movl %eax,%ebx
```

The first copy is valid for the source read of the clobbering load itself: x86
reads the address before writing `%eax`. Copy propagation now rewrites that
last reader and then stops, retaining conservative rejection of partial or
unencodable substitutions.

The second sequence is retargeted to `incl %ebx` only when CFG liveness proves
`%eax` dead after the copy-back. All whole `%eax` source occurrences are
retargeted too (`addl %eax,%eax` becomes `addl %ebx,%ebx`); implicit one-operand
`imull` and partial-register operations are explicitly rejected. Zero idioms
such as `xorl %eax,%eax` are modeled as writes with no dependency on old
`%eax`, which exposes additional safe cases.

Concrete kernel changes include `skip_atoi` folding
`movl (%ebx),%eax; movl %eax,%ecx; movsbl (%ecx),%eax` to the two-instruction
form, and `strlen` replacing its three-instruction cursor increment with
`incl %ebx`.

## 14.4 Measured result and validation

Compared with S20, aggregate text across the same 24 setup objects fell from
45,106 to **40,258 bytes**, a further **4,848-byte reduction** after combining
the rebased main work with this series. Twenty objects shrank, three assembler
objects were unchanged, and `cpu.o` grew by 97 bytes; the latter remains a
specific follow-up rather than being hidden by the much larger aggregate win.
The largest reductions were:

| object | S20 | rebased S21 | delta |
|---|---:|---:|---:|
| `video.o` | 6,401 | 5,499 | -902 |
| `video-vesa.o` | 2,822 | 2,104 | -718 |
| `video-mode.o` | 2,671 | 2,124 | -547 |
| `video-bios.o` | 2,246 | 1,801 | -445 |
| `video-vga.o` | 2,091 | 1,744 | -347 |
| `main.o` | 1,537 | 1,226 | -311 |

Validation is intentionally layered:

- 38 focused i686 peephole tests pass, including alias-clobber, loop-backedge,
  partial/implicit-op rejection, wide return, `%esp` store, stack-top/`ret`,
  and multiply-input soundness cases;
- 118 focused x86-64 peephole tests pass (two pre-existing tests remain
  intentionally ignored);
- the official executable comparison is 325 passes and the same six known
  oracle mismatches; `post_inc_ssa_i686`, `i686_return64_peephole`, and the
  repaired `retpoline_thunk_inline` runtime test pass;
- all 24 setup objects rebuild with clean-source fastbuild LCCC;
- `lccc-ld` and GNU BFD produce byte-identical 42,656-byte setup binaries,
  SHA-256 `1281fe86c7452d5281f52521829046fc7f5651ac6c0199813aabea7a60e2e5fa`.

A broad caller-saved allocation experiment for i686 `ParamRef` values was
rejected: it enlarged `string.o` by 75 bytes and did not improve `strlen`. It
would also require real parallel-copy handling for incoming
`%eax/%edx/%ecx`, including `%edx`↔`%ecx` cycles; the naive change is not kept.

## 14.5 Latest-main red-team findings and cross-backend parity

The latest main changes were complementary and account for most of the rebased
size win: escape-aware DSE, i686 register-base GEP folding, tail calls, and the
new x86-64 CFG liveness machinery all survived the rebase. The older blanket
ESP-store exclusion was deliberately not restored; main's normalized ESP walk
is more precise and passes the cross-push regression.

The audit did find one release-blocking DSE bug. Inline retpolines dispatch with
`movq %target,(%rsp); ret`; the escape-aware x86-64 never-read-store pass
counted only textual loads, deleted the stack-top write, and left execution
spinning forever in `pause; lfence; jmp`. DSE now recognizes a stack-top store
immediately consumed by `ret` or `pop`. The same architectural rule and test
were added to i686 rather than fixing only the observed x86-64 failure. The
full retpoline runtime test now terminates and its disassembly retains the
required store.

The two S21 code-size folds are also wired in both backends where applicable.
x86-64 copy propagation now permits only the narrow final address-reader case
that overwrites the alias source; a first broad implementation was rejected
because it interfered with a stronger SIB/LEA fold. x86-64's existing
CFG-liveness copy/shift/copy-back fold now accepts the same safe explicit
32-bit ALU update family as i686, while rejecting partial-width and implicit
one-operand multiply forms. A non-result caller-saved move with no read before
`ret` can be deleted on x86-64, allowing the propagated relay copy to disappear
without treating arbitrary branch boundaries as dead.

## 14.6 Follow-up

The setup image is materially smaller but is not yet below the 32 KiB wall.
Next work should profile the remaining large bodies (`video.o`, `printf.o`,
`string.o`) for repeatable accumulator/spill shapes, add semantic tests before
each rewrite, and continue file-level BFD comparison after every linked-layout
change. lld, mold, and wild were lost with the interrupted workspace and must
be restored before claiming renewed four-linker coverage; GNU BFD is the
current ELF32 setup oracle.

---

## Session-20 addendum (2026-08-19) — oracle preference audit + setup.ld fidelity

Build/version preferences re-verified in `tests/linker/setup_oracles.sh`:

| preference | status |
|---|---|
| mold from git HEAD, never release tarballs | ✅ encoded |
| mold targets restricted to `-DMOLD_TARGETS='X86_64;I386'` (the i686 preset the user requires to keep build time viable on the 2-core box) | ✅ encoded |
| wild from git HEAD, `-march=native`, binary-only | ✅ encoded |
| GAS/bfd pinned to **2.47** (built locally when system bfd ≠ 2.47) | ✅ encoded (`BINUTILS_VERSION=2.47`) |

`lccc-ld` boot-gate behaviour (kernel setup.ld) re-confirmed faithful this
session: all five ASSERTs evaluate, the live failure is "The setup must be
at most 64 sectors in size" (setup_size = ALIGN(.,4096) at .signature), and
`_end <= 0x8000` is the binding gate once setup_size passes (bss counts;
see session-20 history doc for the exact arithmetic: text budget ≤ 23330).

One assembler/driver correctness fix landed this session that matters for
any linker-driven kernel flow: a missing `#include` in `.S` input is now
fatal (GCC parity). Previously `header.S` preprocessed against an EMPTY
`voffset.h` and the `VO_*` `#if` guards silently went false — a wrong
setup header would have reached lccc-ld without any diagnostic.

## Session-21 addendum (2026-08-19) — lever push
Session 21 implemented the session-20 lever roadmap (see
`docs/history/2026-08-19-session21-lever-implementation.md`). No
linker-side changes were needed: lccc-ld evaluates setup.ld's five ASSERTs
faithfully and the gate remains codegen-bound (boot object text 32 858 vs
the `_end ≤ 0x8000`-derived budget of 23 330). Oracle preferences
(mold `-DMOLD_TARGETS='X86_64;I386'`, binutils 2.47 pin, git-HEAD builds)
re-verified unchanged.
