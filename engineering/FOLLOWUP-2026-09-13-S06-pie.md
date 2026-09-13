# S06 — Position-independent executables, `--hash-style`, and a review of a competing revision

Date: 2026-09-13 · base `8aeaee0` · validation: `ci_local.sh --fast` **31 pass / 0 fail / 3 skip**,
linker suite **178 pass / 0 fail / 1 warn**.

---

## 1. What shipped

| Change | Files |
|---|---|
| Real PIE: `ET_DYN` based at 0, `DF_1_PIE`, rebased `PT_PHDR`/`PT_LOAD`, `R_X86_64_RELATIVE` for every self-referential address | `emit_exec.rs`, `plt_got.rs`, `link.rs`, `args.rs`, `cli.rs`, `bin/lccc_ld.rs` |
| `--hash-style=gnu\|sysv\|both` parsed *and* deduplicated through one shared helper | `args.rs`, `cli.rs`, `bin/lccc_ld.rs` |
| `-pie`/`-no-pie` recorded positionally in the driver (previously dropped by `=> {}`) | `driver/cli.rs` |
| Conservative, *loud* fallback to `ET_EXEC` for the two constructs not yet slidable | `link.rs`, `eh_frame.rs` |
| Tests: 6 PIE cases + `expect_elf_type` in the harness + `known_defect` mechanism | `run_linker_tests.py` |
| `expect_stdout` added to the two IFUNC cases, which previously asserted nothing | `run_linker_tests.py` |

## 2. Measured result

Standard PIE corpus, 11 cases × 5 runs under active ASLR, bfd output as ground truth.
Hard corpus, 15 cases, same method; "real PIE" = correct **and** `e_type == DYN`.

| linker | standard | hard: real PIE | hard: ET_EXEC fallback | hard: **broken** |
|---|---|---|---|---|
| bfd 2.47 | 11/11 | 14 | 0 | 0 |
| mold 2.42.1 | 11/11 | 14 | 0 | 0 |
| wild (git) | 11/11 | 14 | 0 | 0 |
| Agent A | 10/11 | 11 | 0 | **3** |
| **this work** | **11/11** | 11 | 2 | **0 PIE-attributable** |

Agent A's three broken cases: `bigstr`, `ifunc`, `cppthrow`. Mine: `ifunc` only — and that one is a
**pre-existing defect unrelated to PIE** (§5).

## 3. Three classes of self-referential address a PIE must slide

Each was found by measurement, not by reading. All three are needed; any one missing yields a
binary that links cleanly and segfaults under a random load base.

1. **Absolute data relocations onto locally-defined storage.** `const char *msgs[]`, jump tables,
   function-pointer arrays.
2. **GOT slots holding one of our own addresses.** `crt1.o` reaches `main` through
   `R_X86_64_REX_GOTPCRELX`; in an `ET_EXEC` the slot is simply pre-filled, in a PIE it needs a
   `RELATIVE` or the program jumps to the unslid address of `main`. Observed as
   `rip = 0x1560` called from `__libc_start_main`.
3. **GOT slots holding a PLT address.** The applier stores `plt_addr + 16 + pi*16` for a dynamic
   function that has a PLT entry — an address inside our image. This is how an `.eh_frame` FDE
   reaches `__gxx_personality_v0`. Observed as `_Unwind_RaiseException` jumping to `0x1c40`, inside
   `.plt`.

### The string-merge trap

lccc's string merging rewrites a reference into a merged string section to point at a synthetic
`__lccc.strmerge.N` pool symbol with **`shndx == 0`**, carrying the string's offset in the *addend*
(`strmerge.rs:311`). Such a symbol looks undefined while being entirely local. This is what made the
competing implementation segfault on `const char *msgs[]`, and it is invisible in every reference
linker's output because none of them represent it this way.

The fix is a predicate that mirrors `emit_exec::resolve_sym`'s decision structure exactly
(`plt_got::stored_value_is_local`), used by both the data-relocation collector and the GOT-slot
classifier. Divergence between "what the applier stores" and "what we slide" is the single failure
mode here, so the two must be one function.

