# PR #663 — CI repair, red-team audit, and follow-up work

**Base:** `ms178/lccc` `main` @ `93f2a43b` · **Date:** 2026-09-28

> **Read this first.** PR #663 reported seven CI failures. All three root
> causes were found, fixed, and validated in the exact CI container; all seven
> are fixed upstream. That audit is **complete**.
>
> What this branch adds on top of the current `main` is one thing, and it is
> not a re-do of the audit: **§2 is a real, reproducible defect that is still
> present in `main` today** — a local `R_X86_64_GOT64` slot is keyed on the
> section symbol, so three locals at `.data + 0`, `+ 0x10` and `+ 0x20` collapse
> onto one slot and the program reads `slot + A`. On binutils ≤ 2.43 **GNU ld
> has the same defect**; lccc-ld after this change answers `1 1 1` where GNU
> ld answers `1 0 0`. §2.1 records how the earlier version of this document
> concluded the opposite, and why that conclusion was a testing artifact — a
> single reference cannot test a keying defect.
>
> Also here: §3 (the audit record — durable knowledge about which of the seven
> findings were *wrong*, so nobody re-introduces them), §4–§5 (the one harness
> gap and the filed defects), §6–§7 (technical debt and the recommended order
> of next work), and the tests and matrix that make §2 non-regressable.
>
> The *reasoning* is kept in full, because the value was never only the diff.
> Several findings here reversed on measurement, and a reader who trusts the
> original reports will otherwise re-introduce all of them.

---

## 1. What CI reported, and what each failure was

