# CFG threading and asm-goto edge audit

Follow-up on `6733d68f41b00387a98d2a2658a86bbf49632c99` (PR #812,
`6495e6eb6076d6ddf61dc92f3a0b9a7a3dd044cb`). That PR head's hosted Test
Suite, Clippy, Compiler Explorer oracle, and LCCC-vs-GCC checks completed
green. This note describes the additional i686 label-safety fix. It is not
a hosted result for the follow-up head, and not a speedup claim.

## What the review got right

The i686 text threader scanned only `fstart..fend`, and the first window
starts at line 0. Module data is emitted before function bodies
(`emit_data_sections`). A later function's static `&&label` (`.long .LBBn`
in `.data`) is therefore outside the window that considers deleting that
label. The same pass treated every in-window `.long`/`.quad` as a jump
table and rewrote it. Those are different uses: a proven switch entry is
dispatch, a static initializer is an address.

`jecxz` is classified as `CondJmp` because its second character is `e`.
The encoder and ELF writer have no long form (`E3 cb`, rel8). Retargeting
it before layout can assemble only by accident. `jcxz` and `loop` were
accidentally safe because they are not `CondJmp`; that is not a contract.
Final rel8 range is not knowable in this pass: later folds and relaxation
still change sizes. A text-distance guess would be a false proof. The
relay stays. x86-64 already refuses `jcxz`/`jecxz`/`jrcxz` and never
deletes labels.

## What the review overstated

Hosted CI on the merged PR head is no longer pending. A green result there
does not cover this follow-up. The suggested split is rejected: the
label census belongs in the pass this change already hardens.

No corpus-wide non-debug O2 speedup is claimed. Prior four-arm Callgrind
and Os text notes belong to the merged revision; they were not rerun here.
Ir is not cycles. This machine is not an i7-14700KF.

## What changed

Reference classification is module-wide and use-based:

- Relaxable `jmp`/`jcc` may skip a pure trampoline. The label address does
  not change.
- A short-only branch (`jecxz`, `jcxz`, `jrcxz`, `loop`, `loope`, `loopz`,
  `loopne`, `loopnz`) keeps the relay. Ordinary edges may still skip it.
- `.long`/`.quad` is rewritten only when it is a contiguous entry under a
  symbol the module uses as `jmp *TABLE(, %reg, scale)` or `TABLE@GOTOFF`,
  and the payload is `CASE` or `CASE - TABLE`. `static void *p = &&label`
  and `&&a - &&b` do not match. An address pin also blocks dispatch
  retargeting of that same label.
- The label is deleted only when every reference was retargeted and the
  name is defined once. Inline asm stays opaque (`%l[name]` can be
  address-taken).
- Jump-to-next folding still deletes a redundant `jmp` for dispatch-only
  labels. It does not delete the jump that is the first instruction of an
  address-observable label: that would make `&&mid == &&end`.
- The pin scan and the rewrite skip GAS quotes. `.string ".LBB1"` and a
  quoted comment are text, not a symbol use.
- Fall-through elision is not address-preserving. An empty block whose
  address is observable (`LabelAddr`, a static initializer, an asm-goto
  target) still emits the jump when that jump is the only thing separating
  it from the next label. A non-empty block still elides.

A sole-predecessor empty relay that already feeds a PHI is still not
bypassed. That relay is what keeps the arm alive; removing it miscompiled
a vector map kernel. Multi-predecessor relays remain threadable. Empty
asm-goto sources are not fused into their fallthrough.

## What was not copied from LLVM

LLVM's duplicating jump threader (`JumpThreading.cpp`, `ca1dbb8ee2aa`)
evaluates conditions on predecessor edges and copies blocks under a size
threshold. Its history includes quadratic IR growth when zero-cost
instructions bypass that threshold. LCCC already folds constant branches
and threads bool-phi diamonds without duplication. Cloning that pass
without LazyValueInfo, a dominator updater, and a measured cost model
would add compile time and code size without a demonstrated runtime win.
The empty-block threader stays duplication-free. That is a scope decision,
not a claim that no further threading is profitable.

## Oracle

The asm-goto fallthrough oracle is unchanged: `fd6c281bebb77835` at
`-m32`/`-m64`, `-O0` through `-Os`, matching GCC 16.2.0, Clang 23.1.0,
ICX 2024.0.0, and catalog ICC 2021.10.0 at `-O2`/`-Os` on the merged
revision. ICX's provider alias resolved to 2024.0.0, not a proof of
Intel's newest 2026 release. No new vendor timing was collected.

## What was run

`origin/main` was still `6733d68f` (PR #812). No hosted check covers this
follow-up. No O2 speedup was measured.

`tests/regression/check_i686_label_ref_safety.sh` linked and ran with the
fastbuild `-m32` compiler at `-O0`, `-O1`, `-O2`, and `-Os`. All four
returned 0. GCC 14.2.0 `-m32` on the same source returned 0 at the same
four levels. The corpus sidecar is `-m32 -O2`; without it the standalone
corpus compiles the file as x86-64 and the `#error` fires. At `-O0` the two `.long` entries are `.LBB10` and `.LBB11`,
and the compiler emits `.LBB10: jmp .LBB11`. Before that emitter fix the
labels were adjacent and `hop(-1)` returned 14. The backward `jecxz`
fixture enters past the far label; putting the far label at the function
entry returns before the branch, which is a test bug, not a relay bug.

The emitted `-O0` assembly assembles with `as --32`. Pointing `jecxz`,
`jcxz`, and `loop` at the far labels instead of the relays does not:
forward displacement 211 and backward `ffffff2c` are both too large for a
1-byte field. The i686 peephole module tests passed: 341, including the
quoted-text case.