### Count/emit consistency by construction

`DT_RELASZ` is derived from the length of the same filtered `Vec` that is then written, and the
value is resolved through the same `resolve_sym` the applier uses. With `base_addr == 0` the stored
value *is* the addend. mold and lld instead evaluate "the same predicate twice"; here the two cannot
disagree.

## 4. Review verdict on the competing revision

1522 lines of linker change, **zero test files**. Applying it and running the suite gives
**165 pass / 7 fail** — byte-identical to the pristine base, i.e. it neither regressed nor fixed
anything the suite covers.

**Correct and adopted:** `base_addr = 0` + `ET_DYN` + rebased program headers + `DF_1_PIE`;
`HashStyle` as a `Copy` enum with `wants_gnu()/wants_sysv()`; `parse_hash_style` accepting
separators and `both`; the SysV chain construction; driving hash `DT_*` entries from a `Vec`.
Their GOT-slot slide (class 2 above) was right and I had missed it.

**Rejected, with measurements:**

- **Segfaults on string arrays** (§3). Proven root cause: relocs whose target is a string-merge
  section are silently dropped; `strarr` → `rc=139`, `bigstr` → `rc=139`.
- **`DT_RELASZ` computed by a different predicate than the emit loop.** Latent, not demonstrated
  (`.rela.dyn` ends exactly where `.rela.plt` begins — no slack), so any edit to either pass
  silently corrupts the table.
- **`--threads` parsed into `LinkerArgs.threads` and never read** — all 10 hits are writes. That is
  the defect class the revision claims to fix.
- **Wrong `STB_*` comments on live filter conditions**: `(g.info >> 4) == 0 /*STB_WEAK*/` is
  `STB_LOCAL`; `== 1 /*STB_WEAK?*/` is `STB_GLOBAL`.
- **Dead nested `if`** (`g.defined_in.is_none() && …` containing `if g.defined_in.is_none()`),
  `--hash-style`/`--threads` parsing duplicated ~34 lines ×2 in `args.rs` plus a third copy in
  `bin/lccc_ld.rs`, the whole SysV `.hash` block duplicated across `emit_exec.rs` and
  `emit_shared.rs`, `sysv_nbuckets`'s comment claiming 3 buckets where the code gives 4,
  `dyn_count` re-derived inline, `INSERT` support effectively a no-op, and
  `// For now count it` shipped as reasoning.
- **`R_X86_64_GOT32` filled with `g.value` but given no dynamic reloc under PIE.**

## 5. Newly found pre-existing defects

### P0 — `static` IFUNC is a silent miscompile (predates this work)

```c
static int impl(void){ return 1; }
static int (*rsv(void))(void){ return impl; }
static int fn(void) __attribute__((ifunc("rsv")));
```

bfd prints `1`. lccc prints an address (`4199558`). **`lccc-BASE` (pristine `8aeaee0`) and Agent A's
build both reproduce it**, so it is not a PIE regression. `collect_ifunc_symbols` walks only
`globals`, so a *local* IFUNC never gets an IPLT slot or an `R_X86_64_IRELATIVE` and the call binds
straight to the resolver — the caller receives the implementation's address as the "return value".
Global IFUNCs work (`ifuncg` prints `2` on every linker).

**Why the suite never caught it:** `ifunc_resolver` and `ifunc_static_link` had **no
`expect_stdout`** — they asserted only `rc == 0`, so a garbage return value passed. Both now assert
their value. A new `local_ifunc_known_broken` case carries `known_defect=…` and reports **WARN** so
the defect stays visible in the summary rather than being deleted.

Fixing it needs the applier to route `R_X86_64_PLT32` against a local IFUNC through a new IPLT slot;
that is a scoped piece of work, recorded here rather than rushed.

### Harness gap — no test asserted the ELF type

A PIE test that only checks stdout passes whether the image is `ET_DYN` or a silent `ET_EXEC`
fallback, so losing ASLR was unobservable. `expect_elf_type` now parses `e_type` straight from the
header (not from `readelf` text, whose columns shift when a section name is long).

