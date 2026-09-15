# FOLLOWUP 2026-09-15 — .gnu.hash oracle parity (all 5 backends), RELRO page-rightsizing (riscv + new on aarch64), riscv `-l` DT_NEEDED

Scope of this session: finish the `.gnu.hash` rewriting started S03, port it to every supported
architecture, close the riscv RELRO defect that had been parked unverified, and fix every defect
found while doing so. All claims below were measured, not guessed; fixture dir
`/home/user/ldwork/{gnuhash,cross,sotest}`, qemu-user + riscv64/aarch64 cross toolchains installed.

## 1. `.gnu.hash` sizing: measured oracle ground truth and the ported rule

`ldwork/gnuhash/measure.sh` dumps readelf-.gnu.hash geometry for bfd/lld/mold at
n ∈ {8, 64, 512, 1024, 4192 … 20000} exports:

| linker | nbuckets | bloom words | shift | FPR (measured, 20k absent probes) |
|--------|----------|-------------|-------|-----------------------------------|
| lld    | n/4      | next_pow2(n/4) | 26   | ~0.4–0.6 %                        |
| mold   | ≈ n/8    | next_pow2(n/4) | 26   | ~0.4 %                            |
| bfd    | ≈ n/2    | ≈ n/8          | lg2(bloom_bits) | 1–2 %                 |
| old lccc | next_pow2(n) (cap’d) | **1** | 6 | **≈ 1.0 at 20k (filter dead)** |

Adopted lld-parity rule (shared helper `linker_common::gnu_hash_params/build_gnu_bloom` in
`src/backend/linker_common/hash.rs`):

```
bloom_words = max(1, next_pow2(ceil(n/4)))   # power-of-two REQUIRED: glibc masks word idx with size-1
shift       = lg2(bloom_words * class_bits)  # bfd's rule too; probes a high, entropy-rich window
nbuckets    = max(1, n / 4)
```

At n=3000 a lccc-built .so has `nbuckets=750 bloom=1024 shift=16` — byte-for-byte lld-identical
geometry and smaller total table bytes than bfd/lld/mold. Bloom carries the false-positive rate
(the 8-bits-per-symbol lld budget); buckets only bound chain length — that is why bfd at 2× the
bucket table is strictly worse. The old single-word lccc bloom saturated above ~30 exports.

**Hidden defects found & fixed in the same pass**

- x86 `emit_exec.rs`: hashed names *pre-sort* with the version-stripped (`dynsym_emit_name`) form,
  then *re-hashed* raw names post-sort — any `@`-versioned export landed in buckets/chains its
  bloom bits disagreed with (runtime resolution failure). All emitters now hash ONCE and reorder
  via an index permutation; bloom, sort and chain consume the same hash vec.
- `emit_shared.rs` (riscv, arm), `emit_dynamic.rs` (arm), i686 `gnu_hash.rs`: chain-end marking was
  an O(buckets × symbols) rescan per bucket (quadratic at glibc scale) — replaced by one linear
  pass; symbols are bucket-sorted so entry i terminates its chain iff the next entry hashes to a
  different bucket.
- i686 `gnu_hash.rs` (single 32-bit word, shift 5) → helper with `class_bits = 32`; the odd
  per-bucket O(n²) rescan removed. Same API (`(Vec<u8>, Vec<usize>)`), both callers transparent.
- riscv `relocations.rs::build_gnu_hash`: n/2-word bloom at shift 6 (5× lld's table bytes for a
  worse FPR) → helper. Empty-symbol early return stays (identical layout to helper at n=0).
- x86 `emit_script.rs::build_gnu_hash` (script-mode links, incl. kernel-style): pow2(n) buckets
  (4× lld), bloom `pow2(n/class_bits)` (= n bits for 2n insertions, ~86 % full → FPR ≈ 0.74),
  fixed shift 6 → helper at the correct class width.

**Proof chain** (all re-runnable):

- Unit: 4 tests in hash.rs (lld ground-truth table {64→16/16, 512→128/128, 1024→256/256,
  4096→1024/1024}, pow2 invariance n<600 both class widths, glibc two-bit roundtrip, FPR < 5 % at
  5 000 sym inserts). `cargo test --profile fastbuild --lib` = 2826/0.