| Reported failure | Real cause | Now on `main`? |
|---|---|---|
| `prop_note_and_or_merge` | linker-created objects vetoed `FEATURE_1_AND` | ✅ yes (PR #665) |
| `prop_note_or_survives_veto` | same | ✅ yes |
| `prop_note_z_ibt_rescues_veto` | same | ✅ yes |
| `prop_note_z_ibt_joined_form` | same | ✅ yes |
| `prop_note_isa_level_created` | same | ✅ yes |
| `prop_note_z_lam_u48_sets_both_bits` | same | ✅ yes |
| `ie_to_le_local` | harness assumed a CET-free prologue | ✅ yes |

Seven reports, **two** root causes.

### 1.1 The property-note defect (6 failures, one bug)

`.note.gnu.property` entries merge by class: an `AND`-class type
(`FEATURE_1_AND` = `0xC0000002`) is vetoed by *any* input that does not carry
it, and an input with **no note at all** is indistinguishable from one
carrying a veto.

The linker funnels every `SHF_MERGE` input through one linker-created object,
`<string-merge>`. It has no note, so it vetoed `FEATURE_1_AND` for links whose
*real* inputs all agreed. The same applied to `<property>`, `<build-id>` and
`<script-dynamic>`.

"Synthetic" was tracked as a `FxHashSet<usize>` threaded from the caller, so
the first synthetic object appended by a phase that did not know about the set
— string-merge, the only one — was invisible to the exclusion.

**Why it was Ubuntu-only.** On a host whose CRT objects carry no note the veto
is a no-op. It becomes visible only where the real inputs *do* carry
`FEATURE_1_AND` and agree — i.e. exactly the CI image. A defect invisible on
the author's machine and fatal in CI is what a differential oracle is for.

**The fix (mine, and `main`'s, which differ).** I made it a field,
`Elf64Object::synthetic`; `Elf64Object` derives no `Default`, so every
construction site is forced to name it and a missing registration is
unrepresentable. `main` instead passes a prefix count, `real_inputs`, captured
once at the end of input loading, with a bounds check
(`real_inputs > objects.len()` → internal error) and a comment naming every
synthetic producer. I verified the boundary is actually placed after the
string-merge pool is appended (`link.rs:506` vs `link.rs:696`).

Both are correct. `main`'s is arguably the better of the two: one greppable
point, a *checked* invariant instead of an unchecked per-object flag, and no
change to a struct shared by four backends. Its weaker assumption — every
synthetic object is appended rather than inserted — is currently true at all
four call sites. Not re-litigated here.

**Verified.** Removing the exclusion fails 7 of 8 property tests, each
reporting the AND type as dropped (`lccc={…: 5} bfd={…: 3, …: 5}`).
`main` additionally carries `prop_note_synthetic_objects_do_not_veto` and the
unit test `objects_past_real_inputs_neither_veto_nor_carry`.

### 1.2 `ie_to_le_local` — the harness was wrong

The test decoded bytes at hard-coded offsets (0 and 4) for the
Initial-Exec→Local-Exec rewrite. Ubuntu's gcc emits a four-byte `endbr64` in
front of the access, so the rewrite lands at offset 4. On Debian
(no `-fcf-protection` by default) the assumption held.

The **emitted instruction was correct**. Two mutations confirm the test
observes the code it claims to:

| Mutation | Result |
|---|---|
| `tpoff + a + 4` → `tpoff + a` | test fails, `22 0 11 11` |
| `rex_r_to_b` → `rex \| 0x01` | test SIGSEGVs (rc −11) |

---

## 2. A second defect found while auditing: the GOT64 section addend

Not in the CI report — found auditing the relaxation code, then confirmed
against GNU ld, then *survived a second, harder round of testing that the
first round got wrong.*

`R_X86_64_GOT64` is an **absolute** 64-bit reference: the GOT slot holds
`S + A`, and the relocation field holds that slot's plain offset from
`_GLOBAL_OFFSET_TABLE_`. Every other GOT form's addend is a *displacement
bias* and its slot holds plain `S`.

Folding the addend into the field instead gives:

| input | slot value | field | reads |
|---|---|---|---|
| `R_X86_64_GOT64 lvar + 0` (gas ≥ 2.44) | `&lvar` | slot offset | correct |
| `R_X86_64_GOT64 .data + 8` (gas ≤ 2.43) | `&.data` | slot offset **+ 8** | 8 bytes past the slot |

`$lvar@GOT` on a local with no symbol-table entry of its own is emitted as a
**section symbol with the offset folded into the addend** by binutils ≤ 2.43,
and as the named symbol with addend 0 by ≥ 2.44 — two spellings of one
address. The addend is load-bearing, not a bias.

### 2.1 The correction: a single reference cannot show this

The first version of this section concluded that `main` was already correct
and that the four spellings `lvar+0`, `.data+0`, `.data+8`, `.data+0x10` all
"agree at runtime" against GNU ld. **That conclusion was wrong**, and the
reason is worth recording because it is a general trap.

Those four were four *separate* links, each with **one** GOT64 reference. A
single reference produces a single slot, and a single slot cannot collapse
onto itself. The defect only appears when **one section symbol carries several
distinct addends** — because a section symbol is *shared by every local in its
section*, so `.data+0`, `.data+0x10` and `.data+0x20` are three addresses
behind one symbol. Keyed on `(object, symbol)`, they map to one slot, and the
field then points `+ A` past a slot that already holds only the section base.

Measured, on one fixture with **three** locals in `.data` each loaded through
`@GOT` and compared against the address it names:

| assembler | relocations emitted | old lccc | GNU ld | the answer |
|---|---|---:|---:|---:|
| host gas 2.44 | `lvar0 + 0`, `lvar1 + 0`, `lvar2 + 0` | `1 1 1` | `1 1 1` | `1 1 1` |
| chroot gas 2.42 | `.data + 0`, `.data + 0x10`, `.data + 0x20` | **`1 0 0`** | **`1 0 0`** | `1 1 1` |

So the original `main` **had** the defect, and GNU ld 2.42 has it too: both
key the local GOT slot on the section symbol. The old note's claim that the
defect was already fixed was a testing artifact.

### 2.2 The fix

`got_slot_addr_addend(rela_type, addend)` returns the **address** addend for
`R_X86_64_GOT64` and `0` for every other GOT type. That is what keeps "what
the slot holds" and "what the field says" independent, instead of one
`addend` parameter doing both jobs. `LocalSlots` is keyed
`(object, symbol, address addend)`; the TLS slot sets, which carry no address
addend, are keyed `(object, symbol, 0)`. lccc-ld now answers `1 1 1` on
gas 2.42, in both `-no-pie` and `-pie`, where GNU ld still answers `1 0 0`.

**How the test decides.** `got64_spelling_matrix` assembles one fixture with
*every* assembler on `LCCC_ASSEMBLERS` plus the default, reports the actual
GOT relocations, and skips variants that cannot assemble. It asserts **the
specification** — each `check` compares a GOT-loaded value against the address
it names, so the expected result is `1 1 1` on any assembler — and it reports
GNU's answer without requiring it. An earlier version of that test asserted
`lccc == GNU`; combined with a fixture whose C side declared the
assembly-defined `check0/1/2` as `static` (so every call bound to an
*undefined local function*), it printed `0 0 0` on both linkers and passed
**vacuously**. The fixture now declares them `extern`.

Three lessons, each of which generalises past this bug:

1. **Two oracles that are wrong in the same way agree perfectly.** The
   specification has to be asserted directly; a reference linker is evidence,
   not authority — and here it was demonstrably wrong.
2. **A single reference cannot expose a keying defect.** Prove the key
   cardinality, not just the encoding.
3. **The pinned oracle can be blind.** CI assembles with **gas 2.47**, which
   emits the `lvar + 0` spelling, so a suite run under CI's own `as` cannot
   observe this class of defect at all. The matrix exists precisely to
   assemble the same fixture with the *oldest* available assembler too. The
   two APX/MOVRS tests likewise only run when a supporting `as` is on `PATH`.

---

## 3. Red-team audit: the findings that did not survive scrutiny

Seven items were raised for repair. Three were reported as defects; testing
reversed all three. Recording the reversals matters as much as the fixes — they
are the difference between a fix and a regression.

### F1 — "the CIE personality `PC32` guard is wrong" → **NOT A DEFECT**

gcc ≥ 11 never emits a `PC32` against `__gxx_personality_v0` directly. It
routes through a local `DW.ref` symbol:

```
.data.rel.local.DW.ref.__gxx_personality_v0:  R_X86_64_64     __gxx_personality_v0 + 0
.eh_frame (CIE aug 'zPLR', encoding 0x9b):    R_X86_64_PC32   DW.ref.__gxx_personality_v0 + 0
```

`DW.ref` is `STB_LOCAL`, so neither `abs_target` nor `binds_outside` is true
and `pc_rel_to_loader` is never reached. The guard is correct for every
compiler we support. It remains a deliberate divergence for *hand-written* CIE
assembly against a personality symbol directly; documented in the code.

### F2 — "GOTTPOFF→LE silently miscompiles" → **NOT A DEFECT as reported**

Reproduced in `scratch/tls/`. For a **local** `tv2`, GNU ld and lccc-ld both
refuse, and lccc-ld names the exact cause:

```
R_X86_64_GOTTPOFF against 'tv2' at offset 0x2 is not a REX.W movq/addq
sym@gottpoff(%rip), %reg and the symbol has no GOT slot:
cannot convert Initial-Exec to Local-Exec
```

The suggested fix — give a local symbol a real GOT slot — would make lccc-ld
*diverge* from the reference. Rejected.

**The one genuine divergence, and its scope.** For a **global** `tv2` in a
dynamically linked executable, GNU refuses the transition and lccc-ld performs
it. Investigated in full:

* The GOT slot lccc-ld allocates is **correct** — it holds the true thread
  pointer offset (`-4` for the fixture, matching `.tdata` layout exactly).
* The faulting binary is the faulting **program**, not the link: the fixture is
  `movl tv2@gottpoff(%rip),%eax` — a **32-bit** load of a 64-bit signed thread
  offset, which zero-extends `-4` to `0xfffffffc`. No compiler emits this.
  GNU refuses the shape; lccc-ld links it and the program dies.
* With `movq` (the form any compiler emits), GNU still refuses, and lccc-ld's
  static `tpoff` is correct for a symbol defined in the executable — the main
  executable's definitions cannot be preempted, and its TLS block offset is
  known at link time.

So: real, but **divergence-only**, reachable only from hand-written assembly,
in the more permissive direction.

### F3 — "absolute `R_X86_64_64` into read-only `.so` storage is silently wrong" → **NOT A DEFECT**

`.quad hook` in `.rodata` of a shared object. The claim was that lccc-ld
resolves the address at link time. Measured: lccc-ld **does** emit
`R_X86_64_64 hook + 0`; the stored `0x1499` is the pre-relocation placeholder,
exactly as GNU stores `0`. The `.data` variant is identical.

The real difference is placement: lccc-ld puts the section in an **`RW` `LOAD`**
covered by `GNU_RELRO` — ld.so writes it, then the page is mprotected —
where GNU keeps it `R` and sets `DT_TEXTREL`, a security downgrade (which is
why modern GNU ld warns and `-z text` makes it fatal). lccc-ld's approach is
correct and strictly safer. Confirmed the library loads and runs.

### F4 — "--defsym resolution is order-dependent" → **NOT REPRODUCED**

`--defsym=x=y` where `y` is defined in a *later* object, plus `--defsym` in
`-shared` and `-r`: lccc-ld matches GNU ld. The suite already covers this
(`defsym_layout_reloc_reference`, `defsym_layout_shared`). A first attempt to
reproduce produced a spurious "multiple definition of `_start`" — that was my
own fixture linking `crt1.o` alongside an object defining `_start`, not an
lccc-ld defect.

### F5 — "the `PLTOFF64` type gate is loose" → **latent, no miscompile found**

`R_X86_64_PLTOFF64` is gated on the *symbol* rather than the relocation.
`create_plt_got` already routes the whole `GOT64` family to a slot, so no
unencodable reference was found. Left alone; filed in §5.

### F6 — "`note_dso_names` records a library `--as-needed` then drops" → **hardening only, no reachable difference**

I made the code report whether a library survived `--as-needed` and gated
`note_dso_names` on it, matching that set's own documented contract ("names
some shared library **of the link** defines"). The set gates the
`--gc-sections` root filter, so the theoretical harm is dead code kept alive.

**I could not construct a case where it matters, and I tried hard.** The only
way a dropped library's name is meaningful is a collision with the
executable's own symbol — and the collision is exactly what stops `--as-needed`
from dropping the library. Measured with the fix disabled and a colliding
`orphan_fn`: both linkers keep `liborphan.so` in `DT_NEEDED` and both collect
`orphan_fn`.

**The regression test for it was written, then deleted.** I proved by mutation
that it *passes on a build with the fix removed*. A test that cannot fail is
worse than no test: it manufactures confidence and would launder an untested
change into a green one. Neither the test nor the change is carried forward.

---

## 4. What this branch adds

`main` still has one gap of the class §3 exposes, and this branch closes it.

### `crossarch_gnu_hash_relro` blamed lccc-i686 for a missing package

It was the last i386-touching test reporting a missing 32-bit libgcc as a
lccc-ld defect. The other seven i386 gates and the C++ probe already ask the
reference toolchain first; this one did not.

The fix probes `gcc -m32 -shared` before running the i686 driver, and reports
SKIP when the reference cannot link — or FAIL when `LCCC_REQUIRE_I386=1`, which
both `ci.yml` and `ci_local.sh` set, because a runner that promised the
multilib and did not deliver it is broken.

**This is not a weakening, and both directions were measured:**

| environment | result |
|---|---|
| host, no `gcc-multilib` | `SKIP`, with the reference toolchain's own error |
| chroot, multilib present, `LCCC_REQUIRE_I386=1` | `PASS` — 3 arch drivers verified, 48 exports each |

---

## 5. Remaining defects, filed

### 5.1 GOTTPOFF against a preemptible global — **CLOSED: not reachable**

**Status was:** open, divergence-only, unreachable from a compiler. Low impact.

**Measured, then closed.** The proposal below was to add a preemptibility
gate to the executable IE→LE relaxation. Building the probe showed the gate
would be **dead code**: the predicate it would guard is already there, under
a different name.

`exec_ie_target_local` returns `!is_dynamic && defined_in.is_some()`. In an
executable a symbol that is *imported* from a shared object is exactly the
preemptible case, and it is `is_dynamic` — so the relaxation is already
declined for precisely the symbols the gate was proposed to catch. Built and
run, `-no-pie` and `-pie` executables plus a PIE against a `.so` that exports
its `__thread` variables with default visibility:

| link | lccc | GNU ld | relocations lccc emitted |
|---|---:|---:|---|
| exe `-no-pie`, defined `tv1`/`tv2` | `ie 33` | `ie 33` | IE→LE, correct |
| exe `-pie`, defined | `ie 33` | `ie 33` | IE→LE, correct |
| PIE vs `.so`, exported TLS | `so-ie 33` | `so-ie 33` | **`R_X86_64_TPOFF64`**, not relaxed |
| the `.so` itself | — | — | `R_X86_64_DTPOFF64` |

The preemptible case keeps the **dynamic** `TPOFF64` and the loader computes
it, byte-for-byte what GNU produces. Adding the gate would have added a
branch that can never take, plus an error message no user can reach — the
"comprehensive-looking" fix that is really a liability.

### 5.2 `R_X86_64_64` against a preemptible global in read-only storage — **CLOSED: not reachable**

**Status was:** open, unreachable from a compiler (compilers emit
`.data.rel.ro`).

**Measured, then closed.** The hazard was real in principle — a write into a
read-only page is a segfault in `ld.so`, not in the program — but lccc-ld
already promotes such a section. Forcing the case: an `R_X86_64_64` against
symbols defined in the same link, in a section stripped to `A` (read-only,
alloc) with `objcopy --set-section-flags`, linked `-pie -fPIE`:

| linker | result | relocations |
|---|---:|---|
| GNU ld | `ro 106` | — |
| lccc-ld | `ro 106` | `R_X86_64_RELATIVE` against the merged definitions |

`exec_*` decides at emission time and the section ends up in RELRO with
dynamic relocations that `ld.so` applies before the pages go read-only. Both
the premise ("unreachable from a compiler") and the mitigation ("promote to
RELRO") hold, so the "checked precondition" proposal would likewise be a
branch that cannot fail.

**What survives from both:** the *reason* they are unreachable is worth
keeping, and it is now written down in the code and here rather than as a
proposal — a future change that widens the RELRO boundary has to recompute
the writability predicate, and the tests that would catch it are the ones in
`tests/linker/run_linker_tests.py`, not a new error path.

### 5.3 `R_X86_64_GOT64` with a non-zero addend on a **global**

**Status:** not a divergence — deliberately left alone.

The slot is name-keyed and holds plain `S`, so a non-zero addend cannot be
represented. GNU ld does not represent it either: measured on ld 2.42,
`$gv+8@GOT` against a defined global yields `movabs $0x0,%rax` — a faulting
image **from the reference linker**. Every form a compiler emits is addend 0.
Recorded in `emit_exec.rs` with the measurement, so a future reader sees that
matching GNU means *not* fixing this.

### 5.4 `PLTOFF64` gated on the symbol, not the relocation (F5)

No miscompile found. Tightening it to the relocation type is small,
low-risk hardening that removes an accidental dependency on the current routing
in `create_plt_got`.

---

## 6. Technical debt

* ~~**Assembler-version blindness in the test suite.**~~ **RESOLVED.**
  `got64_spelling_matrix` (§2.2) now assembles one fixture with *every*
  assembler on `LCCC_ASSEMBLERS` plus the default, so the gas-2.42 spelling is
  covered without pinning a second toolchain globally, and a variant that
  cannot assemble is skipped rather than failing. The general rule is recorded
  in the matrix's docstring: a relocation whose *spelling* changed between
  assembler versions is only testable if the old assembler is on `PATH`.
* **The property merge has two independent notions of "real input".** Both
  read the same boundary today, but they are separate scans. A single
  `fn real_inputs(...)` would remove the chance of them drifting again — the
  exact shape that caused the six original failures.
* **`--gc-sections` roots derived from a flat name set.** `dso_names` is a
  `FxHashSet<String>` consulted once. The comment beside it explains the
  semantics; the *type* does not. A `DsoNames` newtype whose only constructor
  encodes "of the link" would make the F6 class unrepresentable rather than
  merely avoided.
* **Harness reports environment gaps as FAIL.** Several corpus gates and the
  i386 driver test fail on a host without `gcc-multilib`, blaming lccc-ld for a
  missing package. §4 closes the last one; the same "probe the *reference*
  toolchain first" rule should be applied to the corpus gates
  (`nocfi-peephole-parity`, `reassoc-latency`, `i686-integer-isa-parity`,
  `map-i64-two-lane`), which still fail on such a host.

  **Measured, so the fix is chosen on evidence.** Those four gates fail on this
  host *only* because the sandbox has no root, so `libc6-dev-i386` cannot be
  installed and `gcc -m32` cannot find `bits/libc-header-start.h` or `crti.o` —
  every failure line is a preprocessor or `crt` error from the **reference**
  toolchain, none from lccc-ld. Running the identical gate set in the pinned
  `lccc-ci-noble` chroot, which has the i386 runtime, gives
  `SUMMARY: 96 passed, 0 failed, 5 skipped / ALL GATES GREEN` with the linker
  suite at `283 pass, 0 fail, 0 warn, 0 skip`. So the gates are sound and the
  *report* is the bug: an unprobeable environment should be a distinguishable
  `SKIP (no i386 toolchain)`, not a `FAIL` that reads as a product defect.

---

## 7. Recommended next work, in order

1. ~~**Pin an older assembler for a second fixture set**~~ **DONE** — the
   `got64_spelling_matrix` (§2.2), which found the real §2 defect.
1b. **Make an unprobeable environment a `SKIP`, not a `FAIL`** (§6, last item).
   Now the highest-value item: it is the only thing that currently makes a
   green product look red, and it is what cost this branch a full
   re-investigation. Prefer `SKIP (no i386 toolchain)` after a positive probe
   of the *reference* compiler, as `ie_to_le_forms` and `movrs_relocations`
   already do for `as`.
2. ~~**GOTTPOFF preemptibility gate**~~ **CLOSED, not reachable** — §5.1. The
   predicate already exists as `exec_ie_target_local`; the probe shows the
   preemptible case keeps the dynamic `TPOFF64` and matches GNU.
3. ~~**Localise the read-only precondition**~~ **CLOSED, not reachable** —
   §5.2. A read-only section holding `R_X86_64_64` is already promoted to
   RELRO with `R_X86_64_RELATIVE`, and both linkers agree.
4. **`DsoNames` newtype** (§6). Makes the `--as-needed` contract a type.
5. **Unify the "real inputs" iteration** (§6). Cheap, and it is the exact shape
   that caused the original six failures.
6. **Reference-toolchain probes for the corpus gates** (§6). CI already has the
   toolchain, so this cannot hide a real failure there.
7. **The one measured codegen gap: constant-trip loops.** `int_alu` in
   `tests/oracle/programs/` is the only program where lccc is far behind:
   92 instructions against Clang 23's 23. Clang unrolls the fixed-trip-count
   loops and constant-folds the result, so it emits **no loop at all**;
   lccc strength-reduces the division by 10 and by 7 into multiply/shift
   sequences and keeps everything in registers — a good loop body that still
   runs 64 times at runtime. This is the highest-value optimisation the
   three-vendor oracle points at, and it is named here rather than half
   built: a partial unroller that mishandles one guard is worse than none.
   Start with loops whose trip count is a literal and whose body is
   straight-line with no calls, stores or `volatile`, and verify with
   `tools/oracle/godbolt_oracle.py --filter int_alu`.
8. **Widen the oracle's program set.** Eight programs cover the main passes;
   they do not cover vectorisation widths, atomics, or `-march`/feature
   variation. `docs/GODBOLT_ORACLE.md` states exactly what the current
   table does and does not measure, so the next person extends rather than
   re-derives.
 The property fixtures are hand-written
   notes; a C/C++ fixture compiled by the real toolchain would catch decoder
   bugs they cannot, and would have failed on the author's host — which is the
   actual lesson of this PR.

---

## 8. Reproducing

```bash
# Exact CI environment. Expect: 275 pass, 0 fail, 0 warn, 0 skip.
bash scripts/ci_ubuntu_chroot.sh -- env \
  PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH" \
  LCCC_REQUIRE_I386=1 LCCC_RELOCS_TOOL="$HOME/tools/bin/relocs" \
  python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --strict -v

# Local full gate. Expect: ALL GATES GREEN.
bash scripts/ci_ubuntu_chroot.sh -- \
  env RUSTUP_HOME=$HOME/.rustup CARGO_HOME=$HOME/.cargo \
      PATH="$HOME/.cargo/bin:$PATH" \
  bash scripts/ci_local.sh --fast

# GOT64, assembled by binutils <= 2.43 (NOT the pinned 2.47 -- see §2):
bash scripts/ci_ubuntu_chroot.sh -- python3 - <<'PY'
import subprocess, tempfile, os
d = tempfile.mkdtemp(); os.chdir(d)
open("t.s","w").write("""
\t.text
\t.globl g
g:\tleaq\t_GLOBAL_OFFSET_TABLE_(%rip), %rcx
\tmovabsq\t$lvar@GOT, %rax
\tmovq\t(%rcx,%rax), %rax
\tleaq\tlvar(%rip), %rcx
\tcmpq\t%rcx, %rax
\tsete\t%al
\tret
\t.data
\t.p2align 3
pad:\t.quad 0
\t.quad 0
lvar:\t.quad 9
\t.section .note.GNU-stack,"",@progbits
""")
subprocess.run(["as","t.s","-o","t.o"],check=True)
print(subprocess.run(["readelf","-rW","t.o"],capture_output=True,text=True).stdout)
PY
# Expect: R_X86_64_GOT64 .data + 0x10   (the version-sensitive spelling)

# Mutation checks that prove the regressions have teeth:
#   cet.rs      — drop the synthetic skip  -> 7 of 8 propnote tests fail
#   emit_exec.rs — `tpoff + a + 4` -> `tpoff + a` -> ie_to_le_local fails
#   elf.rs      — `rex_r_to_b` -> `rex | 0x01`     -> ie_to_le_local SIGSEGVs
```
