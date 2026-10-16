# S04 follow-up: closed-PR audit, #757 re-land, and the two main bugs it exposed

Session branch: `arena/d3169e6d-lccc`, PR #844. Base: main `e8ae10ab` (merge
of PR #843). Companion to `FOLLOWUP-2026-10-16-rebase-synthesis.md` (S03).

## What landed (commit order)

| commit | content |
|---|---|
| `71ea23ee` | A13/A14 libcall policy (from #757 P1), 5 fixtures |
| `774da9e3` | Sparse NOBITS `.bss` (from #757 P2), measured RSS table in the message |
| `a185ee1b` | RA-01 sym+reg SIB fold (from #757 P3), seg exclusion is NEW |
| `324f6fa7` | reloc 11 = `R_X86_64_32S`, not `@GOT64` — -fno-pie jump tables SIGSEGVed |
| `5305a28d` | RA-01 gate + fixtures; dormant-fold RA-neutrality (static deadening preconditions in the link extension) |
| `804b73f4` | Array-bound contract (from #757 P4): decay, u128 byte-size, -m32 truncation |
| `020b8a59` | Gates local+hosted (parity), sparse-bss gate case 6 re-measured against GAS 2.47 |
| `07c64842` | `__CHAR_UNSIGNED__` derives from `char_is_unsigned()`; char gate 11/11 |

## Decisions that differ from #757 (with the measurement)

1. **Sparse gate case 6**: 757 demanded a diagnostic for `.p2align 40` at
   offset 0. GAS 2.47 ACCEPTS it (752-byte object, `sh_addralign 2^40`,
   measured) and lccc agrees byte-for-byte; only the padding-bearing variant
   (`.byte 1` first → 2^40-1 fill bytes) is diagnosed, by GAS and by lccc.
   The gate now pins both measured subcases. Rule of the whole session:
   757's comments were written against binutils 2.40/2.44 — main's GAS-2.47
   remeasurements won every conflict.
2. **RA-01 link extension**: 757 linked `ro.off` on map membership alone
   ("harmless over-constraint"). Against main's drift that moved the RA of a
   DORMANT build (+1 copy in ra01_global_match). The extension now replicates
   the deadening pass's two static preconditions (every use of the `Add` is a
   mapped indexed-GEP base; no store consumer) — RA-independent, so an `Add`
   failing them can never need the offset register at the access.
3. **RA-01 seg exclusion** (not in 757 at all): main's seg-indexed emit
   ladder postdates the PR and rebuilds refused addresses via
   `rematerialize_skipped_indexed` from the GEP BASE home — a home a deadened
   `Add` never writes. `build_reg_offset_sym_map` drops any `Add` reachable
   from a seg-qualified access (directly or as GEP base).
4. **Option contract NOT ported** (deliberate): `LCCC_STRICT_OPTIONS`, the
   fail-closed data-model/dialect rows, the `-D_FORTIFY_SOURCE` diagnostic
   and `scripts/lccc_capability_probe.py` belong to 757's cli.rs hygiene part
   (deferred). `docs/builtin-libcall-policy.md` §"Option contract" documents
   the CURRENT behaviour and names the gap. Measured today:
   `-funsigned-char`/`-std=<unknown>`/`-fno-plt`/`-D_FORTIFY_SOURCE=2` are
   accepted, `-fstack-protector` exits 1.
5. **757's script bugs fixed before registration**: undefined name `bss` in
   a reject-path f-string of `check_array_bound_contract.py` (masked only by
   loop-variable leakage), unused import/locals — pyflakes clean now.

## Main bugs found while validating (both fixed here)

- **reloc-11 GOT misclassification** (`e3e7434d` regression): every
  `-fno-pie` switch jump table read from absolute address 0. Found because
  the corpus A/B for RA-01 ran under `-fno-pie`; four programs SIGSEGVed
  with the MAIN binary too (worktree-verified before blaming the port).
- **char-signedness three-way drift**: type said unsigned, macro said
  signed, limits.h followed the macro.

## Still open (carried forward)

- cli.rs option-contract port + capability probe (see §4 above).
- Deep-dive queue by novelty: #714, #712, #816/817, #766/767, #709/705/706,
  #777, #833, #835, #779/778, #814, #697, #831, #828, AArch64 #761.
- Harness: Clang-23.1 leg, godbolt-oracle script, i686-mold preset,
  `.flags` -O2 audit, ICX-2026 PPA, latest-rust attempt.
- Weak kernels vs gcc (geomean 0.661x): classify 0.27x, namechars 0.25x,
  seg_gs_fill 0.36x, seg_gs_copy 0.42x, seg_gs_sum 0.51x, varint 0.66x,
  strcmp_signed 0.69x.
- i686 GOT32 → GOT32X relaxable spelling (documented ledger item in the
  got-slot gate).

## Session traps worth re-reading

- `git apply --3way | head` dies on SIGPIPE and reverts ATOMICALLY — the
  "Applied cleanly" lines lie. Run it unpiped, inspect after.
- Benchmark corpus A/B needs a timing-text exemption (i686_alu_chains
  prints seconds).
- Commit messages with backticks must go through `-F file`, never `-m` with
  a double-quoted heredoc-less string (shell substitution ate `ro.off`).
