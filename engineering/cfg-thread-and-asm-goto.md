# CFG threading and asm-goto edge audit

Independent red-team of the cumulative follow-up, rebased on
`7e9444174f9b76181504329cd4ff8010b1a1483b`. This is not a correctness proof
and not a claim that every function is optimal.

## Reproduced defects

1. **Compound assembler statements.** `.cfi_*`, `.size`, `.loc` and alignment
   predicates inspected only the first token. `.cfi_def_cfa_offset 8; .byte 0xf9`
   was classified as harmless, so a window could step across executable bytes.
   Prefix lookalikes such as `.p2alignment` were also accepted. Both predicates
   now require one complete statement, with quotes and trailing comments
   honored, and reject `;`. Unknown tokens fail closed. Witnesses failed
   before the change and pass after it.

2. **IR jump-chain mutation.** The old threader rewrote a terminator, then
   tried to roll one edge back if a PHI was uncovered. Two rejected edges
   wrapped the change count through zero (`18446744073709551615`) and left a
   partial CFG edit. A second defect treated “a PHI arm for this predecessor
   exists” as sufficient and could replace an asm-goto value with the normal
   fallthrough value. A third rejected every sole-predecessor relay, including
   ones whose PHI translation was available.

   The replacement plans every terminator and PHI addition from an immutable
   snapshot and writes nothing unless the whole source is valid. Missing or
   duplicated required arms reject the source. An existing arm is kept only
   when the translated value is equal. Chain resolution is cycle-complete and
   has no depth cap; PHI-arm growth is bounded by the original arm count.
   A sole-predecessor relay that already feeds a PHI is not bypassed: that
   relay is what keeps the arm alive, and removing it changed loop-header
   rematerialization and miscompiled a vector map kernel. Multi-predecessor
   relays remain threadable. Empty asm-goto sources are no longer fused into
   their fallthrough, which had coalesced two different incoming values under
   one predecessor label.
   The pre-fix compiler returned 9 for an asm-goto fallthrough that must
   return 1. After the fix, `-m32` and `-m64` at `-O0/-O1/-O2/-O3/-Os` all
   print `fd6c281bebb77835`.

3. **i686 trivial-block deletion.** `thread_trivial_blocks` counted branch
   references and then discarded the count. It deleted `.LBB5: jmp .LBB3`
   while `#APP` still contained `jnz .LBB5`. The unresolved symbol became
   address 0 and `-O0 -m32` segfaulted. The same bytes were produced by the
   pre-audit compiler, so this was pre-existing, not caused by the IR change.
   x86-64 does not delete labels. The i686 pass now keeps a trampoline when
   any reference is not a retargetable branch or `.long`/`.quad` token,
   including inline asm. Ordinary branches may still skip that trampoline.
   Rewriting the asm text would be shorter by one jump on that edge, but
   `%l[name]` can be address-taken; the opacity rule wins.

## What was not copied from LLVM

Current LLVM `JumpThreading.cpp` (`ca1dbb8ee2aa`, 2026-10-13) evaluates
conditions on predecessor edges, duplicates blocks under a size threshold,
and refuses loop headers, convergent calls, address-taken blocks and
unprofitable copies. Its own history includes quadratic IR growth when
zero-cost instructions bypass the threshold. LCCC already folds constant
branches and threads bool-phi diamonds without duplication. Cloning that
duplicating pass without LazyValueInfo, a dominator updater and a measured
cost model would add compile-time and code-size risk without a demonstrated
runtime win on this tree. The empty-block threader therefore stays
duplication-free. That is a scope decision, not a claim that no further
threading is profitable.

## Oracle

GCC 16.2.0, Clang 23.1.0, ICX 2024.0.0 and the catalog's ICC 2021.10.0 all
printed the same hash at `-O2` and `-Os`, with no warnings under
`-Wall -Wextra -Werror` (ICC deprecation diagnostic suppressed only for that
probe). ICX's provider alias resolved to 2024.0.0, not a proof of Intel's
newest 2026 release. No vendor-native timing was collected. This host is a
2-vCPU Xeon family 6 model 106 KVM guest, not an i7-14700KF, so no desktop
cycle claim is made.

## Remaining limits

Hosted checks have not run on this head. The fast local gate is the required
local contract; it is not GitHub CI. No universal speedup is claimed. The
retained asm-goto trampoline is one jump on that edge only.
