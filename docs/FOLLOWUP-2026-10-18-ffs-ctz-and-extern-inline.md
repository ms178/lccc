# Follow-up — 2026-10-18 · ffs→CtzNonZero idiom + external-definition inlining (PR #863)

Baseline: main `aec2a27` (post-#861; the pre-session main `8c63961` was
rewritten away upstream — this session's work was rebased onto the new root
and re-validated there).
Deliverable: **PR #863** from `arena/9b3930c8-lccc`, commit `21772cf`
(+ the postinline-gate fix), snapshot `ms178-1.patch` (origin/main..HEAD) and
`lccc-backup/lccc-21772cf.bundle`.

---

## 1. Landed

### L1 — guarded `ffs` shift-loop → one `CtzNonZero` (loop_idiom.rs)

The portable `if (!x) return 0; r = 1; while (!(x & 1)) { x >>= 1; r++; }`
shape (glibc `ffs`, kernel `ffs`/`__ffs`/`ffz`) is `r0 + ctz(x0)` guarded
iterations. The loop-idiom fixpoint matches it structurally and rewrites to
one dominating `CtzNonZero` plus `r0 +`:

* matcher phases (factored for the census): `ctz_collect_header_phis`
  (phi pair, steps in the latch), `ctz_header_test_polarity` (raw/`==0`/`!=0`
  test on `(x & 1)` with body-arm agreement), `ctz_escape_discipline`
  (word never escapes; counter only via the exit block or a phi edge leaving
  it), `ctz_entry_proves_nonzero` (the soundness gate), plus
  `ctz_preheader_edge` shapes `Uncond`/`CondTrue`/`CondFalse`.
* rewrite: preheader materializes `res = r0 + ctz_nonzero(x0)`; a
  conditional preheader keeps its guard arm and only the loop arm retargets
  to the exit; merge-block phis fed from the exit edge get the result
  (post-inline shape); loop blocks die.
* the nonzero guard is mandatory: the source loop does not terminate on
  x == 0, so no proof ⇒ no rewrite (unit-tested).

**Measurement** (ffs over 64 Ki random words, min-of-9, taskset): base
2.35 ns/elem → **0.83**, GCC 16.2 1.08 ⇒ lccc **1.31× faster than GCC** on
this idiom. GCC's `tzcnt` + `and $1` + `cmov` shape keeps both paths live;
`test`+`jne`+`bsf` does not. The rewrite fires on inlined copies too (merge
phi), so real callers benefit — verified via `CCC_DEBUG_LOOP_IDIOM` MATCH in
`main` and 5 `tzcnt` sites in the binary.

### L2 — external-linkage definition inlining (inline.rs, mod.rs, driver)

GCC at -O2 inlines small external *definitions* into same-TU callers while
keeping the out-of-line copy; lccc refused every non-static callee. Admission
(`extern_inline_admission`) mirrors the static windows, gated:

1. `-fPIC`/`-shared` **semantic interposition** ⇒ refuse (driver threads the
   mode through `run_passes`; GCC's `-fsemantic-interposition` default at
   -fPIC).
2. **scalar returns only** — the return-value merge is vetted for scalars;
   cloning an `[Sse, Sse]` two-eightbyte return mis-merged
   `sse_struct_two_reg_return`'s first value (caught by the regression
   corpus mid-session; the paired A/B also showed `matmul`'s main() cloning
   a scalar *klon* of the vectorized kernel).
3. **at most one natural loop** — matmul (3 loops, 34 IR instructions)
   cloned into `main` lost the vectorized form: 2.56× slower in the paired
   A/B. Multi-loop extern kernels keep the call and the optimized body.
