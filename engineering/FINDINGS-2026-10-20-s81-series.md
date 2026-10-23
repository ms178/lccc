# S81 series findings ledger (rebased onto d8668ee24)

Every entry ends FIXED with a test, or states exactly what remains open.
Register-homing work has its own ledger: FINDINGS-2026-10-20-homing.md.
| id | area | defect (measured) | status |
|---|---|---|---|
| TLS-W1 | x86-64 codegen+ld | non-PIC exec reads extern __thread from .so via LE -> garbage; lccc-ld links R_X86_64_TPOFF32 vs DSO symbol silently | FIXED (probe 2026-10-20: `-fno-pic` extern → `x@GOTTPOFF` IE; local `g`/`s` → `@TPOFF`) |
| TLS-W2 | x86-64 codegen | -fPIC -c emits TPOFF32 for static TLS -> any -shared link fails | FIXED (probe: `-fPIC` emits no TPOFF; descriptors `@TLSDESC/@TLSCALL` as GCC 16.2 gnu2) |
| TLS-W3 | x86-64 codegen | -fPIC -shared uses IE (DF_STATIC_TLS) -> dlopen of 1MiB TLS lib fails | FIXED (probe: `-fPIC` uses descriptors, never IE/DF_STATIC_TLS) |
| TLS-I686 | i686 codegen | PIC: GOTNTPOFF for all (static too: missed LE in PIE); non-PIC: NTPOFF for extern (wrong) | FIXED (probe: `-m32 -fPIC` → `@TLSDESC/@TLSCALL`; non-PIC extern → `@INDNTPOFF`, locals `@NTPOFF`) |
| TLS-A64/RV | arm/riscv codegen | LE for everything incl. extern/shared | FIXED (probe: arm exec `:gottprel:` IE + `:tprel_hi12:/:tprel_lo12_nc:` LE, `-fPIC` `:tlsdesc:`+`.tlsdesccall`; riscv `la.tls.ie` / `%tprel_*` / `la.tls.gd`) |
| ASM-386-IE | i686 asm | @INDNTPOFF -> R_386_TLS_GOTIE; GAS 2.44 = R_386_TLS_IE (measured) | FIXED (I686-AS-TLSMOD) |
| ASM-386-DESC | i686 asm | no @TLSDESC/@TLSCALL | FIXED (I686-AS-TLSMOD) |
| ASM-A64-TLS | arm asm | no :tlsdesc:/:tlsdesc_lo12:/.tlsdesccall/:gottprel:/:gottprel_lo12: | FIXED (probe: arm `-fPIC -c` assembles `:tlsdesc:`/`.tlsdesccall`/`:gottprel:` objects) |
| ATTR-TLS | frontend | __has_attribute(tls_model)=1 but attribute silently ignored | FIXED (UP-TLS-POLICY, OWN-ATTR-DIAG; gate `check_tls_model_selection.sh`) |
| VIS-SPEC | parser | `__attribute__((visibility("hidden"))) int a, b;` -> b DEFAULT (GCC: HIDDEN) | FIXED (probe: `readelf` a,b both GLOBAL HIDDEN = gcc-16) |
| CLI-TLS | driver | -ftls-model=global-dynamic/local-dynamic rejected | FIXED (probe: all four `-ftls-model=` values accepted) |
| RV-I128-F128 | riscv codegen | `(long double)(__int128)x` panics i128_ops.rs:297 | FIXED (probe under qemu: `(long double)((__int128)1<<100)+12345` round-trips exactly; `h(-7)==-7.0L`) |
| RV-F128-ADD | riscv codegen | long double add emits 3x __trunctfdf2 around __addtf3 (GCC: tail __addtf3) | ANALYSED, NOT FIXED: result is CORRECT (quad reloaded into a0/a1); the `__trunctfdf2` calls build a dead f64 shadow (`f128_truncate_result_to_acc`, `emit_call_store_result_impl`) consumed by untracked f128 paths in the shared `f128_softfloat.rs` (ARM+RISC-V), and F128 params trigger a 128-byte all-arg-register dump. Removal needs a consumer audit across both non-host arches + qemu validation; scheduled after host-x86 work |
| RV-LD-LIBGCC | riscv lccc-ld | __trunctfdf2 unresolved at link time but no error; runtime symbol lookup error | FIXED (probe: `lccc-riscv` links libgcc; `add(1.5L,2.25L)` prints 3.75 under qemu) |
| TLS-BLOCK-EXTERN | lowering | block-scope `extern __thread int z;` (no file-scope decl) lowered as plain data: `z(%rip)` writes the .tdata TEMPLATE (measured: set z=7, main prints 41) | FIXED (probe: block-scope `extern __thread` before/after the definition → 7, rc 0) |
| TLS-ASM-EXTERN | lowering | check `extern __thread int x __asm__("y")` IR-name consistency | FIXED (probe: `extern __thread int x __asm__("y")`; `x=9` → `y`==9. gcc-16 prints 5: it treats x and y as distinct objects for alias analysis — lccc follows the symbol) |
| PH-TLSCALL | x86-64 peephole classified `call *x@TLSCALL(%rax)` as a full call: dead-write pass deleted a live `imull` (W2 printed 11, expect 56); after reclassifying as `Other{rax}`, `remove_dead_leaf_frame` deleted the alignment `subq $8` | found in own TLS work | FIXED: `is_tls_descriptor_call` → `Other{rax}` pinned + counted as call by the frame pass; i686: opaque `InlineAsm` class; unit tests in types.rs/frame_compact_tests.rs/i686 peephole |
| I686-AS-TLSMOD | i686 assembler: lowercase GCC TLS operators rejected (`@gotntpoff`) or silently `R_386_32` (`$x@ntpoff` ALU imm); `@indntpoff` mapped to GOTIE (GAS: TLS_IE); base-less `x@indntpoff` became R_386_32 against symbol `x@indntpoff`; no `@tlsdesc`/`@tlscall`; GOT-slot TLS addends kept (GAS zeroes) | pre-existing | FIXED: one case-insensitive `i686_modifier_reloc`; TLSDESC=39, DESC_CALL=40 (`ff 10`, field-less patch width 0); GAS byte-diff identical |
| I686-GOT32X | i686 assembler emitted `R_386_GOT32` for `movl x@GOT(%ebx),%r` where GAS emits relaxable `R_386_GOT32X`, so lccc-ld never relaxed lccc objects to `leal @GOTOFF` | pre-existing | FIXED: GAS-identical relocations; lccc-ld conversions at ld 2.47 parity. Gate `check_i686_got32x_relax.sh` |
| I686-FRAMELESS-ALIGN | i686 `eliminate_unused_callee_saves` FRAMELESS shape removed unused push/pop pairs in functions with calls: 3 pushes realigned %esp for `call foo`; after removal %esp ≡ 8 (mod 16) at the call (ABI: 16) | pre-existing (probe: peephole_optimize on 3-push/2-call body) | FIXED: with an alignment-observing call only multiples of 4 pairs are removed; 3 unit tests |
| RV-LD-ALIGN | riscv lccc-ld ignored `R_RISCV_ALIGN`: a GAS object's worst-case NOP run survived untrimmed (`/tmp/rva`: bfd puts `f2` at 0x10120, lccc-ld at 0x101a2, misaligned) | pre-existing | FIXED: `riscv/linker/align.rs` trims each run at parse time (valid: lccc-ld does no other size-changing relaxation), remaps symbols/relocs/addends, raises `sh_addralign`, bfd's "N bytes required" error; 5 unit tests + gate leg GAS+lccc-ld |
| RV-AS-ALIGN | riscv assembler padded code alignment only to the boundary with addend = pad: names too small an alignment for short pads, too few bytes when earlier code shrinks (bfd: "12 bytes required for alignment to 16-byte boundary, but only 8 present" at -O2), no reloc at all for a zero pad | pre-existing (upstream design) | FIXED: GAS `riscv_frag_align_code` parity — step to insn_alignment, worst-case run `align - insn_alignment`, ALIGN addend = run; insn_alignment = rvc?2:4 tracked through `.option rvc/norvc/push/pop` and `-march` (EF_RISCV_RVC); matrix rows re-measured on as 2.47 -march=rv64gc |
| RV-AS-CODEFLOOR | riscv code alignment floor raised the DIRECTIVE to 4 (`.byte 1;.p2align 0,,3;.byte 2` put 02 at 4; GAS 2.47: offset 1, both marches) and was 4 even for RVC objects; measured under a no-C default march, tail padding misread as directive padding | pre-existing | FIXED: floor raises `sh_addralign` only, = insn_alignment (2/4) with `text_align`; tests `explicit_fill_byte_is_honoured_in_text`, matrix |
| RV-AS-LABELDIFF | riscv assembler folded data differences of code labels to constants (`.word b-a`, jump tables `.word .LBBn-.Ljt`): stale after relaxation (GAS: ADD/SUB pair, any section) | pre-existing | FIXED: `ElfWriterBase::keep_code_labels` (+ eval-mode cell, `.` as GAS `.L0 ` label); decided by relax state at end of input as GAS; `.set`/`.org` still fold (GAS-measured); unit tests `relax_value_tests` + runtime gate |
| RV-AS-SECSYM | riscv assembler relocated lone local code labels as section+offset (`.quad .L1` → `.text+8`), which linkers do not adjust when deleting bytes (GAS `tc_fix_adjustable`=0 for code) | pre-existing | FIXED: same mechanism; `a_code_label_is_not_rewritten_to_its_section`; computed-goto leg of the runtime gate |
| RV-AS-ULEB128 | riscv assembler rejected `.uleb128`/`.sleb128` ("unsupported RISC-V assembler directive") | pre-existing | FIXED: constants, canonical forward widths by re-assembly with grown widths, SET/SUB_ULEB128 over code (addend kept — GAS drops it), GAS's "non-constant .sleb128 is not supported"; 4 unit tests + gate |