- Suite: `gnu_hash_sizing` (n=600: asserts 150/256/14, independent glibc-walk resolves all 600
  exports, raw-bloom absent-probe FPR bound, consumer exe resolving through the real ld.so) and the
  new `crossarch_gnu_hash_relro` (below). 230 pass / 0 fail / 0 warn / 1 skip.
- Manual runtime: 3000-export .so consumed by bfd-, lld-, mold- and lccc-linked exes through the
  real ld.so → identical results (8998). n=600 32-bit i686 table: geometry (150/256/13) +
  probe-false-positive bound + ALL 600 resolvable through an independent chain walk.
- riscv: **bfd-built consumer exe resolving f1/f2 from lccc's shared lib under qemu-riscv64 +
  real glibc** → rc=0. This isolates the table from any lccc consumer-side bug.

## 2. RELRO page-rightsizing: riscv fixed, aarch64 added

Bug class (pre-existing): `ld.so` computes `mprotect(PROT_READ)` over
`[round_down(vaddr), round_down(vaddr + memsz))`. A `PT_GNU_RELRO` whose memsz does not reach the
next page boundary protects **nothing**.

- riscv `emit_exec.rs`: relro was `rw_page_start .. raw cursor` (0x218 B in the glibc exe repro) →
  zero protection. Now closes at the page edge; the writable tail (`.data`, TLS, `.bss`, IFUNC
  slots) starts on a fresh page. `p_align = 1` like bfd (glibc consumes only vaddr/memsz — the old
  `p_align = PF_R = 4` was a category error AND would have failed glibc's alignment sanity had it
  been page-sized).
- riscv `emit_shared.rs`: inverted half of the same bug — when `.got.plt` shared the page the code
  *removed* the padding and emitted a sub-page memsz (nothing protected); when it didn't, `.data`
  sharing the page would have been mprotected read-only (write faults at runtime). Now: the RELRO
  group `[arrays, .dynamic, .got]` closes at the page; `.got.plt` and any writable tail start
  there. Verified with a data-bearing lib:
  `glob=41; bump(); bump() -> glob==43, get()==50` through copy-reloculation under qemu-riscv64.
- **aarch64 had no `PT_GNU_RELRO` at all** (both emitters). Added with bfd group composition:
  `emit_shared`: `[init/fini arrays, .dynamic, .rela.plt, .got]` closes at the 64 K page edge,
  `.got.plt` relegated to the tail; `emit_dynamic`: same group incl. `.data.rel.ro` — which
  previously sat *after* `.got.plt`, i.e., permanently writable despite its name. Verified with the
  same data-mutation runtime test under qemu-aarch64 with a **lccc-compiled exe against an
  lccc-compiled lib**, rc=0: `GNU_RELRO 0x30000 +0x10000`.
- i686/arm32 keep the deliberate no-RELRO design (documented in earlier followups: their emitters
  produce 4 KiB-page non-RELRO layouts by design); no change.

The x86-64 emitters already had the correct semantics (`relro_pad` + split-RW-LOAD machinery from
S02) and verified lld-identical pair values — untouched.

## 3. riscv `-lfoo` -> DT_NEEDED drop (found by validation, fixed)

`gnu_hash_sizing`-style cross check failed with `./rtest: undefined symbol: f1` on real riscv64
glibc. Root cause in `src/backend/riscv/linker/input.rs::discover_shared_lib_symbols`: a NEEDED
entry was recorded **only** when `find_versioned_soname` located a `libfoo.so.N` sibling
(Debian-script path for libc/libm). An unversioned `-lr_rv` resolved its symbols at link time and
was then *absent from DT_NEEDED* → guaranteed runtime failure. Also broken: SONAME was never
consulted. Fix: for a real ELF .so input, record `DT_SONAME` (via `linker_common::parse_soname`
— handles section-header-less .so via PT_DYNAMIC) else the linked file name. Verified:

- `-lr_rv` → `NEEDED [libr_rv.so]`, runtime rc=0
- with `-Wl,-soname,libr_rv.so.9` → `NEEDED [libr_rv.so.9]`, runtime rc=0 through symlink
- i686/arm/x86 already had correct soname→NEEDED plumbing (`parse_soname{,_elf32}`); riscv was the
  only backend with this defect.

