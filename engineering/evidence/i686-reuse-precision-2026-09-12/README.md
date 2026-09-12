# i686 reload-reuse precision (2026-09-12)

Closes the explicit follow-up in #505 §4 (`%esp`/`%ebp` in `parse_frame_slot`),
validated by execution where #505 could only diff: 32-bit glibc dev headers are
installed (`gcc-multilib`), and 32-bit ELFs run natively on the x86-64 host.

## What changed (`passes/dead_writes.rs`)

* `parse_frame_slot` accepts `%esp`/`%ebp` as families 4/5 — the same proof as
  x86-64, verified to transfer: `%esp`/`%ebp` map to families 4/5 in
  `scan_register_refs` (the `b'e'` arm), so `subl $N, %esp` ends the scan;
  push/pop/call/ret/Jmp breaks are arch-independent `LineKind`s; SIB and
  segment overrides are rejected by shared code; the proof never uses the
  red zone, and call-arg staging windows always adjust `%esp` first (which
  breaks the scan before any staged store is judged).
* #505's "two-line change" claim was INCOMPLETE: `classify_line` only
  recognizes 64-bit `pushq`/`popq`, so 32-bit pushes classify as `Other`
  and their implicit `%esp` motion is invisible both to the `Push`/`Pop`
  scan break and to `writes_family` (no effects-table row either). Found by
  the `i686_push_breaks_reuse` barrier test (the pass forwarded across
  `pushl %ebx`). Fixed with a textual push/pop break scoped to
  `%esp`-cached scans in this pass only — deliberately not pipeline-wide,
  because push/pop consumers elsewhere may assume 8-byte frames. A
  pipeline-wide 32-bit-push classification is a separate project with its
  own audit (it would also change how `callee_saves`/`frame_compact` treat
  i686 frames).
* 10 new unit tests: same-dst delete, disjoint esp/ebp forwarding, overlap,
  esp movement, push break, esp-vs-ebp conservatism, push-across-ebp
  precision, `leave` (via the effects table), and one full-pipeline
  end-to-end. `dead_writes`: 45/45 green.

## Measured

* Corpus sweep (`lccc-i686 -O2 -S` over benchmark programs + kernel corpus +
  regression corpus, new vs pristine `a0e03144`): **0 differing TUs**. The
  precision is currently inert on i686 (shapes do not occur), so the
  "execute every differing TU" requirement is vacuously satisfied — and the
  change provably cannot regress what it does not alter.
* `asmdiff.py --32`: 20/20 vs GAS. i686 adler32 executes natively under both
  binaries to identical output (`8c331ae0`, rc=0) with identical hashes
  (`ec011476…`, `paired_ab` UNINFORMATIVE): zero impact, as predicted.
* x86-64 blast radius: none (no shared-path edits; the textual break only
  matches 32-bit push/pop mnemonics, which x86-64 output never contains).

## Follow-ups found while testing

* Downstream load-folds reintroduce memory operands after reuse forwards
  (`movl S,%ecx; addl %ecx,%esi` → `addl S,%esi`): legitimate (it deletes an
  instruction), but it hides this pass's firing end-to-end on both targets.
* The unconditional `Push`/`Pop` scan break also fires for `%rbp`-cached
  scans, where pushes are harmless: precision loss on both targets.
