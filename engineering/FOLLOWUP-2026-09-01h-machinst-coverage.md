# Follow-up: measuring MachInst, then closing the largest hole

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `e836c0ab`
Snapshots: `S29`–`S31`

---

## 1. The question nobody could answer

"How much of codegen actually flows through MachInst?" was **unmeasurable**.
`lower_instruction_typed` returns a bool; on `false` the instruction silently
falls back to direct text emission. A shrinking coverage fraction — or a whole
instruction class nobody noticed was excluded — looks exactly like everything
being fine.

`CCC_ISEL_STATS=1` now prints a per-run census: how many instructions were
lowered, and a **ranked** breakdown of what was rejected. Measured across 200
corpus files at `-O2`:

```
MachInst coverage: 1165/2162 = 53.9%
    293  GlobalAddr        (13.6%)
    252  ParamRef          (11.7%)
    213  Call               (9.9%)
     72  Store(float)
     64  Store(other)
     46  Memcpy
```

Nearly **half of all instructions bypassed the typed, tested,
register-allocated representation.** The census is what turned "make MachInst
better" into a ranked worklist.

## 2. Largest hole: `GlobalAddr`, 13.6%

It bailed with an explicit surrender:

> *"GlobalAddr needs leaq symbol(%rip) which isn't directly expressible in
> MachInst Mov (would produce movq, not leaq). Handled by the default codegen
> path for now."*

The comment is correct and the conclusion was wrong: the representation was
missing a variant, so add one. `MachInst::LeaSym { sym, dst }` emits
`leaq sym(%rip), %dst`.

The distinction matters and is not cosmetic: `Mov` with a `RipRel` source means
**load from** the symbol; `GlobalAddr` wants the **address itself**. Conflating
them is a silent miscompile that assembles perfectly, which is why the new
variant is covered by an **execution** test — a probe links it against a real
global initialised to a sentinel and asserts the returned value equals the
symbol's address. An emitter using `movq` returns the contents and fails at
runtime, while remaining invisible to any check that only asks whether the text
assembles.

**Coverage 53.9% → 67.4%**, 561/561 passing.

## 3. Second hole: `ParamRef`, 11.7% — attempted, reverted, explained

`ParamRef` looked like a free win: parameters are materialised by the prologue,
so a `ParamRef` in the instruction stream appears to emit nothing, exactly like
`Alloca` (which isel already accepts).

It fails four corpus tests, and *which* four is the whole story:
`x86_fpo_stack_params_many_args`, `nested_nonlocal_goto_callee_saved`,
`pgo_branchy`, `value_profiling` — every one parameter-related. A parameter is
only already-in-place when it **arrived in a register and kept that home**.
Stack-passed arguments and nested-function frames still need the text path, and
the buffer flush that `false` triggers is what orders that against surrounding
MachInst code.

Reverted, with the reasoning recorded **in the code** so the next attempt models
the incoming-argument location rather than repeating the assumption.

## 4. Third: narrow stores

`I8/U8/I16/U16` stores were refused outright. Nothing about them is
inexpressible — `movb %al, (%rcx)` and `movw %ax, off(%rbp)` are ordinary
`MachInst::Mov` at `OpSize::S8`/`S16`, and the emitter's size tables have
always handled both. The guard only split contiguous MachInst runs and pushed
**byte traffic — the dominant operation in the gzip / zlib-ng / expat
workloads** — onto the untyped path.

Net **−3 lines**. `tests/regression/machinst_narrow_store.c` covers the three
addressing forms the lowering distinguishes (alloca slot, pointer already in a
register, pointer that must be staged) across signed and unsigned 8- and
16-bit types, register and immediate sources, and the `r8`..`r15` registers
whose byte names are spelled differently from the legacy set.

## 5. MachInst test suite: now five layers, 34 tests

| layer | proves |
|---|---|
| table integrity | the four register-name tables agree and are injective |
| type mapping | `OpSize` classifies every `IrType`; suffixes match |
| golden emission | each variant/operand shape emits the expected text |
| assembler differential (GAS 2.47) | 4000 randomized instructions are **valid** |
| **execution** | they are the **intended** instructions |

Only the last catches an AT&T operand-order bug: `subq %rax, %rbx` means
`rbx -= rax`, and a swapped non-commutative operand assembles perfectly.

---

## 6. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1474 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=562 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3366 configs |
| MachInst 5-layer suite | 34 tests green |
| `bench_kernels.py` | `memchr` **1.39x vs GCC**; geomean 0.725x |

---

## 7. To do next, in priority order

1. **`ParamRef` (11.7%) — properly.** Model the incoming-argument location
   (register home vs stack slot vs nested-frame access) instead of asserting
   there is nothing to emit. §3 has the four tests that define correctness.
2. **`Call` (9.9%).** Needs SysV ABI modelling in MachInst: argument
   registers, caller-saved clobbers, return register, stack alignment. The
   largest remaining single class.
3. **If-convert the pure SESE test region** — still the biggest *performance*
   item. `namechars` (0.51x) emits an eleven-branch chain per byte. The chain
   is `if_convert` → `range_fold` → `set_membership` and it is starved at the
   first link. A previous attempt is documented in
   `FOLLOWUP-2026-09-01g`: split the critical edge on the last member first,
   or the normalization creates two phi incomings for one predecessor.
4. **Vectorisation breadth**; **encoding-level differential**;
   **def-dominates-use in the IR verifier**; **linker oracle**.

---

## 8. Notes for whoever picks this up

- **Measure the layer before improving it.** "Make MachInst supreme" is not
  actionable; "53.9% coverage, and `GlobalAddr` is 13.6% of the gap" is. The
  census took one afternoon and turned the whole problem into a ranked list.
- **A surrender comment is a design gap, not a fact.** `GlobalAddr` bailed
  because the representation lacked a variant. Adding it took a dozen lines.
- **The plausible free win is where the bodies are.** `ParamRef` looked
  identical to `Alloca` and broke four tests. Every one named a parameter
  edge case, which is the diagnosis handed over for free.
