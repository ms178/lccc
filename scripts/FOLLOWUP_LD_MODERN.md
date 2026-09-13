# lccc-ld → best-in-class Linux desktop linker: assessment and work plan

**Status of this document:** written after session S04. Every number below was
measured in this sandbox and can be reproduced with the commands in
[§8 Reproducing the evidence](#8-reproducing-the-evidence). Nothing here is
inferred from source reading alone; where a claim comes from reading code rather
than from an experiment it says so.

**Snapshot this plan was written against**

| | |
|---|---|
| repo | `/home/user/lccc`, base `8aeaee0` (upstream `ms178/lccc` main) |
| head | `9624e21` (snapshot **S04-bid-wa-driver**) |
| deliverable | `/home/user/ms178-1.patch`, 129 235 B, sha256 `3401b1371311d0e7b5b9b1cbc5293ed24f18776c0358e87927fe9a845eba7076`, applies clean on base |
| linker suite | **172 pass / 0 fail / 0 warn / 0 skip** (oracles: bfd, mold) |
| `scripts/ci_local.sh --fast` | **31 passed, 0 failed, 3 skipped — ALL GATES GREEN** |
| differential census | `scripts/ld_feature_census.py`: 38 cases × {bfd, lld, mold, lccc}; the only non-`ok` row is `static-pie → REFUSED-OK`, i.e. a deliberate diagnostic |
| host | Debian 13, 2 cores, 1.9 GiB RAM + 8 GiB `/swapfile`. **No PMU** — use proxy metrics (§7) |

Oracles actually available here (apt, because source builds of LLVM/mold/wild do
not fit in 2 cores): **binutils 2.44 (`ld`), lld 19.1.7, mold 2.37.1**.
`tests/linker/setup_oracles.sh` builds newer ones (lld `release/23.x`, mold and
wild from git) when there is room — a stale wild 0.7.0 once produced two false
lccc failures, so never trust a prebuilt oracle you did not version-check.

---

## 1. The one-paragraph summary

lccc-ld is *correct* on the hard semantic problems — symbol resolution, archive
laziness, IFUNC/IRELATIVE, TLS, copy relocs, `.init_array` ordering, version
scripts, `--gc-sections` reachability — and it is the **fastest linker measured
here** (0.021 s vs lld 0.023 s, mold 0.038 s, bfd 0.043 s on a 2.4 MB / 201-object
/ 20 223-relocation link). What stops it being a desktop replacement linker is
not semantics but **the metadata it fails to carry**: it drops `.debug_*`,
emits `ET_EXEC` where every modern distro expects `ET_DYN`, emits no `PT_NOTE`
or `PT_GNU_PROPERTY`, and — the sharpest defect found this session — after
`--gc-sections` it leaves **7 721 symbols at address 0** in `.symtab`, inflating
the symbol table 12× (197 544 B vs lld's 16 632 B) and actively lying to
debuggers and profilers. Those are all *fixable*, mostly small, and each is
independently testable. §3 sequences them.

---

## 2. What already works — do not "fix" these

Verified by the 172-case suite plus the census. Re-deriving them wastes a
session:

`-z now` / `-z relro` (dynamic), `-static` incl. pthread, lazy archive member
selection, `--start-group`/`--end-group`, thin archives, `--gc-sections`
reachability **and** its `.eh_frame` interaction (fixed in S03),
`--export-dynamic` / `-rdynamic` incl. `dlsym`, positional `--as-needed` /
`--no-as-needed`, `--whole-archive` / `--no-whole-archive` (fixed in S04),
IFUNC + `IRELATIVE`, TLS local-exec + `PT_TLS`, copy relocs, `.init_array`
priority ordering, weak-undefined → 0, `-shared` production and consumption,
`-u`, `-M`/`-Map`, `-z max-page-size`, `--sort-common`, `-rpath`/`-runpath`,
`--version-script` narrowing, `--icf=safe|all` (folds identically to lld),
`--build-id` (S04), `--wrap` / `--defsym` in both spellings (S04),
`--exclude-libs`, `--no-undefined`, `-soname`, `-Bsymbolic`.

---

## 3. Defect ledger, ordered by (user impact × frequency) ÷ risk

Each entry: **symptom → evidence → root cause → fix → acceptance test**.
"Acceptance" means a test that must be added to
`tests/linker/run_linker_tests.py` (declarative `case(...)` where possible,
custom `def _x_test(args, oracles)` where the assertion is about the ELF) and
must fail before the fix and pass after.

### P0-1 — `--gc-sections` leaves 7 721 zero-address symbols; `.symtab` is 12× bloated

**Symptom.** After `--gc-sections`, symbols whose defining section was collected
are *kept* in `.symtab` with `st_value = 0` and their original `st_size`.

**Evidence** (201 objects, 2.4 MB, `--gc-sections`, all four linkers produce
`.text` of identical size `0x1fd1` and all run printing `3800`):

| linker | file B | `.symtab` B | nsyms | `.strtab` B | symbols with `value==0 && size!=0` |
|---|---:|---:|---:|---:|---:|
| bfd | 43 344 | 16 752 | 698 | 4 255 | **0** |
| lld | 33 080 | 16 632 | 693 | 4 208 | **0** |
| mold | 35 416 | 18 000 | 750 | 4 627 | **0** |
| **lccc** | **271 824** | **197 544** | **8 231** | **51 871** | **7 721** |

`fn5` / `fn105` / `entry5` are absent from bfd's table and present in lccc's at
`0x0` with size 15/15/396. Without `--gc-sections` lccc and bfd are
**byte-identical** (517 664 B, 0 bad symbols), so the base emitter is fine —
this is purely a GC interaction.

**Why it matters beyond size.** A symbol named `fn5` at address 0 with size 15
tells `addr2line`, `gdb`, `perf` and every sampling profiler that address 0
contains `fn5`. Any null-pointer call is attributed to a real function. This is
worse than a missing symbol.

**Root cause.** `emit_exec.rs` §`.symtab / .strtab` (≈ lines 970–1140) walks the
global symbol map unconditionally. `dead_sections` is computed in
`link.rs` (passed to `gc_collect_sections_elf64_roots_and_sections`, `link.rs:522`)
but is **never passed into `emit_exec`** — confirmed by `grep -n dead_sections
src/backend/x86/linker/emit_exec.rs` returning nothing.

**Fix.** Thread `&FxHashSet<(usize, usize)>` into the symtab writer and skip any
symbol whose `defined_in` section is in it. Two subtleties, both already solved
elsewhere in this tree — reuse rather than reinvent:
* A symbol may be *defined* in a dead section but *referenced* from a live one
  (then it must not have been collected; assert instead of silently dropping).
* Section symbols (`STT_SECTION`) for dead sections must go too, and the
  remaining `st_shndx` values are stable only because collected sections are
  removed from `output_sections` wholesale — verify that assumption, don't
  assume it.

**Acceptance.** New custom test `gc_symtab_no_dead_symbols`: link the 201-object
corpus with `--gc-sections`; assert (a) zero symbols with `value==0 && size!=0`,
(b) `.symtab` byte size ≤ 1.25 × lld's, (c) `.text` size still equals lld's,
(d) output still runs. Add a `scripts/` fixture generator so the corpus is
reproducible — the one used here is in §8.

---

### P0-2 — `.symtab` `sh_info` is wrong (locals not all before globals)

**Evidence.** Parsing the ELF directly (not `readelf` text — see the trap in
§9): bfd `nlocals=207 sh_info=207`; lld `208/208`; mold `748/748`;
**lccc `nlocals=4 sh_info=3`**. `readelf` emits
`Warning: local symbol 16 found at index >= .symtab's sh_info value of 3` on the
lccc output and on none of the others.

**Why it matters.** `sh_info` for `SHT_SYMTAB` is the index of the first global
symbol. Tools that trust it (most do) will classify a local symbol as global.
This is a spec violation, not a style issue.

**Fix.** When building `symtab_entries`, partition locals before globals and set
`sh_info = first_global_index`. Cheap; do it in the same patch as P0-1 since
both touch the same writer.

**Acceptance.** Extend the P0-1 test: assert `sh_info == nlocals` and that no
`readelf -s` warning is produced.

---

### P0-3 — PIE silently downgraded to `ET_EXEC` (no ASLR)

**Evidence.** Every dynamically-linked census row shows `etype DYN→EXEC` for
lccc; the probe binary lands at `0x400000` fixed. Debian's gcc passes `-pie` by
default, so **every** binary built the normal way loses ASLR.

**Why it matters.** This is the single biggest "can I ship this on a desktop"
blocker. A linker that silently removes a security mitigation is worse than one
that refuses.

**Current mitigation.** The `-pie` warning text was made explicit ("no ASLR") in
S04. That is a stopgap, not a fix.

**Fix (this is the largest single item in the plan — budget a whole session).**
`ET_DYN` with no interpreter needs: base address 0, all text/data addresses
relative, `R_X86_64_RELATIVE` emitted for every absolute address in
writable+loaded sections, GOT/PLT indirection for external calls, and
`DT_FLAGS`/`DF_1_PIE` set. The `-static-pie` refusal added in S04 is the same
machinery minus the interpreter, so **do P0-3 first and `-static-pie` becomes
nearly free.**

**Acceptance.** Census `etype` divergence disappears for `hello`, `pie`,
`copyreloc`, `tls`, `ifunc`, `ctors`, `weak`, `export`, `many`. New test:
`e_type == ET_DYN`, `PT_INTERP` present for dynamic, binary runs, and
`readelf -r` shows `R_X86_64_RELATIVE` entries. Cross-check the runtime address
actually varies across runs (`/proc/self/maps`).

---

### P0-4 — `-s` / `--strip-all` is a no-op

**Evidence.** `gcc … -s`: bfd 14 472 B with `.symtab` absent; lccc 7 888 B with
`.symtab` **still present** (the file is smaller only because lccc's baseline is
smaller). `lccc_ld.rs:315` does parse it (`"--strip-all" | "-s" => emit_symtab =
false`) — the flag simply never reaches the writer.

**Fix.** Thread `emit_symtab` into `emit_exec`'s symtab section. Also handle
`--strip-debug` / `-S` (`lccc_ld.rs:314` currently an empty arm) — trivially
correct once P1-1 makes `.debug_*` exist at all.

**Acceptance.** Test `strip_all_removes_symtab`: `-s` ⇒ no `.symtab`, no
`.strtab`, binary still runs; without `-s` ⇒ both present.

---

### P0-5 — `--emit-relocs` is parsed and then ignored

**Evidence.** `-Wl,--emit-relocs`: bfd emits `.rela.text` (1 section); lccc
emits none. `lccc_ld.rs:291` sets `emit_relocs = true` and passes it at
`lccc_ld.rs:698`, so the plumbing exists but the emitter drops it.

**Why it matters.** Required by ftrace/kprobes tooling, by kernel `KALLSYMS`
workflows, and by some sanitizers. Also the mechanism `--emit-relocs`-based
post-link optimizers rely on.

**Acceptance.** Test: `.rela.text` present and its entries match the applied
relocations (spot-check 3 symbols against `readelf -r` of the input objects).

---

### P1-1 — `.debug_*` and `.comment` are dropped entirely

**Evidence.** Input object has 11 `.debug*` sections; bfd's output has 7;
**lccc's has 0**. Census: `sections- .plt.got,.comment→-` on every row.

**Why it matters.** No DWARF means no `gdb`, no meaningful backtraces, no
`perf` symbolization, no debuginfod. For a desktop replacement linker this is a
hard blocker, and it is the item users will notice first.

**Fix.** `.debug_*` are `SHF_ALLOC`-less pass-through sections: copy bytes
verbatim, relocate them (they contain `R_X86_64_*` against section symbols and
line-number tables), preserve order, and do **not** let `--gc-sections` touch
them (they are already excluded as roots after S03 — verify
`gc_sections.rs`'s root predicate still covers `.debug_*` after any change).
Then `PT_NOTE`-adjacent work (P1-2) and `--gdb-index` (P2-4) become possible.
Note the S03 lesson: `.eh_frame` needed *compaction*, not zeroing — `.debug_*`
needs neither, but the same "relocations must be shifted when bytes move"
discipline applies if you ever add `SHF_COMPRESSED`.

**Acceptance.** Test: `.debug_info`, `.debug_line`, `.debug_abbrev`,
`.debug_str`, `.debug_aranges` present; `readelf --debug-dump=info` parses
without error; `addr2line -e out <addr-of-main>` returns the right file:line;
binary size within 1.1 × bfd's for a `-g` build.

---

### P1-2 — No `PT_NOTE`, no `PT_GNU_PROPERTY`

**Evidence.** Probe: bfd `PT_NOTE=3`, `GNU_PROPERTY=1`; lccc `0` and `0`.
Census: `phdrs- NOTE,GNU_PROPERTY→-` on every dynamic row, and
`tags- INIT,FINI,FLAGS_1,RELACOUNT→-`.

**Notes.** `.note.gnu.property` (with `x86 ISA needed: x86-64-baseline`,
`IBT, SHSTK`) *is* emitted as a **section** by lccc — `readelf -n` finds it — but
there is no `PT_NOTE` covering it, so segment-scanning consumers miss it. That
matters for CET: the loader propagates IBT/SHSTK permissiveness across objects
via `PT_GNU_PROPERTY`, and a missing one can silently downgrade a whole process.
`.note.gnu.build-id` (S04) is byte-correct — `file` reports
`BuildID[sha1]=…` and the layout matches bfd exactly — but it too has no
`PT_NOTE`.

**Fix.** Emit one `PT_NOTE` per contiguous run of `.note.*` sections (bfd emits
three because they are not contiguous; one merged segment is legal and better),
plus `PT_GNU_PROPERTY` covering `.note.gnu.property`. Add the missing dynamic
tags in the same pass: `DT_INIT`, `DT_FINI`, `DT_FLAGS_1`, `DT_RELACOUNT`.

**Acceptance.** Census `phdrs-`/`tags-` divergences disappear; new test asserts
`PT_NOTE ≥ 1`, `PT_GNU_PROPERTY` present, all four tags present, and
`readelf -n` reports the build-id (it already does — keep that assertion as a
regression guard).

---

### P1-3 — No `DT_RELACOUNT`

**Evidence.** bfd/lld/mold dynamic outputs carry `RELACOUNT`; lccc does not
(`readelf -dW` count 0 vs 1).

**Why it matters.** `ld.so` uses `DT_RELACOUNT` to bound the relative-relocation
run, which is the dominant startup cost for large PIEs. Free startup win, and it
becomes *mandatory* once P0-3 lands (a PIE without it pays full sort cost on
every exec). Do it together with P0-3.

---

### P1-4 — Static links: no `GNU_RELRO`, and 9.3 % larger than bfd

**Evidence.** `-static`: bfd 755 944 B with `GNU_RELRO=1`; lccc 826 408 B with
`GNU_RELRO=0`. (Census `static` row agrees: `phdrs- NOTE,GNU_PROPERTY,GNU_RELRO`.)

**Fix.** RELRO for static binaries means marking the post-relocation GOT
read-only — the machinery exists for the dynamic path, so this is mostly about
not skipping it when `is_static`. The size delta needs profiling first
(`LCCC_LD_TIME=1` already prints per-phase timings, and `scripts/bisect_boot_size.sh`
exists for size bisection): likely candidates are `.text.unlikely` placement,
`.eh_frame_hdr` duplication, and the `.iplt`/`.rela.iplt` pair that census shows
as lccc-only.

**Acceptance.** `-static` size within 1.03 × bfd; `GNU_RELRO` present;
static binary still runs; `scripts/ci_local.sh` boot gates stay green (they
exercise the static path heavily — treat a regression there as blocking).

---

### P1-5 — `-static-pie` currently refused

**State.** S04 turned a SIGSEGV-in-CRT-self-relocation into a clear diagnostic,
and the census records it as `REFUSED-OK` (see the `expect_link_fail` mechanism
in `scripts/ld_feature_census.py`). **Refusing is correct today.** It stops
being acceptable once P0-3 exists — at that point implement it and delete the
refusal. The test `static_pie_refused` is written to *pass either way* (refuse
with a diagnostic, or link and run) precisely so it does not have to be rewritten.

---

### P2 — modern-linker differentiators

Ordered by payoff for a desktop workload.

1. **`--pack-dyn-relocs=relr` / `SHT_RELR`.** No `DT_RELR` anywhere in
   `emit_exec.rs` (grep: 0 hits). RELR compresses the relative-relocation run by
   ~10× and is the single biggest size+startup win for PIE. Depends on P0-3.
2. **`.gnu.hash` bloom filter is a single 64-bit word with shift 6.** For
   libraries exporting thousands of symbols this makes lookup measurably slower
   than bfd/lld. Size the bloom filter as
   `max(1, next_pow2(nsymbols / 16))` words and pick the shift from the bucket
   count — the formula is standard, the current constant is not.
3. **Parallelism beyond relocation application.**
   `src/backend/x86/linker/parallel_reloc.rs` already uses
   `std::thread::scope` + `available_parallelism` (no external deps — keep it
   that way). Not yet parallel: input parsing, `.eh_frame` merge, string-table
   dedup, symtab construction. On a 2.4 MB link lccc is 0.021 s vs lld 0.023 s —
   already ahead, so **measure before investing**; the win shows up on 500 MB
   kernel/glibc-scale links, not here. Use `LCCC_LD_TIME=1` phase timings to
   find the actual serial bottleneck rather than guessing.
4. **Compressed debug (`SHF_COMPRESSED`) and `--gdb-index` / `.debug_names`.**
   Both are large-desktop quality-of-life. Blocked on P1-1.
5. **LTO plugin is a hard error.** Any real distro build hits this immediately.
   Largest single remaining feature gap; scope it as its own project.
6. **Observability.** `link_builtin` has no input-order diagnostics — this cost
   real time during S04 (see §9, trap 3). `x86/linker/input.rs:46` already
   establishes a `LINKER_DEBUG` idiom; add a matching trace of the resolved
   input list with per-item `whole_archive`/`as_needed` state, and make it a
   permanent diagnostic rather than scaffolding.

---

## 4. Static-source gaps (from reading, not yet measured)

Flagged as *unverified* — confirm each with an experiment before planning
around it.

* Script mode (`-T`, Mode 2) never consults `passthrough`, so `--version-script`,
  `--exclude-libs`, `-rpath` and `--export-dynamic` are silently dropped for
  script links. (This affects kernel links.)
* `-r` on i386 unimplemented.
* `--fatal-warnings` ignored — a build system relying on it gets silent
  acceptance.
* `R_X86_64_GOT32` unhandled.
* Hard-coded `libc`/`libm`/`libgcc_s` fallback; no `ld.so.cache` consultation.
* `resolve_sym` maps a plain undefined symbol to `0` rather than erroring — a
  latent miscompile source.
* Keyword census counts that hint at thin coverage: `gnu.property` **1**,
  `pack_dyn` / `DT_BIND_NOW` / `separate_code` / `debug_frame` **0**.

---

## 5. Suggested sequencing

Dependency-driven, not size-driven. Each milestone ends green on
`scripts/ci_local.sh --fast` **and** the full linker suite, and ends with a
snapshot (§6).

| # | Milestone | Contents | Rough size |
|---|---|---|---|
| M1 | **Symtab correctness** | P0-1, P0-2, P0-4 (all one writer), P0-5 | small–medium |
| M2 | **Metadata fidelity** | P1-1 (`.debug_*`), P1-2 (`PT_NOTE`/`PT_GNU_PROPERTY`/tags), P1-3 (`DT_RELACOUNT`) | medium |
| M3 | **PIE** | P0-3, then P1-5 (`-static-pie`) falls out, then P2-1 (RELR) | large — own session |
| M4 | **Static-link quality** | P1-4 (RELRO + size bisection) | medium |
| M5 | **Scale & polish** | P2-2 (`.gnu.hash`), P2-3 (parallelism, measure first), P2-6 (observability) | opportunistic |
| M6 | **LTO** | P2-5 | own project |

M1 first because it is small, self-contained, and removes actively-wrong output.
M3 is the one that changes lccc-ld's category; do not start it with M1/M2
unfinished, because a PIE link magnifies every metadata defect.

---

## 6. Working agreements (carry these forward — they are hard-won)

* **Build with the `fastbuild` preset, always, `-j2`:**
  `CARGO_BUILD_JOBS=2 cargo build --profile fastbuild --bins`.
  `--bin lccc-ld` alone is **not** enough: `lccc-x86` statically embeds the
  built-in linker, so a stale `lccc-x86` will silently test old code. This has
  already burned one session.
* **Snapshot immediately after each validated fix**, using
  `scripts/lccc-snapshot.sh <NAME>` (it writes `/home/user/ms178-1.patch`,
  `/home/user/artifacts/{lccc-src.tar.gz,lccc.bundle,SNAPSHOT_LEDGER.md}` and
  verifies the patch applies clean). The harness wipes the sandbox and can die
  mid-session.
* **Swap:** `/swapfile` (8 GiB) must exist and be active. Work on the volume
  with the most free space.
* **Tests.** `LCCC_BIN=/home/user/lccc/target/fastbuild/lccc-x86 python3
  tests/linker/run_linker_tests.py [--filter SUBSTR]`. Every fix lands with a
  test that fails without it.
* **Census.** `python3 scripts/ld_feature_census.py --linkers bfd,lld,mold,lccc
  --md out.md --json out.json`. `--cases` is *substring* matching. Use its
  `expect_link_fail=("lccc",)` field for deliberate refusals so they report as
  `REFUSED-OK` instead of `LINKFAIL`.
* **Correctness is a hard constraint.** Generated-code performance > GCC/Clang
  compatibility > simplicity > elegance. GCC/Clang/ICX are baselines, never role
  models — do not assume their algorithm is optimal.
* **No debug leftovers in the delivered patch.** Env-gated diagnostics matching
  an existing idiom (e.g. `LINKER_DEBUG`, `LCCC_LD_TIME`, `LCCC_DEBUG_GCEH`) are
  fine and useful; scaffolding added to chase one bug is not. When in doubt,
  delete it and record the observability gap in §3 P2-6 instead.
* **Keep `cargo fmt` clean** — it is a CI gate and it has failed twice already.

---

## 7. Measurement policy (no PMU in this VM)

Hardware target is an Intel i7-14700KF (Raptor Lake); this VM has no PMU, so use
proxy metrics and say which one:

* **Link time:** best-of-N wall clock, N ≥ 7, warm page cache, linker invoked
  *directly* (not through gcc) so the driver is not in the number. Report the
  input size, object count and relocation count with it — a bare time is
  meaningless.
* **Size:** total file, plus per-section from a **direct ELF parse** (§9 trap 1).
* **Startup:** `DT_RELACOUNT` presence and relative-relocation count as the
  proxy for `ld.so` work; actual exec timing is too noisy at this scale here.
* **Codegen:** the Godbolt-derived oracle work is a separate thread; for linker
  purposes assert `.text` *byte size* equality against lld/mold on the same
  inputs — that catches layout regressions without needing cycle counts.

---

## 8. Reproducing the evidence

Everything below ran in this sandbox. The 201-object corpus generator (used for
P0-1/P0-2 and the link-time numbers):

```bash
mkdir -p /home/user/ldwork/perf && cd /home/user/ldwork/perf
mkdir -p shim && ln -sf /home/user/lccc/target/fastbuild/lccc-ld shim/ld
python3 - <<'PY'
N=200; F=20; TOT=N*F
hdr = "".join(f"extern int g{k};\n" for k in range(TOT))   # externs are REQUIRED
for i in range(N):
    with open(f"f{i}.c","w") as f:
        f.write(hdr)
        for j in range(F):
            k=i*F+j
            f.write(f"int g{k};\nint fn{k}(int x){{ g{k}=x; return x+g{(k+3)%TOT}; }}\n")
        f.write(f"int entry{i}(void){{ int s=0; "
                + "".join(f"s+=fn{i*F+j}({j});" for j in range(F)) + " return s; }\n")
with open("main.c","w") as f:
    f.write("#include <stdio.h>\n")
    for i in range(N): f.write(f"extern int entry{i}(void);\n")
    f.write("int main(void){ printf(\"%d\\n\", "
            + "+".join(f"entry{i}()" for i in range(0,N,10)) + "); return 0; }\n")
PY
for f in f*.c main.c; do gcc -c -O1 -ffunction-sections -fdata-sections $f -o ${f%.c}.o; done
# 201 objects, 2.4 MB, 20223 R_X86_64 relocations
```

Link-time comparison (best of 9; **note `-L`, see §9 trap 2**):

```bash
CRT="/usr/lib/x86_64-linux-gnu/crt1.o /usr/lib/x86_64-linux-gnu/crti.o /usr/lib/x86_64-linux-gnu/crtn.o"
for L in "bfd:ld" "lld:ld.lld" "mold:ld.mold" "lccc:shim/ld"; do
  tag=${L%%:*}; b=${L#*:}; best=99
  for i in $(seq 1 9); do
    t0=$(date +%s.%N)
    $b -o o_$tag main.o $(ls f*.o) --gc-sections \
       -dynamic-linker /lib64/ld-linux-x86-64.so.2 -L/usr/lib/x86_64-linux-gnu $CRT -lc
    rc=$?; t1=$(date +%s.%N); best=$(python3 -c "print(min($best,$t1-$t0))")
  done
  printf "%-5s %ss rc=%s out=%s size=%s\n" "$tag" \
    "$(python3 -c "print(f'{$best:.3f}')")" "$rc" "$(./o_$tag)" "$(stat -c%s o_$tag)"
done
#   bfd   0.043s rc=0 out=3800 size=43344
#   lld   0.023s rc=0 out=3800 size=33080
#   mold  0.038s rc=0 out=3800 size=35416
#   lccc  0.021s rc=0 out=3800 size=271824
```

Symbol-table forensics — `scripts/elfprobe.py` is committed for this purpose
(parses the ELF directly instead of scraping `readelf`):

```bash
python3 scripts/elfprobe.py o_bfd o_lld o_mold o_lccc
```

Feature probe (`.debug`, `PT_NOTE`, `RELACOUNT`, `-s`, `--emit-relocs`, `--icf`,
`-static`): `ldwork/gcchk/probe.sh` in the sandbox; the results are the tables in
§3.

---

## 9. Traps that already cost time — read before debugging

1. **Never scrape `readelf -SW` with `awk '{print $2}'`.** `readelf` wraps each
   section header over two lines when names are long, so columns shift and you
   get garbage. Use `readelf -SW` with a real regex, or parse the ELF directly
   (`scripts/elfprobe.py`). This produced two wrong conclusions in one session.
2. **A benchmark that measures a failed link looks like a great result.** Three
   of four linkers initially "failed" only because `-L/usr/lib/x86_64-linux-gnu`
   was missing; lccc appeared fastest at 0.011 s. **Always assert the output
   exists, runs, and prints the expected value before believing a time.**
3. **`link_builtin` and `link_shared` are different code paths.**
   `src/backend/x86/linker/link.rs` has exactly two entry points:
   `link_builtin` (line 20) and `link_shared` (line 559). The ordered-input
   machinery (`ordered_items`, per-item `whole_archive`/`as_needed`) lives only
   in `link_shared`. `link_builtin` resolves through flat name lists
   (`libs_to_load`, `deferred_libs`) — that asymmetry is why `--whole-archive`
   was silently ignored for executables, and an earlier "one-line fix" claim was
   simply wrong. Check which function you are in before concluding anything.
4. **Beware shadowed `else if` arms in `lccc_ld.rs`'s long fallback chain.** A
   newly added `a.starts_with("--build-id")` branch was dead code because the
   same predicate appeared ~40 lines earlier. Grep for an existing prefix match
   first.
5. **`gcc -B<dir>` only selects lccc-ld if `<dir>` contains a file literally
   named `ld`.** `-B target/fastbuild` silently falls back to bfd and you will
   benchmark the wrong linker. Always build a shim dir (`tests/linker/`'s
   `_shim_for` helper does this).
6. **`-Wl,` is a compiler-driver prefix, not a linker argument.** Passing
   `-Wl,--build-id` directly to `lccc-ld` correctly yields "unknown option".
   Conversely, inside `linker_common/args.rs` each `-Wl,` group was split
   independently, which broke `--opt VALUE` pairs (`-Wl,--wrap -Wl,bv`); S04
   fixed this by re-joining such pairs up front (`VALUE_TAKING` table). If you
   add a two-argument option, **add it to that table too.**
7. **`run_linker_tests.py` `expect_stdout` is compared exactly** — omitting the
   trailing `\n` fails a passing test. `run_bin()` returns `(rc, str)`, not
   bytes. `setup=` takes a **callable** `(tmpdir) -> None`, not a string.
8. **A `--whole-archive` test where `main` references the archive member proves
   nothing** — lazy loading pulls it in anyway. The suite's `whole_archive_exec`
   case was exactly this and could not fail; it now defines the shared variable
   in `main.c` so only `--whole-archive` can pull the member. Pair every such
   test with its negative (`archive_lazy_loading_is_lazy`).
9. **`mawk` has no `strtonum`.** Map FDE ranges to symbols in Python.
10. **Do not zero dead FDEs in place** — a zero-length record is the DWARF
    end-of-`.eh_frame` terminator. Compact, fix relative `CIE_pointer`s, remap
    relocs. `SectionData` has no `DerefMut`, so an owned `Vec<u8>` is required.
11. **A keyword grep census is not evidence** (`RELR` matches `RELLO`), and the
    x86 linker README's LOC counts are 2–3× stale. Trust the code and the
    binary.

---

## 10. Session ledger

| snapshot | head | what it contains | suite | CI |
|---|---|---|---|---|
| S03-gc-ehframe | `176b3e12` | `--gc-sections` no longer destroys `.eh_frame`; `--export-dynamic` roots; `ld_feature_census.py`; `check_gc_eh_frame.py` | 166/0 | green |
| **S04-bid-wa-driver** | `9624e21` | `--build-id` note emission; `--whole-archive` honoured on executable links; two-argument `--wrap`/`--defsym`/`--icf`/`-rpath` via `-Wl,` re-joining; `-static-pie` refused instead of SIGSEGV; warn on unimplemented options; +6 tests | **172/0** | **green (31/0/3)** |
