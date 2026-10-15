# Latest-main integration — 2026-10-15

Base refreshed from `63e783b15586eca86f306bd75a52b82c57304bbb` to
`f07f4c514460e4116a574773cd84b69d185bf65e` (PR834, implementation `2b69047`).
The complete S82 branch/checkpoint was preserved before integration, including
`artifacts/pre-f07-session.bundle` in the delivery workspace. The cumulative
work is consolidated onto the new base; it is not a blanket “ours” resolution.

- Retain upstream VEX/region-liveness/vector-copy safeguards, reduction definition
  indexing and malformed-SSA refusal, IVSR topology assertions, tested-binary
  benchmark workflow and read-only evidence integrity gate.
- Retain session 128-bit strict integer packing and its exact-width lowerer,
  prior semantic/wrapping/FP fixes, oracle contracts, native artifact lifecycle,
  fail-closed preboot verdicts, and the new GP forwarding safety checks.
- Resolve the CI conflict by keeping **both** integrity and resource gates.
- Resolve the vector-copy test conflict by keeping all five upstream test groups
  and the session's eight gather-mask opcode cases.
- The old upstream 256-bit `StrictPackedDag` type was already replaced by the
  session's 128-bit builder. Port PR834's new transactional-allocation regression
  to that builder instead of resurrecting an unused implementation or dropping
  the new contract. Retain the other new reduction/CFG tests unchanged.
- Clarify the packed intrinsic as one scalar F64 result, with four signed I32
  lanes occupying exactly 128 bits and ordered, unreassociated FP operations.
- Keep the **byte-identical protected upstream original** at its canonical
  `engineering/PR831-AUDIT.md` path. The independently extended report is retained
  separately as `engineering/PR831-COMPILER-AUDIT.md`. All 69 catalog entries keep
  their original byte counts, hashes and canonical scope; modified prose must
  not replace a frozen record or require blessing new expected hashes.

## Forwarding phase-order regression

The pre-integration full Rust suite found three missed compare/branch fusions
(4785 passed / 3 failed / 8 ignored). Early replacement of a memory reload erased
the original operand needed by the existing boolean-slot census. Do not relax
that proof or change the tests to accept worse code: run GP stack forwarding
**after** compare/branch fusion, still under the existing `combined` ablation
control. Preserve raw load text when deleting a genuine 64-bit self-copy;
32-bit self-copies must retain their upper-half zeroing write.

This document records integration decisions, not acceptance. Fresh compilation,
full Rust units, architecture/contracts and FAST remain mandatory. Measurements
labelled S79/S77 are on the former base and are not final-new-main measurements.

## Subsequent semantic audit supersedes the legacy fusion contract

The pass-order repair passed S84's 4,800 tests, but five native counterexamples
then proved the inherited raw late fuser unsound. See
[FUSION-REDTEAM-2026-10-15.md](FUSION-REDTEAM-2026-10-15.md). Preserving an old
positive test is not a reason to preserve deletion of live state. Positive
fixtures are being made semantically complete, retaining their fusion assertions;
fixtures that explicitly required live-value deletion now require preservation.
The shared proof-gated fusion is given ownership before load/test-to-memory-CMP
folding, as well as before GP forwarding. Its late invocation restores all pins
and uses the same implementation rather than a weaker textual fallback.
