# Guarded scalar folds and unwind identity

Baseline: merged main `cfb2e0a822e2dba42a3d474f5970ab529e0e5e84`.
The baseline compiler was built warning-denied with the fastbuild preset and
copied out before editing Rust. Candidate and baseline executables are immutable
separate files; timings never share a mutable Cargo output.

## Source review and disposition

Fourteen original bodies and their accessible comments were retrieved through
the GitHub API. `guarded-folds-sources.json` records their provenance. This is a
review cohort, not fourteen claimed new optimizations. Closed upstream issues,
old IR, alleged speedups and suggested assembly are not implementation proofs.

| Report | Disposition and scope |
|---|---|
| [1884](https://github.com/llvm/llvm-project/issues/1884) | Historical XMM splitting/window-spill problem remains a distinct allocator problem. The hot-byte GPR copy fix below does not implement general XMM live-range splitting. |
| [3007](https://github.com/llvm/llvm-project/issues/3007) | AddRec/spill report reviewed; latest comment reports no spills. No new polynomial-recurrence or general rematerialization claim. |
| [6509](https://github.com/llvm/llvm-project/issues/6509) | Historical integer/XMM domain transfers, mostly resolved upstream. Current comment explicitly retains a genuine integer-compare transfer. Not a justification to delete arbitrary MOVD operations. |
| [10156](https://github.com/llvm/llvm-project/issues/10156) | Destructive CMP-to-INC/DEC is a size policy, not a generic throughput improvement. Not enabled for speed. |
| [12193](https://github.com/llvm/llvm-project/issues/12193) | Six-byte span inside a 22-byte object: an eight-byte load can overread. The native boundary fixture places byte 21 immediately before a protected page. No speculative widening is introduced. |
| [13134](https://github.com/llvm/llvm-project/issues/13134) | General in-place byte-reversal loop remains separate from existing load+BSWAP/MOVBE recognition. No unsafe widening or invented vector reduction is added. |
| [13687](https://github.com/llvm/llvm-project/issues/13687) | Late tail merging reviewed; existing epilogue/block passes are not recounted as new. General branch-layout/unwind-tail merging remains open. |
| [7145](https://github.com/llvm/llvm-project/issues/7145) | CH/MAJ and BMI1 ANDN are existing LCCC mechanisms. The C CH witness is an already-handled control. Comments discuss undef/freeze and target-dependent XOR serial dependencies; no blanket canonicalization is justified. |
| [212139](https://github.com/llvm/llvm-project/issues/212139) | Implemented reduced Boolean-mask conditional negation, including comparison-derived masks and known-bit masks. Arbitrary nonzero integers and shared expensive networks are refused. |
| [33226](https://github.com/llvm/llvm-project/issues/33226) | Implemented terminal register-select commutation into the ABI result home. TEST is deliberately retained after BMI2/effectively-zero variable shifts. Removing that TEST generally would be incorrect, not missing performance work. |
| [46623](https://github.com/llvm/llvm-project/issues/46623) | Implemented signed lower-guard elimination with a proven nonnegative upper bound, preserving comparison width and the Boolean result's type. Unknown/negative upper bounds remain guarded. |
| [233455](https://github.com/llvm/llvm-project/issues/233455) | Current promoted C byte division already materializes 123 with MOVL and divides at promoted width. The reported MOVB+MOVZX setup is not reproduced; not counted as a new fold. |
| [233785](https://github.com/llvm/llvm-project/issues/233785) | Implemented the null-pointer/Boolean-PHI join from the OpenSSL-shaped example. Only pure comparison/cast work moves; predecessor identity and operand dominance are checked. Not a general relational PHI solver. |
| [230039](https://github.com/llvm/llvm-project/issues/230039) | Auditing found an actual safety hole: text-only ICF could fold different catch types. Added immutable CIE/FDE/LSDA/relocation identity. Exact identical-unwind folding remains available; different personality/typeinfo/CFI/LSDA cannot fold. This is a correctness repair, not a claimed speedup over unsafe folding. |

## Real-workload backend tuning

* The byte-reassembly loop used RBP as an ordinary saved GPR but copy folding
  rejected every RBP family. Reuse the existing function-wide frame/CFI/opaque
  use proof before admitting it. RSP stays excluded. Low-lane restrictions,
  source clobber checks, implicit-register exclusions and CFG liveness stay in
  force. Three staging copies disappear from the existing hot unrolled loop;
  no load width or memory footprint changes.
* Switch dispatch masks the index to 15, then sign-extends its dword spelling
  solely to address the jump table. The nearest-write proof requires **both**
  clear sign bit and zero upper half; zero upper 32 bits alone is insufficient.
  A bounded block-local proof permits a full-register identity and removes that
  extension. Unknown/partial writes, labels, calls and pinned instructions stop
  the proof.
* Negated select arms can be computed in the result home without the second
  copy. The temporary must die, the result cannot be observed early, and the
  optional intervening compare/test must mention neither register. No flags,
  memory operations, CFI or calls move.

## Integration and semantic boundaries

Scalar work reuses the existing bounded `bit_idioms` observation context and
its early/late scheduling and disable policy. Fresh IDs and source spans are
preserved; producers and shared users are not edited. Conditional-negate
matching requires the exact same SSA Boolean in the arithmetic mask and add,
with consistent scalar width. A comparison-derived Boolean may bypass only
integer copy/cast chains, which preserve zero and one. Integer NEG in this IR
is wrapping (as in the existing constant evaluator), with no added no-wrap or
poison assertion. This matters for signed MIN with Boolean zero: the original
source is defined, and the unused negated arm must not introduce UB. Native
signed 32/64-bit MIN/zero controls pin that case as well as the bit-vector tests.
Range matching does not
peel mismatched-width comparison operands.

The CFG follow-up admits exactly two real predecessors with matching, unique
PHI labels. One pointer input is null and the other Boolean input is exactly
one. A definition/dominator check proves the new predecessor comparison's
operand available there. Result PHIs are inserted in the PHI prefix; only pure
comparison/cast work is speculated. Existing DCE, not this transform, owns
removal of orphaned loads and preserves volatile accesses and calls.

ICF metadata identity is immutable: it does not refer to evolving ICF classes,
so the existing dirty-class worklist has no invisible new dependency. CIE
backpointer position is normalized; actual CFI bytes, relocation offsets/types/
addends, global-name binding, LSDA bytes and typeinfo/personality identity are
not discarded. Local self-text references normalize to the owning function;
unrelated local references keep object-qualified identity. Metadata relocation
ranges must be supported, in bounds and nonoverlapping. Missing/malformed
associations, interior/multiple FDEs, recursive LSDA graphs or the 16 MiB
per-object copied-byte budget make identities opaque. These are explicit fold
losses, never an unsafe fallback. No `.gcc_except_table` data folding is enabled.

## Evidence and remaining work

`check_llvm_guarded_folds.py` covers native optimization levels, debug mode,
exact GCC/Clang oracles, disabled controls, real freestanding ELF32 execution,
guard-page boundaries, call effects and assembly mechanisms. Unit tests cover
negative masks/bounds, frame/CFI contracts, source clobbers, PHI edge/width errors,
and different unwind identities. The root-level corpus wrapper is self-contained;
the dedicated checker separately compiles the subdirectory driver and helpers
to preserve opaque call effects. `prove_guarded_folds.py` provides independent
bit-vector checks and required SAT refusal witnesses; it is not a matcher or
machine-code certificate.

Measured runtime and final gate outcomes are recorded in `GUARDED-FOLDS-BENCH.md` and
associated JSON. No VM result is an EPYC 7763 measurement or a claim of global
optimality. General XMM splitting, arbitrary swap-loop vectorization, unsigned
XOR-with-IV reduction vectorization, LSDA class-graph equivalence beyond exact
identity, and historical branch-layout/runtime gaps remain explicitly open.
