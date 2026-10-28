# ANDQ-immediate upper-zero width facts (backlog PERF-5)

## What changed

A new shared predicate `andq_imm_narrow_width(t, fam) -> Option<u8>` (in
`relay_and_lea.rs`) proves that `andq $imm, %r64` with a non-negative
sign-extended imm32 leaves the result bounded to 8/16/32 bits, so the upper
32 bits are architecturally zero. The three upper-zero fact machines now
consume it:

1. `relay_and_lea::s_write_proves_upper_zero` (64-bit relay legality);
2. `narrow_copy_fold::update_upper32_zero` (the 32-bit copy that re-proves
   the invariant becomes redundant and is folded);
3. `redundant_ext::update_facts` (byte/16-bit zero-extension facts too, so a
   following `movzbl`/`movzwl` of the masked value is deleted).

Masks that encode as negative imm32 (`$0x80000000`+, which GAS sign-extends
to an all-ones high mask), register/memory-source ANDs, wrong-destination
ANDs and `andl` all return `None`: no fact is established and the existing
extensions are retained. Rationale and the `x & m <= m` result-width bound
are documented at the predicate.

This closes archive item **PERF-5** ("no redundant-extension elimination /
`nonzero_bits`"), whose reproducer is the `andq $127` + `movslq`/`movl`
residue in the reverse-varint digit loop.

## Blast radius (assembly A/B, feature forced off via a temporary env gate)

Of all 78 benchmark translation units, exactly **5** change; every diff is a
pure removal of a redundant narrow copy (one also enables a LEA fold):

| TU | removed |
| --- | --- |
| `double_reduction` | `movl %r15d, %r15d` self-copy inside the 1,048,576-trip LCG init loop |
| `fir_filter` | `movzwl %ax,%edi; subl` folded to `leal -512(%rax),%edi` |
| `linux_list_sort` | `movq`+`movl` pair around `andq $511` (2 insns) |
| `reg_off_window` | four `movl %eax,%rNNd` copies; LEAs read the masked register directly |
| `switch_dispatch` | `movl %edi,%esi` before a sign-extending indexed load |

## Correctness

- Full differential corpus vs GCC 16.2 (`run_regression.py`): **1195
  passed, 0 failed** (63 host m32 skip-runs where the host GCC fails
  identically; 2 pre-existing skip-compares). Summary in
  `corpus-differential.json`, full log in `corpus-differential.log.gz`.
- Full lib suite: 6765 passed / 0 failed; asmdiff semantic gate: 28/28.
- Mutation verification: with the predicate forced off, the five positive
  new gates fail while the two negative/soundness gates still pass.

## Runtime (wall-clock, paired, CPU pinned, 5 reps × 56 kernels)

Reference-normalized geometric means (lower = faster than the references):

| run | LCCC/GCC geo | LCCC/best-reference geo |
| --- | --- | --- |
| baseline (`baseline-56.json`) | 0.83354 | 0.87196 |
| with fix (`after-56.json`) | **0.82899** | **0.86361** |

Per-kernel within-run normalized change: geo **−1.09 %** vs best
reference; 29 kernels improve, 7 flat, 20 move within the run-to-run noise
band. Every apparent raw-median "regression" is on one of the 73 TUs whose
assembly is byte-identical between the two compilers, i.e. host noise, not
a codegen effect.

Focused paired A/B (same binary, feature on vs forced off;
`paired-ab-20reps.json`, `paired-ab-40reps.json`), the three affected
benchmark kernels:

| kernel | 20-rep effect | 40-rep effect |
| --- | --- | --- |
| `switch_dispatch` | −1.22 % | −4.97 % |
| `double_reduction` | +7.48 % (noise; min on < min off) | −0.11 % |
| `linux_list_sort` | +0.87 % (noise) | −0.15 % (best-case min −6.8 %) |

No affected kernel regresses once reps are raised; the change strictly
removes rename/issue uops from hot paths and never lengthens a dependency
chain. Static counts are reported alongside wall time per the backlog's
metric-trap rule; Callgrind was not used for any runtime claim.