**Landed later in the same session (commit 'riscv: accept positionally-named shared libraries')**:
positional `./libfoo.so` inputs on riscv are ET_DYN-routed by `load_input_files` into
`register_positional_shared_libs` (DT_NEEDED = DT_SONAME, else file name) — verified
`./libr_rv.so` → `NEEDED libr_rv.so.9`, runtime rc=0. Also landed later: **native i686 runtime
proof** — `lccc-i686` exe + `-lbig_i686` (600-export lib built by lccc-i686) resolving through the
real `/lib/ld-linux.so.2` (rc=0, prints 649; also passes under qemu-i386) after
`libc6-dev-i386`/`libc6-i386` were installed. Gates: `ci_local.sh --fast` **34/0 ALL GREEN**
(5 earlier "failures" were missing i686 libc headers — environmental, fixed by install, not code).

Remaining gap at this level: `lccc-i686 -soname NAME` (space form) is not parsed
(use `-Wl,-soname,`), and `lccc-ld -m elf_i386` without a script is deliberately refused.

## 4. Suite state

- `run_linker_tests.py`: 230/0/0/1 (was 228 pre-session).
  New tests: `gnu_hash_sizing`, `crossarch_gnu_hash_relro` (lccc-i686/arm/riscv drivers' own linkers,
  phdr-parse without section headers — the i686 emitter writes none).
- `cargo test --profile fastbuild --lib`: 2826/0 (4 new hash.rs tests).
- Zero warnings under the fastbuild `-D warnings` profile at every landing.
- `/swapfile` 3 GiB verified active.

## 5. Next-session candidates (symmetric with ms178-task, priority order)

1. ~~i686 runtime validation~~ **DONE** (headers installed; native + qemu-i386 runtime proof).
2. ~~Positional `.so` inputs on riscv~~ **DONE** (ET_DYN routing in `load_input_files`).
3. ~~i686 `.so` section headers for bfd input inter-op~~ **DONE** (same session): the emitter now
   writes a minimal shdr table (`.dynamic`/`.dynsym`/`.dynstr`/`.gnu.hash`/`.shstrtab` ≈ 300 B) for
   shared objects — verified `gcc -m32` (system bfd) links a consumer against a 600-export
   lccc-i686 .so and the result runs natively (rc=0, h599(5)=649). Runtime-only users
   (glibc/qemu-i386) are phdr-based and were unaffected before/after.
4. ~~`-soname NAME` (space form)~~ **DONE — GCC-parity, not acceptance**: GCC rejects bare
   `-soname`/`--soname`/`--soname=` ("unrecognized command-line option"); lccc's driver silently
   swallowed the flag and misrouted its *value* as a phantom positional input (the baffling
   "not a relocatable object" diagnostic). cli.rs now hard-errors identically with the
   "-Wl,-soname,NAME" hint (lccc-ld keeps supporting all direct-ld forms).
5. **RELRO tail-split file density**: emit_shared riscv+arm pads the FILE up to the page when a writable
   tail exists; bfd does the same, lld/mold pad only vaddr (`new_segment()` trick like x86
   `split_relro_load`). Port that to riscv/arm to save ≤ (page − relro tail) bytes per .so/exe.
   Deliberately deferred: it changes the RW LOAD topology on the two big emitters; byte-counts
   never outrank kernel-safe green oracles.