## Linker unwind/dynamic defects found by the extended CFI gate (2026-10-20)

| ID | Defect | Evidence | Status |
|---|---|---|---|
| RV-LD-EHFRAMEHDR | RISC-V exec: synthetic `.eh_frame_hdr` pushed after `sec_indices` was computed -> never laid out/written; PT_GNU_EH_FRAME at vaddr 0. Same for GCC-compiled objects (pre-existing). | `found=0 depth=1` with gcc -O2 object linked by lccc-riscv | FIXED (layout order inserted before `.eh_frame`); gate legs PASS |
| RV-LD-SHARED-EHHDR | RISC-V `-shared`: no `.eh_frame_hdr`/PT_GNU_EH_FRAME at all | readelf -l | FIXED (emit_shared) |
| A64-LD-EHFRAMEHDR | AArch64 lccc-ld exec + shared: no PT_GNU_EH_FRAME | `found=0 depth=1` whole program | FIXED (2026-10-20 re-probe: PT_GNU_EH_FRAME in exec+.so on arm/riscv; DT_RUNPATH written; `check_builtin_dso_link.sh` PASS x86-64/i686/aarch64/riscv64 incl. crtbeginS `__dso_handle` atexit leg) |
| RV/A64-LD-RPATH | `-rpath` parsed but no DT_RUNPATH/DT_RPATH written (exec + shared) | readelf -d | FIXED (2026-10-20 re-probe: PT_GNU_EH_FRAME in exec+.so on arm/riscv; DT_RUNPATH written; `check_builtin_dso_link.sh` PASS x86-64/i686/aarch64/riscv64 incl. crtbeginS `__dso_handle` atexit leg) |
| RV/A64-LD-SHARED-LIBS | `-shared`: no crti/crtbeginS/crtendS/crtn, no default `-lgcc --as-needed -lgcc_s -lc`; ld scripts skipped (`-lc` gives no NEEDED); missing `-lfoo` silently ignored | DSO has no NEEDED -> `undefined symbol: _Unwind_Backtrace` at load | FIXED (2026-10-20 re-probe: PT_GNU_EH_FRAME in exec+.so on arm/riscv; DT_RUNPATH written; `check_builtin_dso_link.sh` PASS x86-64/i686/aarch64/riscv64 incl. crtbeginS `__dso_handle` atexit leg) |

