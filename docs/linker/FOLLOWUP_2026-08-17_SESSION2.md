# LCCC Linker — Follow-Up Plan (Sessions 2–6)

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

### 4.1b Borrowed / mmap section data — **next, now the top cost**

Still open, and now the single largest item: `__memcpy_avx_unaligned_erms` is
**8.71% of all instructions**, the #1 self cost in the post-S22 profile.
`parse_elf64_object` copies every section with `to_vec()` (`parse_object.rs`
:127) after `std::fs::read` already copied the whole file. The remaining 20 234
allocations are dominated by these section buffers plus the `Elf64Symbol` vec
growth (3.1 MB in 16 blocks). An arena/lifetime model or `mmap` removes the
copy; policy prefers a small `unsafe` mmap wrapper over adding `memmap2`.
`section_data` is referenced 56× across 18 files, so this needs the same
staged, span-driven approach that worked for `SymStr`.

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
  the duplicate. **Highest-value structural item after the ones below.**

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