4. **leaf callees only** — a callee that itself calls out (libm `fma@PLT`,
   the guarded `__lccc_cpu_init` probe initialiser) is refused: GCC declines
   this class too, and duplicating such sites broke two pinned CI contracts
   in the paired run (the fma ISA gate's byte-for-byte 8-call libm count
   became 10; the cpu_supports runtime gate's 2 initialiser calls became 4).
   Opaque inline asm is refused for the same reason. CI caught both; both
   gates PASS locally again with the gate.

The single-call-site death exemption stays static-only. Discovered and fixed
en route: **`CCC_DISABLE_PASSES=inline` was a silent no-op** — the late
inliner phase gated only on `"postinline"` and re-inlined everything the
disabled primary phase skipped. The late phase now also honors `"inline"`.

## 2. Receipts

* 5164 lib tests, 0 failed (after rebase onto `aec2a27`).
* Regression corpus **979/9** (the standing i686-multilib baselines +
  `segment_fill_copy_alias` only). The corpus caught both mid-session
  miscompiles that produced gates 2 and 3 of L2 — the harness works.
* clippy `-D warnings`, `cargo fmt`, audit census held (the two new
  over-500-line functions were factored back under the ratchet; 
  `knobs_undocumented` dropped 290→289).
* static-local 94/94 and seg-typeof gates PASS vs the GCC 16.2 oracle.
* Paired benchmark A/B vs the base binary (hash_table, binary_trees,
  ring_fifo, zstd_count, byte_load_reassembly, matmul), min-of-7
  interleaved: all within ±1% (matmul 0.994, sieve 0.995) — no regressions
  from the admission caps; the earlier full-corpus sweep (45 benches vs
  gcc-16, 2×9 reps) held the geomean within noise.

## 3. Verified-open (entry points for the next session)

* **Runtime-trip partial unrolling**: `try_partial_unroll_two_block` needs a
  constant trip; k21_fir4 (runtime `n`, overlapping loads) stays rolled
  where GCC SLP-vectorizes. Entry: a factor-k clone guarded by the trip
  check, PostVec phase, store+load profitability predicate already present.
* **k21 SLP**: GCC widens the fir4 dot to `pmuludq`; BB-SLP's
  `PackKind::MemLoad` could pack the a[i..i+3] window after the unroll above.
* **Global-accumulator store-to-load promotion**: a global `sink += …` in a
  loop keeps a load/store pair per iteration when the inlined body stores it
  twice (zero path + fast path); `loop_memory_promote` demands exactly one
  store. Entry: extend to two stores on mutually exclusive arms (both
  dominate the exit), or a pre-pass merging equal stores on arms.
* **k18_bitreverse8**: lccc keeps `movslq`-plumbed address webs + one extra
  register copy vs GCC's tight `shrb $4 / and $15 / orb` form (1.19–1.3× in
  microbenches, inside corpus noise). Entry: RA/narrow copy coalescing.
* **Performance survey standing** (from the /tmp/kgen2 era, lost in a
  sandbox reset — re-derive when needed): k05/k13/k20/k23 lead GCC; the four
  flagged kernels are addressed above except k10 (DONE this session: the ffs
  idiom is now 1.31× ahead).
* ICX 2026 / Clang 23 oracles remain CI-only (disclosed; gcc-16.2 is the
  local oracle).

## 4. Environment laws re-learned

* The sandbox resets mid-session (this session: twice). Durable state =
  `/home/user` only; `/tmp` evidence dies. Toolchains from repo tags
  (harness-cache-1 = gcc-16.2.0; rust-cache-1.97.0 = rustc/cargo/clippy —
  strip-components=2 unpack + rust-std into
  `~/.rust-bundled/prefix/lib/rustlib`), 8G swapfile re-created with sudo.
* `cargo clippy --version` needs the prefix on PATH (`cargo-clippy` shim);
  `clippy` alone is not a cargo subcommand binary.
* Origin main was **history-rewritten** to a single commit (`aec2a27`);
  `git merge-base` against old baselines fails (rc=1). Rebase with
  `git rebase --onto origin/main <old-base>` and force-with-lease the
  session branch.
