# Follow-up — 2026-10-18 — PR #858: loop_memset k16 (Slt + runtime fill byte), #859 merge, CI lanes

Branch `arena/41471e5a-lccc`, session date 2026-10-18. Oracle: **GCC 16.2.0**
from the repo's own `harness-cache-1` tag (sha256-verified), binutils 2.47,
mold 2.42.1 — per the user's standing rule ("I do not care about GCC-12").

## 1. k16_memset8 — the one real -O2 perf miss in the 24-kernel survey, fixed

Two refusals, in order (the first fix exposed the second — read the FULL
`CCC_MEMSET_LOOP_TRACE` refusal chain before scoping):

1. **Slt exits refused.** `for (int i = 0; i < n; i++) d[i] = v;` lowers its
   exit as a *signed* first-reach compare. The exit and guard recognizers now
   accept `Ne | Ult | Slt`. Trip count from a new helper `slt_trip_count`:
   `init <s bound ? (size_ty)bound - (size_ty)init : 0` — computed in the
   POINTER-WIDTH type, because the same subtraction in the iv's 32-bit signed
   type wraps for `init = INT_MIN, bound = INT_MAX` (gives -1 → SIZE_MAX byte
   count → memset scribbles 2^64-1 bytes). The zero-trip case is skipped by
   the existing `n != 0` guard.
2. **Runtime fill bytes refused** ("stored value is not a constant"). The
   census now classifies a *loop-invariant* value stored through a 1-byte
   store: `BodyCensus.store_val: Option<Operand>`; invariant = not an iv phi
   and not defined in the loop body block. Wider stores of runtime values
   stay refused on purpose — `d32[i] = v` writes every byte of `v`, not four
   copies of its low byte, so widening that to memset would change the
   program. `Plan.fill_byte` is an `Operand` end-to-end; emission widens to
   memset's I32 argument (`IrConst::I32` for the constant case, a cast for
   the value case).

Receipts (fastbuild binary, gcc-16.2.0 cross-linked harnesses):
- k16 at -O2 now emits `call memset@PLT` (gcc-16 emits `jmp memset` — same
  call, tail-shaped).
- `/tmp/memset_sound.c` battery: n=10 → 10 x's; n=0/n=-5/n=-7 → no writes;
  `init=3` offset → first 3 bytes preserved; const fill 7 → 7 c's; dynamic
  `scanf` n=INT_MAX → fills without wrap/crash.
- Cross-link (gcc-16 main + `lccc fill_only.o`) correct for n ∈ {0,1,-5,10}.
- 5153 lib tests 0 failed; corpus 978/9 (the 9 = 8×i686-multilib env gaps +
  segment_fill_copy_alias, unchanged); census held (`fns_over_500` stayed 94
  — the inline Slt arm tipped it to 95, so the arm was extracted into
  `slt_trip_count` instead of re-baselining); clippy -D warnings; fmt.
- Static-insn survey after the fix: k16 dropped from the >15% list
  (28 vs 6 = call scaffolding vs 6-insn tail jump — dynamic cost is one
  guarded branch + libc memset, a large win for any real n).
- Remaining >15% shapes are prologue scaffolding whose hot loops are tight:
  k05/k20/k23/k30 (versioned-loop headers), k10 (ffs 13v11), k13 (strcmp —
  loop weight parity, exit scaffolding only), k18 (bitreverse 30v18), k21
  (fir4 37v26). Dynamic parity verified for k13/k18; k10/k21/k30 hot-loop
  review still open (see §4).

Commits: `f25b7c3fd` (loop_memset), `391cd1c4a` (SysV one-lane FP), merged
main `87713facc` (#859 AVX one-%ymm/%zmm + gcc-16 oracle pinning) as
`916ae72f5`, CI fixes as `7d76a1639`.

## 2. Merge with #859 — clean except one gate union

Only `scripts/ci_local.sh` conflicted (my varargs gate vs their AVX-level
gate — unioned). My SysV `vector_size` V1SF/V1DF = `SysvPassing::Memory`
decisions (GCC 16.2.0-measured) auto-merged beside their 32/64-byte
one-%ymm/%zmm carrier work; the `ExtVector` one-lane-FP = SSE (Clang) path
was untouched by them.

## 3. CI lanes fixed on the merged tree

- **Pipefail/SIGPIPE detector (my own violation):** the F2 gate
  `check_fno_builtin_folds.sh` used `nm | grep -q` — exactly the pattern the
  detector exists for. Switched to `grep -c ' U fabs' >/dev/null` (drains the
  pipe). Detector rc 0, gate still PASS.
- **Codegen quality gate:** my tree carried a STALE baseline (from the
  pre-#859 era; zstd_count expected ymm=4/stackmem=7 codegen that no binary
  produced anymore). Root-caused by measuring main's binary against the same
  metric (both binaries measure identically except an 8-register RA hunk);
  took main's re-measured baseline, then refreshed all workloads against the
  merged line. Deltas: everything improved (insns −1..−22%) except
  expat_xml_scan pushes 6→7 and the zstd ymm 4→0 that main's own green run
  already carried.
- **Check driver option contracts:** passed on the merged tree locally;
  the failure at `391cd1c4a` is consistent with the pre-merge tree missing
  #859's phantom-%rax kill fix (their `sse_scalar_mul_not_rax_kill` lane).

## 4. Open leads (measured, not yet chased)

- **expat_xml_scan pushes 6→7**: one extra callee-saved push in the scan
  entry; loop body instruction-mix identical (diffed `-I$GCC_INC -O2`).
  Prologue-only, amortizes to noise. Lead: RA pressure after the
  varargs-spill line; a bisection over the 46 branch commits would localize
  it if anyone wants the push back.
- **zstd_count ymm 4→0**: the FSE loop's vectorization is gone in *main's*
  history already (their green gate measured the same scalar asm). Worth a
  vectorizer session: why does the zstd_count loop no longer take %ymm?
- **k10_ffs (13v11), k21_fir4 (37v26), k30_unroll_shapes (309v164)**: hot
  loops not yet weight-diffed against gcc-16.2.0 (k13/k18 verified par).
- **F6 still queued**: f32×1 `vector_size(4)` into the abi-grid fixture
  (the SysV MEMORY decision is committed; the grid fixture leg is not).
- **debs-bookworm.tar.xz + mold-2.42.1 + qemu (harness-cache-3) still
  packed** in the toolchain cache — unpacking closes the i686 env gaps
  (8 of the 9 corpus fails are i686-multilib).

## 5. Process notes

- The stash dance during the zstd bisect briefly reverted the working tree
  to `391cd1c4a` content; recovered via `git checkout -- .` (history was
  never at risk) and the pipefail fix was re-applied. Lesson: for
  old-binary comparisons use a **worktree** (as done for main), not
  checkout+stash in the live tree.
- CI logs are unreachable from the sandbox (results-receiver/
  blob.core.windows.net not in the allowlist) — reproduce lanes locally
  from `.github/workflows/*.yml` step commands instead.
