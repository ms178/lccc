# Session 2026-10-05: PR #765 merged with the S20a encoder halves, and the
# verbatim loop rotation re-implemented

Two work streams landed together on top of `ms178/lccc` main `08f4e1a1` (the
merge of PR #765, "fail-closed operand legality, two-oracle parity, and the
fail-open remainder"):

1. **The AArch64 encoder.** #765's fail-closed work and this branch's S20a
   halves were merged into one encoder and then driven to **6817/6817
   agreement with GNU as 2.47 in both directions** — the curated matrix
   (`scripts/aarch64_operand_legality_matrix.py`, `--check`) and our own
   encoder against that same table (zero disagreements) — plus a new
   spelling-sweep probe, 411 spellings, zero unpinned disagreements.
2. **`loop_invert`'s verbatim clone mode** ("fix B"), which had been measured
   at −11.0% on `k_strcmp_signed`, was lost to a harness wipe and
   re-implemented from its surviving specification. It reproduces the original
   numbers exactly (below).

## 1. The merge, and what the matrix caught that #765 had not

The starting point was honest: #765's encoder plus this branch's matrix
disagreed with the table on **40 of 6817 rows** in `--check-lccc` (and the
table itself was re-verified against the pin, `--check` 6817/6817). Each
disagreement is a defect in the merged encoder; the fixes were ported from the
S20a halves, not re-derived, and each is now pinned by the same rows that found
it. The classes, with one example each and GNU as 2.47's own verdict:

| # | class | example | GAS | merged tree before |
|---|---|---|---|---|
| 1 | mnemonic-keyed destination width | `ldrb x9,[x10]` | reject | accepted (`strb`/`ldrh`/`ldrsb`/`ldtrh` likewise: 14 rows) |
| 2 | unscaled/unprivileged widths missing | `ldurb w0,[x0,#8]`, `ldtrsh w0,[x0,#8]` | ok (`38408000`) | rejected — family absent |
| 3 | SP in a transferred-register slot | `ldur sp,[x0,#8]`, `stur wsp,[x0,#8]`, `strh wsp,[x0]` | reject | accepted (Rt=31 read as XZR) |
| 4 | exclusives' status register | `stxr x0,x1,[x2]` | reject | accepted (X0's number in a W field) |
| 5 | exclusive/LSE Rt vs SP | `ldxr sp,[x1]`, `ldsmax x0,x1,[x2]` | reject / ok | accepted / rejected |
| 6 | CAS/SWP width agreement | `cas x0,w1,[x2]`, `swp x0,w1,[x2]` | reject | accepted (one size field for both registers) |
| 7 | unscaled prefetch missing | `prfum pldl1keep,[x0,#-8]` | ok (`00809ff8`) | rejected — mnemonic absent |
| 8 | SYS-form Rt legality | `ic iallu,x0`, `tlbi vmalle1,x0`, `at s1e1r` | reject | first two accepted, third accepted with no register |
| 9 | register offset extend rules | `ldr x0,[x0,w1]`, `ldr x0,[x0,x1,uxtx]`, `ldr x0,[x0,x1]!`, `ldr x0,[x0],x1` | reject | accepted (and the writeback/index silently dropped) |

Two of these were *hiding behind a predicate*: `is_fp_reg` tested the first
letter only, so `sp` read as an S register — which encoded `stur sp,[x0,#8]`
as `stur s31,[x0,#8]` and let `ldur sp,[x0,#8]` slip past the
transferred-register gate as a scalar FP access. The predicate is now exact
(width letter *followed by a register number*).

The NEON side of the merge was the same shape but larger: 30 by-element rows
(`fmul`/`fmulx`/`fmla`/`fmls` across `.h/.s/.d` and scalar spellings),
`addv`/`smaxv` refusing a destination class that disagrees with the source
arrangement (`addv b0,v1.4h` must not assemble) while `fmaxv s0,v1.4s`
(=`6e30f820`) must, `ld1r/ld2r/ld4r` including the `#2`/`#4` post-index forms
and the R-bit-corrected words, `tbl/tbx` list-vs-arrangement rules, and
`shll/shll2`.

Two *parser* defects came out of the same sweep, both of which had made the
assembler quietly accept assembly no architecture defines:

* `v0.16b[3]` and `v0.b[3]` are the same operand to GNU as (measured: the
  words are identical, the index bound is the element's, not the
  arrangement's — `ins v0.8b[15],v1.b[15]` assembles, `ins v0.4h[8],v1.h[0]`
  does not), so the lane parser now accepts the eight canonical arrangement
  spellings and maps them to the element type.
* A *register* writeback is not an addressing mode: `ldr x0,[x0,x1]!` used to
  lose the writeback and `ldr x0,[x0],x1` its index, assembling as plain
  register-offset loads. Both are refused now, and the register-offset
  `extend` rules were measured one by one (`uxtw`/`sxtw` only on a W index,
  `lsl`/`sxtx` only on an X index, `uxtx` not an addressing extend at all).

## 2. The pin is not optional, and a fallback is worse than a skip