## 6. Known limitations (deliberate, loud)

`-pie` falls back to a fixed-base `ET_EXEC` with a warning naming the construct when the objects
contain:

- any `STT_GNU_IFUNC` symbol — the `R_X86_64_IRELATIVE` resolver address is not rebased;
- any `.eh_frame` CIE with a `P` (personality) augmentation — the GOT slot the personality pointer
  resolves through is not slid, so the first C++ `throw` faults.

Detection is `eh_frame_has_personality()` (`eh_frame.rs`), a linear scan of CIE augmentation
strings. Losing ASLR is a hardening regression; a segfaulting binary is a correctness one, and
correctness wins. Both are fixable and are the top two items below.

`-static-pie` is still refused outright (needs the `rcrt1.o` self-relocation protocol; the static
emitter produces no `.rela.dyn` at all).

## 7. Not measured this session

**Link throughput was not re-measured.** The 201-object fixture's link command was not preserved
across the workspace wipe (`ldwork/cmd.txt` is empty) and the surviving `f*.c` sources are a partial
fixture with no definitions TU, so they cannot be linked standalone. The earlier S05 numbers stand
as the last measurement; no new claim is made. Structurally the PIE work adds no new pass over the
objects: RELATIVE coordinates are collected inside `create_plt_got`'s existing relocation walk, the
GOT-slot classifier is one pass over `got_entries` (already iterated), and emission is O(n) writes
into a pre-sized buffer.

## 8. Next, in order

1. Slide `R_X86_64_IRELATIVE` resolver addresses, then drop the IFUNC PIE gate.
2. Slide the personality GOT slot, then drop the `.eh_frame` gate — that unlocks real PIE for C++.
3. Fix local IFUNC (§5) and drop `known_defect`.
4. `--hash-style` is parsed and deduplicated but the SysV `.hash` table is **not yet emitted**;
   `LinkerArgs.hash_style` is currently a dead field. Emit it from one helper shared by
   `emit_exec.rs` and `emit_shared.rs`, driven from a `Vec` of `DT_*` pairs.
5. Re-measure link throughput once the fixture is regenerated with a checked-in command line.

---

# Session v3 — linker defect sweep (2026-09-13)

Base `a361f26` (upstream main). Deliverable `/home/user/ms178-1.patch`,
27 files, +4814/−247, applies clean, zero mode churn.

## Verified results

| Check | Result |
|---|---|
| Linker suite | **186 pass / 0 fail / 0 warn / 0 skip** |
| Oracles active | bfd **2.47.20260726**, mold **2.42.1** (`301382e5`), wild-git (`4d969005`) |
| `scripts/ci_local.sh --fast` | **31 passed / 0 failed / ALL GATES GREEN** |
| Hard PIE corpus | lccc **realPIE=14, fallback=0, broken=0** — identical to bfd 2.47, mold 2.42.1, wild-git |
| Standard PIE sweep | 11/11 (5 ASLR runs each) |
| `--hash-style` | gnu/sysv/both correct on **both** `-o exe` and `-shared`: right `DT_*`, right sections, `dlsym` over 60 symbols `OK bad=0`, 0 readelf diagnostics |

## Defects fixed this session

**P0-1 symtab bloat under `--gc-sections`.** The locals filter consulted
`section_map`, which still holds a slot for every dead section (layout assigns
before collection decides reachability). 61-object `-ffunction-sections` link:
2341 of 2551 entries were collected-section locals at address 0. Now:

| linker | file | .symtab | nsyms | `st_value==0 && st_size!=0` |
|---|---|---|---|---|
| bfd 2.47 | 22 048 | 6 192 B | 258 | 0 |
| mold 2.42.1 | 17 872 | 7 416 B | 309 | 0 |
| **lccc** | **16 592** | **4 992 B** | **208** | **0** |

**P0-2 `sh_info`.** Was snapshotted between the local and global loops; any
global-loop entry carrying `STB_LOCAL` made it one short. Now derived from the
finished table.

**P0-4 `-s`.** Reached only the linker-script path, so a "stripped" binary
shipped a full symbol table — worse than ignoring the flag, because the user
believes the symbols are gone.

