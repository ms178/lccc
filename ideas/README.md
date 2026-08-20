# Improvement backlog (not yet scheduled)

Ranked ideas with evidence. Naming: `<priority>_<topic>.txt` for
enhancements, `fix_<area>_<symptom>.txt` for latent defects that need
investigation before they can enter `current_tasks/`.

Priority reflects (expected impact × workload breadth × confidence) /
implementation cost — see `hotspots/RESEARCH_BACKLOG.md` for the scored
performance queue; this directory holds everything else.

## High-value engineering (from 2026-08 audits)

The following items were validated against the codebase during the
GLM-audit review (2026-08-15) and remain the top structural work:

1. **Typed AST / sema expansion** (`high_sema_expansion_typed_ast.txt`)
   — sema still emits ~1 type diagnostic; the lowerer carries ~382.
2. **Vector register retention across intrinsic boundaries** — the
   5.3× zlib-ng SIMD gap documented in `docs/simd-audit.md`; re-measure
   before starting (backend value-tracking, not a new pass).
3. **Dead-store elimination** — `dce.rs` treats every Store as
   side-effecting; pair with pure/const call-attribute propagation.
4. **Vectorizer cost model** — `vectorize_gate` returns `true`
   unconditionally (pgo/unroll_pgo.rs:122); 4-iteration loops get
   vectorized for zero win.
5. **Reload-at-next-use + remat** (`register_allocator.txt`) —
   `future_uses` exists; 2026-08-20 gzip `longest_match` is 118 vs 0 stack-mem.
6. **SCEV** (`optimization_passes_future.txt`) — would subsume both
   IVSR pattern matchers (~2k LOC) and unblock LICM GEP aliasing.
7. **Parser panic-mode recovery** — one missing `;` still cascades.
8. **outline_switch**: either lower `MIN_CASES_FOR_OUTLINING` from
   999999 to a real threshold with a size-mode gate, or delete the pass.
9. **CI wiring** — the harnesses under `tests/` are not run by
   `.github/workflows/ci.yml` (build + `cargo test --lib` only). All
   five suites are verified green locally; wiring them requires
   workflow-file permissions that repo automation currently lacks, so
   this needs a manual PR by the maintainer:
   correctness, regression, intrinsics, linker (bfd+mold oracles),
   differential-fuzz smoke (120 seeds, O0/O2).

## Linker follow-up (from 2026-08-14/15 sessions)

Remaining items from the linker work (kernel links & boots; -r, PIE,
build-id, RELRO, .eh_frame_hdr, strmerge all landed):

- mmap-based input/output I/O (biggest remaining speed lever vs wild)
- parallel relocation application (rayon-style, per output section)
- ELF32 script path for setup.elf (`-m elf_i386` currently GNU ld)
- `--emit-relocs` for CONFIG_RELOCATABLE kernels
- positional `--start-group` semantics in the userspace driver path
- `--icf=safe/all`, `--exclude-libs`, `-Map=` real output
