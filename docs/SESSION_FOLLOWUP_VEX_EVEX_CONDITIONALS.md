# Session follow-up: VEX/EVEX + conditional-assembly + GOT parity (ms178-1)

**Base:** `84962763772d5c7a3b7094a1b6d8818f44d7bc62` (ms178/lccc main)
**Deliverable:** `/home/user/ms178-1.patch` and `/home/user/artifacts/ms178-1.patch`

## Accomplished

### Conditional-assembly (GAS 2.47 parity)
Unified collector for the full head family (`.if`, `.ifdef`/`.ifndef`/`.ifnotdef`,
numeric `.ifeq`…`.ifge`, `.ifb`/`.ifnb`, `.ifc`/`.ifnc`, `.ifeqs`/`.ifnes`).
Lazy `.elseif`, sequential label tracking for `.ifdef`, verbatim diagnostics.
Battery: `scripts/check_conditionals_family.py`.

### XOP/LWP + i686 vector delegation
New `encoder/xop.rs`; i686 pure-vector/`v*` and XOP delegate to shared x86-64
core with reloc translation. EVEX unsigned converts in avx table.

### GOT-base symbol
`needs_got_base_symbol` (complete class set) + post-build UND emission;
`symbol_table` also emits bare `.globl`/`.weak` UNDEF.

### Driver ISA replace-and-replay
`-march=` resets arch-implied set then replays explicit `-m*`/`-mno-*` in
order. GCC-exact both directions. SSE4.2⇒POPCNT; integer macros not gated
on `-mno-sse`.

### Peephole soundness
cmpxchg implicit RAX write; load-test fold AF + address-family vetoes.

### Scripts
`ensure_gas_247` mirror chain; `distill_evex_opcodes.py`; `isa_gap_sweep.py`.

## Constraints
No container swap; 1.2 GiB RAM OOMs full `cargo check`/`build`. Validate on
a host with ≥4 GiB or swap.

## Next-agent backlog
1. Build + run conditionals battery + asm-diff new cases vs GAS 2.47.
2. `isa_gap_sweep.py --rank` → close MISSING mnemonics with distill scripts.
3. encdiff/insndiff/asmdiff vs GAS + Clang + ICX for best encoding.
4. i686 casefile coverage for every new XOP/EVEX path.
5. Extend implicit-write oracle to full atomic family.
6. `ci_local.sh --fast`, rustfmt, clippy on build-capable host.
7. Resume godbolt codegen quest (zlib-ng/gzip/expat) under `-march=x86-64-v3`.

## Red-team
Dual GOT paths collapsed; duplicate `unclosed-if` removed; all
`expand_gas_macros_with_state` call sites 4-arg; no test weakening.