**P0-7 local IFUNC.** `dyn_irelative_count` counted globals only. An IFUNC
symbol's `st_value` *is* the resolver address; locals are keyed by
`(obj_idx, sym_idx)` because local names repeat. Correct in all six
`-no-pie`/`-static`/`-pie` combinations, matching bfd.

**P0-3 PIE.** PIE gates removed; three RELATIVE categories now covered (see the
S06 section above).

**P1-6 `--hash-style`.** Four independent bugs, each of which alone produced a
plausible-looking but wrong image:

1. `bin/lccc_ld.rs` swallowed the flag with `warn_unimplemented`, so no emitter
   change was observable at all.
2. `emit_exec.rs` held **three** copies of the section-header numbering
   (the `h` walk, `sh_count`, hardcoded `dynsym_shidx`/`dynstr_shidx`);
   `emit_shared.rs` had the same disease. Each header now names its own index.
3. With `gnu_hash_size == 0` the two tables **alias the same offset**, and the
   unconditional GNU-hash data writer silently clobbered the SysV table —
   `.hash` had a correct `sh_size` and garbage contents, so `dlopen` segfaulted
   rather than failing cleanly. Both writers are now gated.
4. `next_hdr` in `emit_shared.rs` started at a hardcoded `4`, so
   `--hash-style=both` pointed `.symtab`'s `sh_link` at `.symtab` itself.

`.hash` uses `sh_info = 0` (bfd's value); the "first global symbol index"
reading belongs to `SHT_DYNSYM` only.

**New (found by CI, not by me):** lccc's preprocessor searched
`/usr/include/x86_64-linux-gnu` for **i686** targets. `set_target`'s comment
claimed arch-specific paths are added there; it added none. Every 32-bit
compile failed at `#include <stdio.h>`. Fixed by swapping the `/usr/include`
multiarch dir — and *only* that dir: a substring match also caught
`/usr/lib/gcc/x86_64-linux-gnu/<ver>/include`, which holds GCC's own
freestanding headers and is shared between `-m32` and `-m64`.

## Shared code

`SysvHash` / `build_sysv_hash` / `write_sysv_hash` now live once in
`linker_common/hash.rs` and are used by both emitters. Agent A duplicated the
whole block across `emit_exec.rs` and `emit_shared.rs`; that duplication is
precisely how bug 3 above survives review.

## Still open

* **P0-5 `--emit-relocs`** is a silent no-op on the builtin path. Not fixed.
  It must not stay silent: the kernel links cleanly and then fails to boot.
* lld was not built this session (`setup_oracles.sh` was still running when the
  budget ran out), so the oracle set is bfd/mold/wild.
* Link throughput **not re-measured** since the symtab fix — do not quote the
  S05 numbers; that file was bloated and violated `sh_info == nlocals`.
* P1-1 `.debug_*`/`.comment` dropped · P1-2 no `PT_NOTE`/`PT_GNU_PROPERTY`,
  missing `DT_INIT/FINI/FLAGS_1/RELACOUNT` · P1-4 `-static` +9.3 %, no
  `GNU_RELRO` · P1-5 `-static-pie` refused · P2 no `DT_RELR`, single-word
  bloom, no `SHF_COMPRESSED`, LTO hard error.

## Environment notes for the next session

* `gcc-multilib` / `libc6-dev-i386` are **required** for the i686 codegen
  gates. Without them `/usr/include/i386-linux-gnu/bits/` is empty,
  `gcc -m32` fails identically, and `phi-acyclic-copy-order`,
  `gla-remat-policy` and `tight-loop-align` fail on missing system headers —
  which looks exactly like a compiler regression and is not one.
* The harness wipe strips exec bits from `/home/user/tools/bin` **and** from
  the checkout. Restoring them with a blanket `chmod +x` puts 81 spurious mode
  changes into the patch; upstream tracks most of those `100755` and nine
  `100644`. `artifacts/.base_ref` must also be refreshed after upstream moves,
  or the snapshot folds upstream's own commits into the deliverable.