## Rebase onto 4c0d36489 (PR #873 GNU TLS set) — red-team of both TLS systems (2026-10-20)
| id | area | defect (measured) | origin | status |
|---|---|---|---|---|
| MRG-TLS-2SYS | everywhere | two TLS model systems after auto-merge: 2 `TlsModel` enums, 2 `tls_model` AST fields (String + parsed), 2 `parsing_tls_model` fields, 2 attribute arms (the 2nd unreachable), 2 `call *@tlscall` encoders (2nd dead), attribute map AND `IrGlobal.tls_model`, two `tls_models` fields in `CodegenState` | merge | FIXED: one enum (`common::tls`, re-exported by `ir::module`), one attribute store (`IrModule.tls_model_attrs`), one accessor `CodegenState::tls_model` over one table; dead arms/fields/fns deleted |
| UP-TLS-POLICY | backend/state.rs | upstream `tls_model_for_symbol`: `-ftls-model=` and the attribute OVERRIDE (GCC: floor, max(default,floor,attr)); hidden TLS in -fPIC → GD (GCC: LD); non-PIC GD floor on a definition → descriptor + PC thunk (GCC 16.2 -m32: `%gs:value@ntpoff`) | upstream | FIXED: policy is `common::tls::model_table`; gate legs measured against gcc-16 |
| UP-TLS-REDECL / OWN-TLS-REDECL | lowering | GCC 16.2 uses the LAST declaration's model for every access (file + block scope; a later PLAIN declaration resets to the default: `extern IE; extern;` → GD under -fPIC). Upstream kept the inherited model on plain extern redeclarations (wrong) and dropped it when the IrGlobal was re-created; mine kept the last EXPLICIT model (wrong for plain later decls) | both | FIXED: every TLS-object declaration records its attribute or clears it; 9-symbol gate leg = gcc-16 relocations exactly |
| UP-386-INDNTPOFF | i686 asm | upstream mapped `@INDNTPOFF` → R_386_TLS_GOTIE (non-PIC IE reads the GOT-slot ADDRESS: GOTIE is an %ebx offset → wrong address); case-sensitive (GCC's lowercase rejected); TLSCALL in the field mapper | upstream | FIXED: mine (`i686_modifier_reloc`, GAS-probed: indntpoff=TLS_IE, gotntpoff=TLS_GOTIE, case-insensitive); unit test asserts TLSCALL is field-less |
| UP-386-PSEUDO-CLASS | i686 peephole | `lccc_tls_call32` pseudo was an unknown mnemonic to the i686 classifier: no %eax effect, not a call for frame alignment/elision (glibc `_dl_tlsdesc_dynamic` slow path needs the 16-byte psABI stack) | upstream | FIXED: classifier keys on the pseudo (opaque InlineAsm class, aligned call); x86-64 classifier likewise gives `lccc_tls_call` its %rax effect, pinned; unit tests without volatile markers |
| UP-LE-GATE | x86 memory.rs | LE direct fold additionally required `local_symbols ∥ tls_definitions` (refused explicit `tls_model("local-exec")` externs that GCC folds) and a `shared_lib` check the unfolded path does not have | upstream | FIXED: model == LE decides |
| OWN-ATTR-DIAG | parser/lowering | my attribute errors/warnings were not GCC's text (`"local-exec"… or "global-dynamic"` vs GCC `'local-exec', …, or 'global-dynamic'`; `attribute ignored` lacked `because 'y' does not have thread storage duration` / `'f' is not a variable` and `-Wattributes`) | own | FIXED: verbatim gcc-16 text; new `WarningKind::Attributes` (`-Wno-attributes` silences, `-Werror=attributes` promotes); gate legs |
| UP-STATE-DOC | backend/state.rs | orphan doc line "Counter for generating unique labels" above `pic_mode` | upstream | FIXED |
| UP-TLS-FOLD-DEAD | generation.rs | PR #873 omitted TLS `GlobalAddr` roots whose uses are all loads when `supports_global_addr_fold()` — true on i686 non-PIC, whose backend had no direct-load hook: `-m32 -O1+ -fno-pic` LE/IE int loads read an UNWRITTEN register (`movl (%edx)` with %edx never set) | upstream | FIXED: capability `supports_tls_direct_load_fold()` (+ per-type `tls_direct_load_type_ok`) gates omission; i686 now implements the hook (below) |
| UP-TLS-SEG-ROOT | generation.rs | the omission ignored `seg_override`: `*(int __seg_fs *)&tls` (IR forwards the no-op cast) went down the segment path, which reads the omitted root's register — x86-64 `movslq %fs:(%rdi)` / i686 `movl %fs:(%edx)`, unwritten | upstream | FIXED: only default-address-space accesses qualify; probe now materializes the root |
| UP-TLS-ROOT-RA | x86/i686 prologue | omitted TLS roots still got allocator homes (never written): i686 `g` pushed %esi/%edi for nothing | upstream | FIXED: one shared `compute_omitted_tls_roots` runs BEFORE the prologue (`CodegenState::omitted_tls_roots`); both prologues deny those values registers and slots, generation reads the same set — allocation and emission agree by construction |
| I686-TLS-FOLD | i686 codegen | no direct %gs: TLS access: LE/IE loads and LE stores went through `movl %gs:0; addl $x@NTPOFF` (+ a register) per access | missing | FIXED: `movX %gs:x@NTPOFF` / `movl x@INDNTPOFF|x@GOTNTPOFF(%ebx),%r; movX %gs:(%r),%r` loads and `movX $imm|%r, %gs:x@NTPOFF` stores (scalar store path shared with the absolute fold: byte staging via %eax), only for omitted roots (a materialized root's `movl (%root)` is already 1 insn; the IE form would re-read the GOT). Bytes = GAS; `main` = GCC 16.2 (`movl $3,%gs:s@NTPOFF; movb $-56,%gs:sb@NTPOFF`); gate runtime leg PIE + non-PIC |
| X86-TLS-LE-FUSE | x86 peephole load_op_fuse | `movl %fs:s@TPOFF,%eax; addl %eax,%edi` not fused (the IE `%fs:(%rax)` twin was): `is_plain_memory_operand` demands `(…)` | missing | FIXED: `is_fusible_memory_operand` adds the exact base-less TLS shape for the two verbatim-reuse parsers only (movbe pairing derives displacements, keeps the plain predicate); window alias safety unchanged (`:` = memory). `a+s`, `a<s`, `(a&s)^e` = GCC 16.2 instruction counts; unit tests incl. intervening-store negative |
| DRV-WNO-DROPPED | driver/cli.rs | `-Wno-<kind>` / `-Wno-error=<kind>` never reached the warning config (`!flag.starts_with("no-") && process_flag` short-circuit): `-Wall -Wno-return-type -Wno-cpp` printed 2 warnings (gcc 0) | pre-existing | FIXED: config first, unknown-name diagnostic skipped only for `no-`; CLI unit test + gate legs |
| DRV-WNOERROR-GLOBAL | common/error.rs | `-Werror -Wno-error=cpp` stayed an error (`werror_all || errors.contains`) | pre-existing | FIXED: per-kind `not_errors` set; GCC 16.2 measured in all 4 orders (per-kind request beats global -Werror; last per-kind wins; -Werror=X re-enables) |

## x86 LEA width / composition (2026-10-20, S81)

| ID | Where | Finding | Origin | Status |
|---|---|---|---|---|
| X86-LEA-WIDTH | isel.rs / machinst | MachInst `Lea` carried no width: i32 `x*3/5/9` emitted `leaq` (forces 64-bit result spelling, blocks `leal`-aware folds) | pre-existing | FIXED: `Lea.size` = operation width (S8/S16 → S32), `leal`/`leaq` emission; `mul_by_lea_scale_carries_operation_width`, `lea_golden_shapes_by_width` |
| X86-LEA-ADDR32 | emit.rs imm-mul | text path spelled `leal (%r11d,%r11d,s)` / `leal (%eax,%eax,s)`: 32-bit ADDRESS registers = addr32 0x67 prefix (encoder test: `leal (,%ecx,1)` → `67 8d 11`); result identical with 64-bit address regs (GCC: `leal (%rdi,%rdi,2),%eax`) | pre-existing | FIXED: 64-bit address names; `g6` probe `leal (%rsi,%rsi,2),%eax` |
| X86-LEA-ADD | flag_peepholes "1a" | `leal d(%B,%I,s),%T; addl $k,%T` not folded | missing | FIXED: `fold_lea_add_into_lea` (dead flags, width-matched; `leal`+`addq` refused); 8 tests in `lea_add_tests` |
| X86-LEA-RELAY | relay_and_lea | copy relays across self-addressed LEA not removed; `reads_64` treated `leal` as a 64-bit read | missing | FIXED: `reads_64 = !leal` (sound only for `leal`: reads low 32 bits); `fam_written_only_as_dest` LEA special case (`is_read_modify_write` contract unchanged for its 14 callers); `compose_displaced_relay`; consumer-address-write relaxation in `fold_lea_into_load` (ModRM computed before the write; `%rsp`/stack ops/self-base excluded) |
| OWN-LEA-MEM-SIB | local_patterns `fold_lea_into_memory_op` | REGRESSION in my in-flight work (caught by `vectorize_map_i32_scale_add`, rc=1): the LEA admission in `fam_written_only_as_dest` let a consumer naming `%T` twice into the one-slot splice: `leaq -3(%rdi),%r10; leaq (%r10,%r10,4),%r10` → `leaq -3(%rdi,%r10,4),%r10` (stale index). 32-bit twin `leal (%r10,%r10,4),%r10d` reachable only via X86-LEA-WIDTH. Baseline d541951a0 built and checked: NOT a pre-existing bug (isel emitted `leaq` consumers, rejected by the textual RMW check; regression program passes on baseline) | own (in flight, never committed) | FIXED: `source_family_mentions(next,T)==1` on both arms' redefining path; tests `lea_mem_redefining_consumer_tests` + corpus `regress_lea_self_scaled_after_lea_producer.c` (O0–O3 = GCC 16.2, memcheck-clean) |
| X86-LEA-SELFSCALE | local_patterns | the double-mention shape is composable exactly: `lea d(%B),%T; lea d2(%T,%T,s),%D` = `lea d(1+s)+d2(%B,%B,s),%D`. leaq→leaq mod 2^64, leaq→leal and leal→leal mod 2^32; leal→leaq REFUSED (zext between add and mul); `%rsp` base, addr32 spellings, disp32 overflow refused; T dead after or redefined | missing | FIXED: `compose_self_scaled_lea`; `vectorize_map_i32_scale_add` hot loop 3 LEAs → `leaq -8(%rdi,%rdi,4),%r8`; `(i-3)*5` loop `leal -15(%r8,%r8,4),%ebx` (= GCC's single LEA); 6 tests |
| X86-CALLRES-RELAY | relay_and_lea `coalesce_copy_into_rmw` | call results relay through `%r11` (the RA never homes values in `%rax`): `movq %rax,%r11; addq …,%r11; movq %r11,%rax` survived | missing | FIXED: coalesce into the RMW on `%rax`; 3 tests |
| X86-FRAME-SLOP | prologue.rs reserve / `fpo_frame_size` | FPO frames over-reserved: reserve `n*8+8` and alignment rounding left 8–16 dead bytes per frame | pre-existing | FIXED: `fpo_frame_size(raw,has_calls)=((raw+7)&!15)+8` (frame ≡ 8 mod 16 after pushes), reserve `n*8`; `frst.c` 12/12 (O0–O3 × default/no-pie/FP) under memcheck vs GCC reference; unit tests |
| CORPUS-SKIPS | tests/regression | 4 tests silently skipped GCC comparison: `vex_move_false_dep` (needs `-lm`), `jump_table_fno_pie` (`-no-pie`), `bmi2_bzhi` (`-mbmi2`), `builtin_apply_return` (prototype mismatch GCC rejects) | test-infra | FIXED: per-test `.flags` (replaces `-O2`, restores the GCC oracle), casts matching GCC's `__builtin_apply` prototype; remaining 1 skip = C11 6.7.9p11-violating union init (GCC rejects), now self-checking |
| X86-ARG-DEAD-ZEXT | emit.rs ParamRef / prologue | `u3`: U32 param re-zero-extended (`movl %edi,%edi`) though the home is only read at 32 bits | pre-existing | OPEN-ANALYSED: needs a U32 twin of `signed_i32_home_requirements` with Copy/Phi propagation and a consumer audit (`value_to_reg` reads homes with full `movq`) |

Validation (current tree vs baseline d541951a0): unit 479/479 (filters lea_mem_redefining relay_and_lea lea_add_tests local_patterns isel machinst); corpus 1066/0/1-skip-compare (GCC rejects that C11 6.7.9p11 test; it self-checks); codegen gate 11/11 (sqlite_varint 254→253 insns); frst 12/12 under memcheck; Callgrind -O2 31 workloads: 0 regressions, 6 improved (histogram 0.953, binary_search 0.986, double_reduction 0.9925, sqlite_varint 0.9983, glibc_memcmp 0.9998, +1 sub-0.00001), geomean 0.99769 simulated Ir (not a measured speedup). Superseded by the whole-series Callgrind A/B (31 programs vs d8668ee24, geomean 0.98429) in `engineering/FINDINGS-2026-10-20-homing.md`.

## Rebase onto d8668ee24 (#882) and follow-up (2026-10-20)

| ID | Where | Finding | Status |
|---|---|---|---|
| A64-LD-DT-INIT, X86/I686-LD-DT-INIT-NOSTARTFILES | lccc-ld | DT_INIT/DT_FINI written without `_init`/`_fini`, or missing with them | FIXED: tags exist iff the symbols are defined (GNU ld). `check_builtin_dso_link.sh` initfini legs on 4 targets |
| DRV-NOSTARTFILES | driver | `-nostartfiles`/`-nodefaultlibs` not forwarded to the builtin linker | FIXED: no crt objects / no default libraries (unresolved `puts` fails as with GCC). Same gate |
| X86/A64-LD-DEFAULT-LIB-GUESS | lccc-ld | default libraries guessed from undefined symbols | FIXED: only the driver's explicit list. Same gate |
| A64-LD-WEAK-UNDEF-BINDING | lccc-ld | import binding taken from the provider (glibc's weak `getc`) instead of the references | FIXED: STB_WEAK iff every reference is weak. Same gate (import-binding leg) |
| LD-SYMVER-MISSING | lccc-ld x86/RISC-V | executables and DSOs lacked VERNEED for versioned imports | FIXED: `linker_common/symver.rs`, 3 unit tests. Coverage: x86 exe/so, i686 exe, RISC-V exe/so |
| LD-SYMVER-INDEX-OVERFLOW-PANIC | `symver.rs` | version index past the 15-bit field panicked | FIXED: link error (GNU ld refuses too); `VerneedPlan::new` returns Result |
| RV-LD-BIND-FIRST-WINS, RV-LD-DEFSYM-PLT | RISC-V lccc-ld | `--defsym` alias kept libc's JUMP_SLOT although the target is object-defined | FIXED: defsym evaluated before PLT planning. `check_builtin_dso_link.sh` defsym leg |
| MATH-ERRNO-SQRT | `simplify.rs` `split_errno_sqrt` | `sqrt` lost `errno` under the default `-fmath-errno`, or paid a full call | FIXED: inline `sqrtsd` with a never-taken `call sqrt` slow path (GCC's shape). Regression `math_errno_sqrt_slow_path`. 24.77M -> 16.58M Ir on its microbenchmark |
| RA-XMM-CALL-SPANNING-SPILL | `regalloc.rs` | see the homing ledger | FIXED |
| X86-FP-NE-BRANCH-UNFUSED, X86-FP-EQNE-SELECT-MATERIALIZED | x86 codegen | FP `!=`/`==` branch and select materialised setcc pairs | FIXED: fused `ucomisd`+`jp`/`jne` |
| X86-JCC-INVERT-SILENT-DEFAULT | `comparison.rs` `invert_jcc` | an unknown condition fell through to a default, which branches the wrong way | FIXED: closed match over every `cmp_jcc` condition; the remaining arm is `unreachable!` because a fallback would be a silent miscompile |
| PEEP-RELAY-RBP-GPR, PEEP-NCF-RBP-GPR | `relay_and_lea.rs`, `narrow_copy_fold.rs` | rbp copies never folded in frameless functions, where rbp is an allocatable GPR | FIXED: folded only on `rbp_is_gpr_in_function` evidence; rsp never. 3 + 5 unit tests, mutation 4/4. zstd 101.60M -> 97.41M |
| PEEP-LIVE-VMOVQ-GP-WRITE | `helpers.rs` `is_read_modify_write` | `vmovq %xmm,%rax`, `vpextr*`, `cvt*2si` and friends were phantom reads of their GPR destination; liveness kept %rax live across adler32's loop nest | FIXED: pure writes. Orthogonal to #882's implicit-operand model (explicit operands only) |
| REDEXT-CMOV32-UPPER | `redundant_ext.rs` | a 32-bit `cmov` zero-extends even when not taken; `ends_with('l')` misread `cmovl` as a width | FIXED, re-applied over #882's re-carried version. Unit test `cmov_upper32_zero_facts_follow_the_architecture` |
| PEEP-DEAD-INPLACE-NARROW-EXT, X86-NARROW-STORE-RAX-STAGING | peephole | in-place narrow extensions feeding only a narrow store survived | FIXED: `eliminate_dead_inplace_ext`, `narrow_copy_fold_tests.rs` |
| LIVE-SEG-ENVELOPE-OVERAPPROX, LIVE-PHI-COPY-WHOLE-FN, LIVE-FOLDED-READ-FIRST-DEF, LIVE-GEP-BASE-COPY-CHAIN | `liveness.rs` | intervals longer than the value's real live range | FIXED: see the homing ledger; 3 liveness unit tests |
| COLD-SPLIT-DEAD-SAVE | `regalloc.rs` | a cold-call save restored over the call's own result | FIXED: never restore over the result register. Unit test `cold_call_xmm_saves_skip_the_result_register` |
| RA-CAST-HINT | regalloc | steering a cast's home toward its source | REJECTED by measurement (hint + rehome 385.87M vs 356.6M on adler32); removed |
| TLS-PRED-DEDUP | TLS policy | duplicated TLS model predicates | FIXED: one `common::tls::model_table` |
| AUDIT-CENSUS-PANIC-SITES | census | this series added 8 panic sites | FIXED: 7 made fallible; `invert_jcc` kept (above). Knob literals re-baselined with justification |
| GATE-SHIM-VACUOUS | `check_tls_dynamic_witness.sh`, `check_cfi_unwind.sh` | with lccc-ld unbuilt, gcc silently used bfd for every lccc-ld leg, which then passed | FIXED: legs run only when `gcc -B$shim -print-prog-name=ld` resolves the shim, else SKIP |
| LD-NEEDED-ORDER | lccc-ld | base wrote DT_NEEDED with libc before user libraries, reversing GNU ld's search order (wrong interposition) | FIXED: command-line order, identical to ld 2.47 `--no-as-needed`. Default `--as-needed` deliberately NOT adopted: FSF GCC, Clang and GNU ld default to no-as-needed; the oracle gcc-16 injects it from a distro spec (`%{!fsanitize=*:--as-needed}`). `-Wl,--as-needed` is honoured (verified: unused libm dropped, used libm kept, `--no-as-needed` scoping) |

### Upstream #882 verdicts

- `ExtFacts::clear_implicit` over `classify_implicit_operands`: **AGREE**.
  One model instead of two tables; the write mask is exact for every form
  the emitter produces. Nit: the fast path returns no implicit operands for
  `push`/`pop`/`call`/`ret` although they move %rsp, while `pushf`/`enter`
  model it. Harmless (every consumer excludes %rsp and handles call
  clobbers separately), but the "exact" doc claim is overstated for %rsp.
- `is_frame_pointer` removal on redundant_ext's same-value rewrites:
  **AGREE**. Copy deletion/redirect sites need more than that, so they take
  `rbp_is_gpr_in_function` evidence (PEEP-NCF-RBP-GPR).
- const_eval consolidation and `is_unsigned` for the 2^32 reduction:
  **AGREE**. Differential check vs GCC 16.2: 21 negate/bitnot forms
  (unsigned, enum, sub-int, size_t-widened, _Bool), each folded statically
  (sema) and at run time (lowering), at -O0 and -O2: identical.
- `trim_start` instead of consulting the model first in
  `has_implicit_reg_usage`: **AGREE** (+5.1% compile time for no semantic
  change is a bad trade).

## Rebase onto b5a6b6b9c (#884 TLS models, #885 bit idioms) — red-team (2026-10-20)

### #885 verdicts

| Change | Verdict | Reason |
|---|---|---|
| `div_by_const_exact` (SDiv/SRem by 2^k with proven trailing zeros) | AGREE | Lower-bound transfer functions are sound for every op (Shl +n, shifts −n, Add/Sub/Or/Xor min, And max, Mul sum + parity rule); 2^(w−1) excluded; fails closed on non-SSA input. |
| Default-target ordered reciprocal SIMD gate (`ymm && sse41`) | AGREE | `ymm` is the AVX2 permission and honours explicit denials; the integer-load rejection only tightens. |
| `bit_idioms_demand` | AGREE | Demanded-bit transfers checked one by one: And-by-constant identity, URem by c with tz(c) above the demanded closure, Mul-by-constant `closure >> tz(c)`, Shl/LShr/AShr (sign bit when a top-k result bit is demanded), shift amounts fully demanded; zero replacement only inside fresh private nodes, never under a divisor or shift amount. |
| `return_bool` peephole | AGREE + precision fix | The rewrite drops the write of the `setcc` temporary, so it must be dead at `ret`. `%dl` is admitted only under the prologue's `# LCCC_RET_RDX 0` (bounded by `.cfi_endproc`); `%rdx` is the high half of `__int128` / `[INTEGER, INTEGER]` returns. Unit test `dl_requires_proof_that_rdx_is_not_returned`. |

### #884 findings

1. **RISC-V GD stale accumulator (upstream miscompile, fixed).** The GD path with an
   allocated destination never invalidated the register cache after
   `call __tls_get_addr`; the next use of a value cached in `t0` skipped its reload.
   Runtime witness: `gd_acc.c` `w2` under `LD_BIND_NOT=1` (the lazy PLT0 stub loads the
   resolver arguments through `t0`; glibc's bound fast path happens to preserve it)
   stored the link-map pointer into `ext_tls` on origin/main. `check_tls_cross_models.sh`
   now pins it twice: a linker-free sweep (no `t0` read after the resolver before a
   write) and the qemu runtime check. Both fail on origin/main and pass here.
2. **#884 gate fixture never linked.** `wide.c`/`fp.c` referenced `ext_tls`, which no
   provider defines (`provider.c` defines `external_tls`), so the AArch64/RISC-V
   runtime half could not link wherever cross tools exist (on AArch64 the undefined
   TLSDESC reference also crashed BFD 2.44 at `elfnn-aarch64.c:5329`). The fixtures now
   use the shared provider's symbol; the wide-type runtime check runs on both arches.
3. **TLS call sequences inverted the allocator's priorities (my earlier fix, corrected).**
   Making a dynamic TLS address an allocator call point is the right *eligibility*
   contract: the AArch64 text peephole models the TLSDESC `blr` as a full call
   (`eliminate_dead_call_staging_moves`, the l.4162 death scan, text liveness), so no
   caller-saved home may span it. But it also promoted every value spanning the access
   into Phase 1 ahead of the parameters, which spilled (AArch64 `press`: 129 → 162
   instructions; RISC-V GD: 300 → 332). `LivenessResult::sequence_call_points` now
   separates the two: eligibility keeps the full `call_points`/`call_spanning`; Phase 1
   promotion, leaf detection and Phase 2c admission use the real calls only. Unit
   tests: the allocation is identical with and without sequences when only callee-saved
   registers are offered, and no caller-saved home spans a sequence when they are.
   Result: AArch64 `press` 112 (base 129), RISC-V 300 (= base).

### Allocator: AArch64 ran the i686 hazard phases (pre-existing since 335fcf88c)

The i686 phases (Phase 2's ecx/edx scratch-hazard split, 2d, 2e, 2g, 2h) keyed on
"the caller-saved pool contains PhysReg 4 or 5". AArch64's pool `{x4..x8, x13, x14}`
matches, so AArch64 Phase 2 only ever offered x4/x5 under i686 `%ecx`/`%edx` hazard rules
(nearly every point is a hazard), and Phase 2e's "%eax home" was x6. `i686_scratch_pool`
now requires `e_machine == EM_386`. Opening the generic Phase 2 exposed three AArch64
hazards the accidental restriction had masked, all fixed before enabling it:

| Hazard | Fix |
|---|---|
| Call staging writes `ARM_TMP_REGS[k]` (x9, x10, ...) in argument order, and the generic Phase 2 routes argument-consumed values to the pool minus `call_arg_regs`, i.e. exactly {x13, x14}: a sixth argument homed in x13 would read the fifth | x13/x14 join AArch64 `call_arg_regs` |
| Atomic RMW/cmpxchg loops use `w4` as the `stxr` status register | `arm_pool_scratch` removes x4 |
| `va_arg`/`va_start` use x4-x6, `Memcpy` uses x13 (`ldp x12, x13`) | removed per function |

`arm_pool_scratch` is an allowlist in the style of RISC-V's `riscv_call_free_ir`: an
instruction not proven scratch-free (dynamic stack, inline asm, non-local control, PGO
and libcall-backed helpers) closes the whole pool.

Measured on the 62 benchmark programs at `-O2` (AArch64, origin/main → this series):
instructions 16421 → 14993 (−8.7%), spill reloads 986 → 544 (−45%), spill stores
796 → 465 (−42%), callee-saved `stp` 501 → 295 (−41%). x86-64 and i686 output is
byte-identical (128/128) before and after the change.

### Entry-copy coalescing broke the callee-saved contract (pre-existing, AArch64 + RISC-V)

Found by the AArch64 runtime oracle once the generic Phase 2 produced new homes:
`global_addr_pressure.c -O2` printed a wrong total because `mix` returned with x21
holding x20's saved value (`ldp x19, x21, [sp, #16]; ldp x1, x23, [sp, #32]`). The pass
renames `dst` to `src` on every later line, epilogue restores included, and had three
holes:

| Hole | Fix |
|---|---|
| Rules 2/3 asked `gp_operands` "does this line write R?", but its def set is a liveness kill set (must-def): `ldp` destinations, `ldr x0, [x1]`, writeback bases and store-exclusive status registers were all invisible, so the epilogue guard never fired for pair restores | new `may_write_gp` over-approximation for every clobber question; the must-def stays the kill set |
| Rule 3's return model read x0-x7 but not x19-x28 | `ret` reads the AAPCS64 callee-saved set |
| A callee-saved `dst` written earlier in the entry run loses the caller's value when its restore is renamed, even into a dead scratch `src` no liveness can protect | Rule 4 (AArch64 and the RISC-V port, where `mv s1, a0; mv s2, s1; mv s1, a1` returned a0 in s1); a line writing both `dst` and `src` (`ldp x21, x21`) is rejected |

Both ports, and copy propagation, also renamed register-named symbols
(`:got_lo12:x20`, `lga t0, a1`, `bl w2`, `call a1`): C identifiers may spell registers.
`peephole_common::rename_register_operand` / `replace_source_reg_in_instruction` now
skip relocation operands, direct branch/call targets and the symbol operand of
address-forming pseudo-ops and literal loads (x86 is immune: AT&T registers carry `%`).
Every new test fails under its targeted mutation (one fixture was vacuous at first: an
earlier *copy* into `dst` gets coalesced on the next round and hides the defect, so it
now uses a computation). Cost of the fixes on the 62 programs: 14993 → 14998 AArch64
instructions.