While re-running the table check, `--check` without `--as` reported **40
"defects" in rows that the pinned assembler accepts**: the script had picked
up the distro `as` (2.44), which does not know `tco`, `afgdtp0_el1` or
`actlrmask_el1` and disagrees with 1077 rows overall. The table was right; the
oracle was wrong. Consequence, now encoded in `ci_local.sh`: the aarch64 gates
resolve the **pinned** `ensure_gas_247.sh aarch64-linux-gnu` triple by
absolute path, spell it literally at every invocation (so
`scripts/check_ci_gate_parity.py`'s exact-token contracts pin it), and are
**skipped, not failed**, when it is absent — a distro fallback fails loudly on
rows that are not defects, which is worse than no coverage at all.

## 3. Fix B: verbatim duplication for headers that walk memory

A top-test loop whose header walks memory —

```c
while (*a && *a == *b) { ++a; ++b; }
```

— was refused by rule 2 ("loads are rejected outright"), so these loops kept a
taken jump per iteration. The refusal was really rule 3's doing: the header's
load *is* the loop's payload, and a renamed copy in the latch would leave the
body reading the guard's byte forever. `loop_invert` now has two clone modes,
chosen from the header's instruction mix:

* **Pure** — every instruction pure: the existing path, fresh names, rule 3
  enforced.
* **Verbatim** — every instruction pure *or* a non-volatile load/store: the
  copy is the header's own instructions with the same destinations and
  operands, and rule 3 is skipped because the copy re-defines exactly the
  values the body reads.

Volatile accesses, atomics, calls, inline asm and stack allocations are refused
in both modes. The transform is a re-labelling, not a motion: the header's
sequence executes at the same points in the same order, `n+1` times per entry,
whether it sits at the top of the iteration or at the bottom of it, so every
load and store reads and writes the same address the same number of times and
nothing becomes reachable that was not executed before. The duplicate is
appended after phi elimination's edge copies, so the copied test observes the
updated induction variable.

Measured, best-of-5, Callgrind Ir on the program's own object (libc and the
loader excluded), stdout checksum verified identical for both arms:

| kernel | before | after | Δ |
|---|---|---|---|
| `k_strcmp_signed` Ir | 844,951,167 | 751,844,222 | **−11.02%** |
| `k_strcmp_signed` wall | 52.5 ms | 35.9 ms | **−31.6%** |
| `k_memchr` Ir | 82,172,461 | 82,172,461 | 0 |
| `k_adler32` Ir | 49,443,748 | 49,443,748 | 0 |

Assembly of `signed_strcmp_loop`: the `jmp` back to the top is gone; the latch
falls through into a duplicated `movsbl (%rdi),%edx; test %dl,%dl; jne`.

Six tests pin the behaviour, including the two that make the mode safe
(`a_verbatim_copy_lands_after_the_edge_copies`,
`a_header_load_escaping_into_the_body_is_inverted_for_memory_but_not_for_pure_values`)
and the four refusals (volatile load, volatile store, call, stack allocation).
The superseded negative control `a_header_containing_a_load_is_not_inverted` is
removed: its premise was the defect.

## 4. Gates

Three gates were added on top of #765's set, each mirrored in hosted CI with an
exact-token invocation contract (`scripts/check_ci_gate_parity.py`, 139
commands, 17 contracts, `scripts/test_ci_gate_parity.py` 66/66):

* `a64-gas-provision` — installs the pinned aarch64 `as`/`objdump`/`objcopy`
  triple the aarch64 gates take their verdicts from (it was hosted-only
  before, so a clean local box fell back to a distro assembler).
* `aarch64-sysreg-table` — 1619 registers, both directions, against the pinned
  as *and* our encoder, from the pinned binutils `.def`.
* `aarch64-legality-probe` — the spelling sweeper (411 generated spellings,
  both verdicts), whose four pinned divergences are documented: GAS ignores
  arbitrary trailing junk after the five-field raw `s3_0_c1_c0_1` spelling
  (`s3_0_c1_c0_1_xyz` assembles), which is parser leniency rather than an
  architected form, so this assembler refuses it.

## 5. Verification evidence

* `aarch64_operand_legality_matrix.py --check` (pinned GAS 2.47): **6817/6817**.
* `... --check-lccc target/fastbuild/lccc` (our encoder): **6817/6817**.
* `aarch64_sysreg_table.py --check`: **1619/1619** (GAS + our encoder).
* `aarch64_legality_probe_differ.py`: 411 spellings, **0 unpinned
  disagreements**, 4 pinned.
* `cargo test --lib --profile fastbuild`: see the snapshot commit message.
* `cargo fmt --all -- --check` clean; `cargo clippy --profile fastbuild --lib
  -- -D warnings` clean.
* `scripts/ci_local.sh --fast`: see the snapshot commit message.

## 6. Follow-ups

Unchanged from `SESSION_FOLLOWUP_S21_LOOP_ROTATION.md` §6, in priority order,
with the data that motivates each:

1. **Count-bits loop idiom** (`bitops`, lccc/GCC = 1.344): `while (x) { x &= x-1;
   c++; }` stays a five-instruction loop although `popcount` is a single
   instruction on the default target. Needs a loop-level folder with SSA phis.
2. **Register-allocator copies and separate zero-extension in byte loops**
   (`strings`, now 1.149): three instructions per byte remain —
   `mov %r15,%rsi` + `mov %rsi,%r15` around the xor/multiply chain, and
   `Cast(I8→U8)` materialised as `movzbl %r9b,%eax` beside a `movsbl` load
   instead of one zero-extending `movzbl (%rbx),%eax`.
3. **`-O0` and FMA contraction**: the x86-64-v3 baseline contracts
   `u*0.999 + 0.001` even at `-O0`, which is legal but differs from GCC's
   `-O0`. Decide explicitly: document it next to the baseline policy, or gate
   contraction on `-O1`+.
4. **The remaining S20a matrix groups as standing coverage**: the matrix is
   regenerated by `--regenerate` from the curated groups, and the sysreg sweep
   reads the encoder's own table, so a register added to `SYSREGS` becomes a
   row on the next regeneration. Keep that invariant when editing the encoder.
