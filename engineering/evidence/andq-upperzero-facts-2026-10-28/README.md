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

Negative signed imm32 masks, register/memory-source ANDs, wrong-destination
ANDs and `andl` all return `None`: no fact is established and the existing
extensions are retained. GAS 2.47 rejects positive `$2147483648` and
`$0xffffffff` for `andq`; the encodable sign-extension-boundary spellings are
`$-2147483648` and `$-1`. The negative tests now use these valid spellings.
Rationale and the `x & m <= m` result-width bound are documented at the
predicate. The byte-fact cutoff is conservatively `0x7f`, not the mathematically
possible `0xff`; the wider cutoff has not been justified by workload evidence.

This addresses the recorded **PERF-5** reproducer: the `andq $127` +
`movslq`/`movl` residue in the reverse-varint digit loop. It does not by
itself implement general `nonzero_bits` tracking or establish a runtime win.

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

The per-kernel within-run normalized change is geo −1.09% vs best reference.
This compares two separate five-repetition runs, not paired per-round deltas;
the hypervisor and scheduler can affect it. The reported assembly screen says
73 TUs were byte-identical, but that does not establish that every timing
movement is noise or prove a causal speedup on the five changed TUs.

Focused paired A/B (`paired-ab-20reps.json`, `paired-ab-40reps.json`)
compared generated programs using feature-on and forced-off compiler binaries.
The earlier table divided independent wall-time medians; that is not the median
of the paired ratios. The recorded 40-round **on/off paired ratios** and 95%
bootstrap intervals are:

| kernel | median paired ratio | 95% interval |
| --- | ---: | ---: |
| `switch_dispatch` | 1.00248 | [0.99164, 1.01852] |
| `double_reduction` | 0.99941 | [0.98908, 1.00737] |
| `linux_list_sort` | 1.00185 | [0.99665, 1.01062] |

All three intervals include 1. The samples do not establish either a speedup
or the absence of a modest regression, and static instruction removal does not
by itself prove a uop or dependency-chain improvement. These measurements are
from a virtualized host, not the requested target hardware; PMU data was not
available. Callgrind was not used for the wall-time claims.
