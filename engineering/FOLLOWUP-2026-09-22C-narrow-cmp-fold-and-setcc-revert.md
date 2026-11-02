# Follow-up: the narrow zext→immediate-compare fold, a false-ISA-law revert, and the flag-law guard that caught it

Date: 2026-09-22 (session 61, continued)
Base: `c7917656` (`ms178/lccc` main, post-#591; carries our S05 as `0fbe7ef2`
and the 73a2ab1e LEA-hoist/RORX work, red-teamed earlier this session).
Head: `5e97d517` — S06, snapshot `ms178-1.patch` 28876 B sha `4b33a998…`,
APPLIES-CLEAN (ledger row 6).

---

## 1. Where the session picked up

Session 61 (first half) landed the i686 narrow-compare peephole work and left
two debts: the `leak()` in the source parser, and an unproven corner of the
design. This half closed both — and the guard test written to prove the fold
correct instead **caught an unrelated live miscompile**, which turned out to
be the more valuable event of the day.

## 2. De-leaking the source parser

`parse_narrow_zext_src` originally returned `NarrowSrc::Reg(&'static str)`
built with `src_part.to_string().leak()` — a per-call leak (the house forbids
ungodlike resource behaviour, and `leak()` in a peephole hot path is exactly
that). Rewritten to `NarrowSrc::Verbatim(String)`: the verbatim-source cases
(narrow registers, base-only indirect operands) own their text; the
frame-slot case stays `NarrowSrc::Slot(i32, &'static str)` because the
displacement is *re-emitted canonically* (`format!("{}({})", disp, base)`) —
the line's ESP-slot-biased text must never leak into the compare operand.
`cargo fmt` clean, 3143 unit tests green (net −3: the three tests of the
reverted pass below were removed with it).

## 3. The setCC self-zext deletion: wrong premise, full revert

The first half of the session extended `delete_setcc_self_zext` to the
`movsbl` twin on the stated "ISA law" that a low-byte write zeroes bits
8..31 of the containing register. **That law is false on x86.** A `setCC
%al` is a partial-register *merge*: it writes the byte and leaves bits 8..31
of `%eax` exactly as they were. (The zero-extension rule I confused it with
is x86-64's "32-bit writes zero-extend into the 64-bit register" — a
different width, a different direction.) The `movzbl %al,%eax` after a
setCC is **load-bearing**, and the two pins that asserted it survives
(`setcc_fuse_keeps_window_for_live_bool` and the setbe test) were *right*
before I "improved" them.

How it was caught, because that chain is the reusable lesson:

1. The new freestanding guard `tests/regression/i686_narrow_cmp_fold.c`
   (flag-law edge sweep, exit-status checksum, regparm-free, multilib) ran
   red: gcc rc=0, lccc rc=1.
2. `-S` outputs on/off of both peephole switches were byte-identical —
   **`-S` skips `peephole_optimize`**, so that diff proves nothing (written
   down as a measurement trap: text-level switch A/B must go through the
   object path, e.g. `peephole_optimize` unit tests or assembled objects).
3. The guard decomposed per-helper: rows through the *narrow-compare* fold
   (slot, register, word shapes) matched gcc exactly — the fold is innocent.
   Divergence started exactly at the `sete`/`setne` + relay shape.
4. A 256-value sweep (`00:1 80:6 FF:2` correct vs `00:7 80:6 FF:6` broken,
   i.e. *every* byte wrong, both select arms always taken) pinned it: with
   the `movzbl` deleted, `sete %al` leaves garbage in bits 8..31 of `%eax`,
   the full-width `movl %eax,%ecx`/`%esi` relays propagate it, and every
   downstream `testl %ecx,%ecx; jne` mispredicts the branch — the selects
   all take the "true" arm.

Reverted completely: pass, wiring, kill switch `CCC_NO_SETCC_ZEXT_DEL`,
and its three unit tests deleted. The two pins restored to their original
assertions **with** a justification comment documenting the partial-register
merge law and the failed attempt, so the next reader cannot re-derive it.
The surviving `setcc` pin also re-checks the movzbl presence explicitly.

Nothing else consumed the deleted pass: `fold_narrow_load_imm_compare` never
depended on it (its unit tests exercise the fold with the movzbl present),
and the boot census below is measured after the revert.

## 4. The narrow zext→immediate-compare fold (what shipped in S06)

`fold_narrow_load_imm_compare`, kill switch `CCC_NO_NARROW_CMP_FOLD`:

    movzbl SRC,%r; cmpl $I,%r; jcc   →  cmpb $I,SRC; jcc    I ∈ [0,127]
    movzwl SRC,%r; cmpl $I,%r; jcc   →  cmpw $I,SRC; jcc    I ∈ [0,32767]
    movzwl SRC,%r; testl %r,%r; je/jne → cmpb $0,SRC; je/jne
    movzwl K(%esp),%r; cmpw $I,%r    →  cmpw $I,K(%esp)