6. ~~arm `emit_dynamic`/emit_shared IFUNC/got.plt `-z now`~~ **DONE**: AArch64 emits DF_BIND_NOW /
   DF_1_NOW unconditionally, so `.got.plt` is link-time final and now sits INSIDE the RELRO window
   (bfd's `-z now` rule). Data mutation + PLT resolution at runtime verified under qemu-aarch64.
7. ~~emit_script vDSO-class re-measure~~ **checked**: `--filter script` suite battery green (8/8)
   after the sizing-rule swap; `.gnu.hash` emitted under scripts now uses the shared rule too.

## 6. Post-rebase state + PR #536 red-team audit (2026-09-15, evening session)

**Rebase**: my 10-commit series rebased onto origin/main 3670cac (which now carries arena PRs
#530-535: peephole, RA, CET/ISA linker, archive-weak/RUNPATH/.comment/ELF-hardening, SLP).
Post-rebase gates (individually, foreground, swap active): fastbuild 2m41s/2m07s clean;
cargo test 2829/0; linker suite 230/0/0/1; clippy green (119s); regression-corpus-ssa 761/0;
benchmark-output-oracle 204/0 (394s); differential-correctness 57/57; rustfmt green.
i686 shdr bfd-inter-op re-verified post-rebase (`mi_appF` → 649).

**"CI is red" — NOT REPRODUCIBLE**: every gate from `.github/workflows/ci.yml` reproduces GREEN
locally on the final tree; GitHub API shows main 3670cac check-runs all `success`
(00:34Z), and PR #536 head 9d572a4 has ZERO statuses — its CI never ran.

### PR #536 (Agent A, iterative ICF, 9d572a4) — verdict

**Scope: not a competitor, complementary.** My series = dynamic-link correctness/performance
(RELRO composition, hash sizing, bfd/gcc inter-op, cross-arch). #536 = static-layout ICF
(+2668-line icf.rs, 2795 total vs main's 641; ICF not merged — `merge-base --is-ancestor` rc=1).

**Empirical race, its own 100K-function corpus** (`scripts/icf_scale_corpus.py`, 102 objects,
100×1000 fns, 10-const fold groups, 200-link chain, PIE, full runtime):

| tool | mode | .text bytes | .text saved | wall (warm) |
|---|---|---|---|---|
| bfd 2.43-ish (no ICF) | — | 1,711,285 | 0% | 0.94 s |
| lld 23.x | --icf=safe | 1,711,285 | 0% (no fold) | 0.14 s |
| lld 23.x | --icf=all | 5,926 | 99.65% | 1.66 s |
| mold | --icf=safe | 5,926 | 99.65% | 0.14 s |
| mold | --icf=all | 5,926 | 99.65% | 0.17 s |
| **lccc #536 (lccc-ld)** | safe | 271,445 | 84.1% | 0.59 s |
| **lccc #536** | all | 271,445 | 84.1% | 0.59 s |

`[icf] mode=safe groups=11 folded=89990 bytes_saved=512943 rejected_unsafe=0 iters=32
comparisons=107510 shattered=167` — hits the MAX_REFINEMENT_PASSES=32 cap with 167 shatter
fallbacks; convergent tail (the 100×10 same-file call-edge functions and their neighbors)
never folds, hence the 15-pp gap to mold. Deterministic (identical sha256 across re-links);
trivial fold executed correctly (folded fa/fb, native rc=0); `--icf=none` = no fold.
Corpus program segfaults identically under bfd/lld/mold/lccc — generator property, not a
link defect. min-case works: two byte-identical `return 42` funcs fold to one address.

**Defects found**:
1. `is_call_or_jump` (icf.rs:593) implements only `0xE8 / 0xE9 / 0x0F 0x8x`; PR body + commit
   message claim short JCC `0x70..0x7F` and short JMP `0xEB` support — NOT implemented
   (`_ => false` = address-taken). Safe direction (over-conservative, no miscompile), but
   doc-vs-code mismatch and its tests don't cover the gap. One-arm fix upstream-side.
2. Zero CI evidence for the PR head; merge would be blind.
3. Fold-quality: pass-cap truncation at 32 with 167 shatters leaves ~84% vs mold 99.65%
   on its own corpus (still ≫ lld-safe 0%; 2.8× faster than lld-all, 3.5× slower than mold).

**Design credits (sound)**: only `.text`/`.text.*` foldable w/ output-section discriminant
(the crtn.o `.init`/`.fini` epilogue lesson is documented in-code); dead-section exclusion
pre-plan; FDE pruning for folded code (unwind guarded); stable IFUNC resolver protection;
weak/strong multi-definition marking; disjoint-set + content-hash bucketing w/ exact
confirmation; `--icf` actually plumbed through lccc-ld (that defect was separately fixed).

**Recommendation**: mergeable IF complementary-sequenced: (a) rebase onto 3670cac, (b) run CI
green, (c) add the `0x70..=0x7F | 0xEB => true` arm or correct the docs, (d) align test claims.
It does not supersede anything in this series; both should land.

### Next-session candidates (updated)

8. **ICF post-merge work** (if #536 lands): raise/scale refinement budget adaptively
   (cap by candidate-set size, not constant 32) to close the mold gap; benchmark on the
   kernel/glibc golden workloads; measure `--icf=safe` vs `all` divergence cases.
9. (5 from above) RELRO tail-split file density on riscv/arm — still deferred pending owner
   of emitter topology change.
