# Follow-up: x86-64 / i686 linker, review of PR #661 (S24) — 2026-09-28

Base: ms178/lccc `main` @ 6f8ace9c (PR #661 merged).  Scope: the x86-64 and
i686 backends and their linkers only.

## Review of the S24 audit: verdicts

| # | Finding | Verdict | Outcome |
|---|---------|---------|---------|
| 1 | Only a small part of the linker suite runs in CI | **Agree, and it was worse.** CI ran `run_linker_tests.py --filter i386_` (a handful of tests). The parity checker did not track `tests/linker/*` at all, so dropping the step would have passed. | Fixed (see "CI") |
| 2 | Undefined symbols with non-default visibility | Plausible: the DSO match in `link.rs` ignores hidden visibility, and `emit_shared` zero-fills a strong hidden undefined symbol. | **Open** (needs a strong/weak × visibility × location × reference-kind matrix) |
| 3 | GOTPCRELX relaxation trusts the relocation type, not the bytes | **Agree.** The linker never checked that the promised REX/REX2 prefix is really there. The rewrite kept REX.B, so `49 8b 05` became a move to `%r8`. GNU ld 2.47 still has this bug; measured. | Fixed |
| 4 | Linker-created absolute symbols are treated as addresses | **Agree.** Constant `--defsym` values got RELATIVE relocs / were slid by relaxation in PIE, and address `--defsym`s in a `.so` got no RELATIVE reloc. | Fixed |
| 5 | Version-script `local:` precedence | Plausible (`emit_shared.rs` export filter vs. GNU precedence). | **Open** |
| 6 | Executables have no GOT slots for local symbols; opcode-guessing fallback | **Agree.** | Fixed |
| 7 | `real_workloads.py` scoring (wrong outputs not disqualified, RSS, naming) | Plausible. | **Open** |

Found while fixing 3/6: in IE→LE for local symbols the immediate was
`tpoff + A`, which is 4 bytes low (`A` is the -4 of a pc-relative field).
Every local initial-exec TLS access in an executable read the wrong address.
Fixed, and covered by `ie_to_le_local`.

## Done this session

- **x86 lccc-ld** (`elf.rs`, `plt_got.rs`, `emit_exec.rs`, `emit_shared.rs`,
  `link.rs`, `types.rs`, `linker_common/defsym.rs`):
  - GOTPCRELX/REX/REX2 relaxation checks the prefix inside the section, and
    the rewrite moves REX.B to REX.R.
  - REX/REX2 `call`/`jmp *GOT` are not relaxed.
  - `GotTarget::{Image, Absolute}`: absolute targets relax only to
    immediates, and only when the value fits.
  - Executables get local GOT slots (section symbols included); local GOT64
    works.
  - `GlobalSymbol::absolute` and GNU-style `--defsym` classification
    (`is_address_expression`). In PIE: no RELATIVE reloc for absolute GOT
    slots or `.quad`, and the dynsym entry is `SHN_ABS`. In a `.so`: address
    defsyms get a RELATIVE reloc and a real section.
- **Tests.** Each one fails on the previous build:
  - `gotpcrel_edges`: `7 55 1 1 1 1 1`
  - `absolute_symbols_pic`
  - `ie_to_le_local`: `11 22 33 33`
  - new unit tests in `backend::x86::linker` and `linker_common`
- **Linker test harness** (`tests/linker/run_linker_tests.py`):
  - Every test is a registry entry with declared names and tags, so
    `--filter`, `--tag` and the new `--list` are exact.
  - A result whose name is undeclared or duplicated is reported as FAIL.
  - A declared test that its runner silently did not report is reported as
    SKIP.
  - `--strict` fails on SKIP or WARN. `--json FILE` writes the results.
- **Test gaps closed:**
  - The "R_X86_64_32S" range case really emitted R_X86_64_32 (`.long`), under
    a duplicate name. It now uses `.reloc` and checks 0x80000000.
  - The C++ parts of the DSO tests reported PASS when `g++` was missing.
    They now report SKIP.
  - Reference-link failures used to drop results silently. They are now
    reported.
- `tests/linker/setup_kernel_tools.sh`: the v6.12 sources are pinned by
  SHA-256, and the default prefix is `$HOME/tools`.
- **CI:**
  - `ci.yml` and `ci_local.sh` run the whole suite: `--strict`,
    `LCCC_REQUIRE_I386=1`, the kernel relocs tool, and the pinned GNU as 2.47
    first in `PATH` (APX/REX2 fixtures).
  - The job installs `g++-multilib`, which the i386 C++ DSO test needs.
  - `check_ci_gate_parity.py` tracks `tests/linker/*`.
    `check_linker_suite_parity` rejects every weaker spelling; it has mutation
    tests in `test_ci_gate_parity.py`.

Validation on the rebased tree:
- Linker suite 274 pass / 0 fail / 0 warn / 0 skip, both with the system
  toolchain and with the exact CI command (GNU as 2.47).
- rustfmt and `cargo clippy --all-targets -D warnings` are clean.
- `ci_local.sh --fast` ran (see the session log).

## To do

1. **Finding 2.** Undefined-symbol visibility matrix. Then make hidden
   undefined references refuse to bind to DSO definitions, and error on a
   strong hidden undefined symbol in `-shared`, as GNU does.
2. **Finding 5.** Version-script `local:` with GNU precedence (exact name >
   wildcard, `global` over `local` at equal rank).
3. **Finding 7.** `real_workloads.py`: disqualify wrong outputs, report RSS as
   N/A when it is not measured, rename `loaded_bytes`.
4. **lccc-ld drops `--disable-new-dtags`/`--enable-new-dtags`** when called
   as `ld` by a GCC driver (`src/bin/lccc_ld.rs` falls through to "ignoring
   unknown option"). A plain `-rpath` then always gives DT_RUNPATH.
   - The `-Wl,` path through the lccc driver is correct, and so is the
     existing x86-64 test.
   - Fix: forward both spellings. Test: an i386 and x86-64 matrix over
     enable, disable and default.
5. **GOTPCRELX fixtures under older assemblers.** `gotpcrel_edges` reads
   `7 55 1 1 0 1 1` when the fixture is assembled with GNU as 2.42. CI pins
   2.47, so this does not affect CI. Worth checking whether 2.42 emits a
   different but valid encoding that lccc-ld should accept.
6. Earlier open items:
   - -fPIC regression test ("5 20", no PC32).
   - `-Bsymbolic-functions` with copy relocations.
   - `-z` keyword coverage.
   - `.so` RELRO.
   - PC32 refusal.
   - `DT_SYMBOLIC` through lccc_ld.
   - x86 `--dynamic-list`.
   - i686 `@GOTOFF` against preemptible symbols.
7. Carried over (x86):
   - `.plt.got`.
   - Relaxation in `emit_script`.
   - `DF_STATIC_TLS` in `.so`.
   - RW `.rela.dyn` in `.so`.
   - Linker-created absolutes are never relaxed.
   - Negative absolute immediates.
   - REX2/CODE_4 GOTTPOFF on locals.
8. Carried over (i686):
   - GOT32X on named globals.
   - `.so` versioning / PIE.
   - IFUNC in `.so`.
   - `--dynamic-list` / `--version-script`.