Flag-law proof (unit-tested at the exact wrap rows): for immediates inside
the window, every condition code of the narrow compare of a zero-extended
value equals the 32-bit compare — the wrap row (byte ≥ 128 / word ≥ 32768)
sets SF and OF *together* on the narrow view, which is what makes `jl/jg`
decode to the unsigned order; `testl → cmpb $0` is restricted to ZF-only
readers (`je/jne/sete/setne`) because SF diverges. Refusals, each pinned:
sign-extending sources (`movsbl`/`movswl` — the value is negative),
out-of-window immediates, signed consumers of `testl`, live consumers after
the compare (adjacency = nops-only window), indexed/scaled operands,
opcode mismatches, `%esp`/`%ebp` destinations.

Source shapes, generalised across the session: base-only frame slots →
narrow registers (`%dl`,`%dx`) → base-only indirect operands (`2(%edi)` —
pointer loads; the compare re-reads the byte-identical address expression,
fault-equivalence argued in the parser comment). The parser is the narrow
twin of `parse_load_from_ebp` (which deliberately only matches full-width
moves, so narrow extends classify as `Other` — that is why the old sibling
dispatch never saw them, and why `testl` needs text-dispatch *inside* the
`Cmp` arm: `testl` classifies as `Cmp`, making a sibling `else if` dead).

### Measured effect (boot corpus, 21 `arch/x86/boot` TUs)

| stage | lccc insn | lccc bytes | gcc insn | gcc bytes |
|---|---|---|---|---|
| pre (session-61 start) | 5477 | 24460 | 3461 | 14270 |
| + slot folds | 5468 | 24424 | | |
| + register sources | 5457 | 24356 | | |
| + indirect sources | **5451** | **24332** | 3461 | 14270 |

Mnemonic closers (lccc vs gcc): `movzwl` 167/49, `movzbl` 134/50,
`cmpl` 238/150, `cmpb` 39/78, `cmpw` 25/35. The gap is now dominated by
structural levers, not this peephole: store-to-frame 515 vs 46 (RA quality),
loop accumulators resident in slots (cpucheck: 515-vs-46 class), and
reg-vs-reg narrow compares (`video-mode`: 28 residual `movzwl`).

### The guard test (shipped)

`tests/regression/i686_narrow_cmp_fold.c` + `.flags` (`-m32 -Os -mregparm=3
-fno-pic`): every branch flavour (signed/unsigned/eq) driven across the
byte and word wrap boundaries on slot, register, and pointer sources, plus
the bool-materialisation shape; checksum verified against `EXPECT_A`
(derived from gcc -O0/-O2/-Os agreement; `-DPRINT_REF` re-derives). It is
deliberately freestanding so it links under the boot regime. This is the
test that caught §3 — its continued presence is the insurance that the
partial-register-merge lesson can never regress silently.

## 5. Validation

- `cargo test --lib`: **3143/0** (fastbuild).
- 256-value byte-map sweep vs gcc: **0 divergences** (post-revert).
- Guard test: gcc rc=0, lccc rc=0 (was rc=1 pre-revert).
- Boot census/oracle: 5451 insn / 24332 B (above).
- `ci_local.sh --fast`: **59 passed, 0 failed, 3 skipped — ALL GATES GREEN**.
- Full CachyMod 6.18.52 rebuild by lccc/lccc-ld: SUCCESS; vmlinux
  `.text` 11894550, boot gate `_end = 28944` (paired, 3824 headroom).
  Paired gate A/B (same tree state, boot TUs forced): fold OFF
  `_end = 29024` → fold ON **28944 = −80 B** on the setup sector; the
  fold's corpus-level effect (−128 B over 21 TUs) does not fully reach the
  gate because the gate measures a different (compressed+setup) blend.
- `qemu_boot_test.sh`: **16/16 PASS, zero WARNING/Call Trace/ACPI noise**
  on the serial log (bzImage sha `9c3fe192…`).
- Godbolt oracle (`scripts/godbolt.py compare`, revisions pinned:
  gcc 16.2, clang 23.1.0, icx latest, icc 2021.10.0) on the guard test
  itself, i686 `-Os -fno-pic`: **lccc 90** instructions vs **gcc 128**,
  **clang 161**, **icx 161**, **icc 198** — the first corpus in the
  campaign where lccc beats every competitor outright (−30% vs gcc,
  −44% vs clang/icx, −55% vs icc). Artifacts in
  `artifacts/godbolt-s06/`.

## 6. Still open, in evidence order

1. **i686 RA slot-residence** (515 store-to-frame vs gcc 46): the dominant
   remaining lever; cpucheck's loop accumulator is the canonical repro.
2. **Reg-vs-reg narrow compares** (`movzwl (%edi),%edx; movzwl
   %bx,%eax; cmpl %eax,%edx` → `cmpw (%edi),%bx` after allocation-width
   reasoning): 28 sites in video-mode alone.
3. **andl-mask zext gap** (`movzwl 60(%esp),%eax; andl $-32769,%eax;
   movzwl %ax,%eax`): the mask kills the high bit, but the fold needs a
   mask-aware value-range argument before it may widen.
4. Shift-bit-test IV placement (cpucheck: `movl $1,%eax; movl %ebx,%ecx;
   shll %cl,%eax` vs gcc's IV-in-%ecx).
5. The gdbstub `free_large_kmalloc` discriminator (carried from S02).

<!-- src: engineering/FOLLOWUP-2026-09-22C-narrow-cmp-fold-and-setcc-revert.md -->
