# ideas/

Ranked raw notes. **Authoritative backlog:**
[`../engineering/agent/BACKLOG.md`](../engineering/agent/BACKLOG.md) — ideas
graduate into the backlog (and then into
[`../engineering/tasks/`](../engineering/tasks/) as actionable queue files)
only when they have evidence and acceptance gates.

# Improvement backlog (not yet scheduled)

Ranked ideas with evidence. Naming: `<priority>_<topic>.txt` for
enhancements, `fix_<area>_<symptom>.txt` for latent defects that need
investigation before they can enter the engineering queue.

Priority reflects (expected impact × workload breadth × confidence) /
implementation cost — see
[`../engineering/agent/SEQUENCE.md`](../engineering/agent/SEQUENCE.md) for
the scored performance order; this directory holds everything else.

## Status of the 2026-08 structural list (re-checked 2026-08-28, v3 doc audit)

Several items from the original high-value list have since landed; do not
redo them:

1. **Dead-store elimination** — same-block DSE is landed (`dse.rs`,
   `CCC_NO_DSE`); cross-block extension is BACKLOG OP-13b.
2. **Vectorizer cost model** — the per-natural-loop PGO profitability gate
   is landed (exact trips; trip <8 rejected, >80-inst bodies need ≥32).
3. **Reload-at-next-use + remat** — interference half landed (RA-05a,
   segment-aware scan + Tier-2 graph coloring); the splitting half is
   RA-06, the top P0 item.
4. **outline_switch** — threshold is 40 (was 999999), landed.
5. **CI wiring** — the structural codegen gate (`ci-codegen-gate.py`) is
   now wired into the benchmark workflow; extending CI to the full test
   suites remains MS-03.
6. **Vector register retention across intrinsic boundaries** — the 5.3×
   zlib-ng SIMD gap from the deleted `docs/simd-audit.md` was addressed by
   the vector-temp-promotion rewrites (sessions 42–45) and the
   segment-aware vector allocator; re-measure before reopening.

Still open (see the .txt files):

1. **Typed AST / sema expansion** (`high_sema_expansion_typed_ast.txt`).
2. **SCEV** (`optimization_passes_future.txt`) — would subsume both IVSR
   pattern matchers and unblock deeper aliasing (OP-28).
3. **Parser panic-mode recovery** (`ideas`, FE-07) — one missing `;`
   still cascades.
4. **Use-def-chain shared optimizer context** (`high_use_def_chains.txt`,
   FE-13).

## Linker follow-up (post 2026-08-15 sessions)

Landed since: congruent segment packing, `parallel_reloc.rs` (UB-fixed,
opt-in `LCCC_LD_PARALLEL`), ICF analysis (`LCCC_LD_ICF`), `--icf=safe`
rules, exclude-libs/PLT fix, `SIZEOF_HEADERS` single-source fix, COMDAT
exe-path dedup, `--as-needed` GROUP-script fix, `-D warnings` build gate.
Still open (BACKLOG LK-*): mmap I/O (LK-01 — note: `Vec→Arc` measured
always-copy; needs real mmap), `SymbolId(u32)` interning (~7 %, 149 call
sites, on top of SymStr), thin-archive shared buffers, `--emit-relocs`
(LK-16), ELF32 script setup.elf (LK-15), grammar-fuzzer depth.
