# Typed widening versus canonical source homes

Base: `2f477ce01749824ccab73ead7ccac90abd77fa08`. Targeted validation only;
see `typed-widen-2f477ce.json.gz` for compiler/source hashes and actual logs.

The signed-I32 canonical-home repair exposed four redundant sign extensions in
`double_reduction`: two hot counter updates and two immediately before explicit
widening casts. Do not waive the 135-instruction ratchet or remove canonical
homes wholesale. Raw-wide consumers still require the repair.

The pre-allocation analysis now distinguishes explicit I32/U32 -> I64/U64/Ptr
casts (which read low source bits and extend themselves) from raw-wide users.
GEPs, wide operations, stores, calls, returns, atomics, Select and transitive
Copy/Phi demand retain their existing treatment. F128 stays conservative. This
is an IR-wide distinction, not a lookup into the previous function's register
assignment map. The extracted helper is pure and the closure is cycle-bounded.

Cast-path audit: direct GPR/slot and verified accumulator casts extend explicitly;
XMM sources fall back to the default accumulator cast, which also extends.
Integer-to-I128 conversion first widens its low half explicitly. PF-15 deferral
accepts only byte/half sources, not I32. Nevertheless its shared widening helper
had XMM/no-home fallbacks that only copied bits: these now materialize then use
the same typed extension as ordinary GPR sources. Tests cover these fallbacks
rather than relying on their current restricted reachability.

Results with official fastbuild (Rust 1.99.1, O1, jobs 2, 6-GiB swap):

* Six new Rust analysis/emitter tests PASS. These are demand/assembly-shape
  checks, not native execution proof for every synthetic register assignment.
* Vector-copy gate: 47 PASS / 0 FAIL / 0 SKIP. `double_reduction`: **139 -> 135**
  static instructions; budget unchanged. This is not a hardware speed claim.
* Native witness matrix: 142 PASS / 0 FAIL.
* Exact GCC16.2 / Clang23.1.3 / candidate Memcheck matrix: canonical 60,
  signed-scale 30, domains 24 arms, all PASS. Both ABIs are included; canonical
  covers O0/O1/O2/O3/Os and both signed-overflow policies.
* The canonical fixture now executes 82 nonoverflowing cases: its original 32
  raw-wide address cases plus 50 explicit-widening checks. These include negative
  and boundary inputs, signed/unsigned destinations, zero extension, branch
  joins, volatile loads and loop-carried accumulation. Expected signed/zeroed
  boundary answers are independent constants, not the tested I32 computation.
* Oracle-environment harness tests: 2 PASS; rustfmt and diff check PASS.

Full exact-tree FAST/Clippy and fresh uncontended performance evidence remain
separate acceptance obligations. The fixture's integer/pointer round trips are
ELF x86 implementation-defined conversions; no converted pointer is dereferenced.
