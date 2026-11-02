# RA validation: zlib-ng, gzip 1.14, Expat (2026-08-20)

**Purpose:** compile real codec/parser C with LCCC vs GCC `-O2`, measure
kernels, and read assembly for register-allocation failures.

**Not** a Raptor Lake PMU result. Host: 2-core VM, ~2 GiB RAM, GCC 14,
LCCC `target/fastbuild` (`opt-level=1` compiler, generated code still `-O2`).
Checksums matched GCC on every kernel. Screening evidence only.

Compiler: `/home/user/lccc/target/fastbuild/lccc`  
Sources: GNU gzip 1.14 (configured), zlib-ng (configured `--zlib-compat`),
Expat 2.7.1 (configured), plus in-tree kernels in
`tests/benchmark/programs/`.

## 1. Kernel runtime (correctness first)

Same source, `-O2`, 7 runs, **median** wall-clock. Outputs byte-identical.

| Kernel | Origin | LCCC | GCC 14 | LCCC/GCC |
|--------|--------|-----:|-------:|---------:|
| `zlib_ng_adler32` | zlib-ng generic Adler-32 | 0.0564 s | 0.0378 s | **1.49×** |
| `gzip_crc32` | gzip `lib/crc.c` table path | 0.2457 s | 0.1675 s | **1.47×** |
| `expat_xml_scan` | Expat UTF-8 name scan | 0.0764 s | 0.0391 s | **1.95×** |

All return codes 0; Adler check `8c331ae0`, CRC `372e56ab`, Expat hash
`626766774715194881`.

**RA is not the only gap** (ISel, inlining, addressing), but the asm
below shows **stack homes in the hottest loops** where GCC uses GPRs.

## 2. Whole-file instruction and stack-mem counts (`-S -O2`)

Stack-mem = instructions with `(%rbp)` / `(%rsp)` addressing.

| TU | LCCC insns | GCC insns | ratio | LCCC stack-mem | GCC stack-mem | ratio |
|----|----------:|----------:|------:|---------------:|--------------:|------:|
| gzip `deflate.c` | 1169 | 742 | 1.58 | 229 | 8 | **29×** |
| gzip `trees.c` | 2199 | 1414 | 1.56 | 434 | 99 | 4.4× |
| gzip `inflate.c` | 2499 | 1126 | 2.22 | 674 | 133 | 5.1× |
| gzip `zip.c` | 818 | 305 | 2.68 | 84 | 2 | 42× |
| gzip `bits.c` | 504 | 215 | 2.34 | 40 | 2 | 20× |
| zlib-ng `adler32.c` | 317 | 105 | 3.02 | 44 | 0 | ∞ |
| zlib-ng `deflate.c` | 4432 | 1935 | 2.29 | 551 | 88 | 6.3× |
| zlib-ng `deflate_fast.c` | 329 | 209 | 1.57 | 81 | 3 | 27× |
| zlib-ng `inflate.c` | 6386 | 2867 | 2.23 | 2644 | 172 | **15×** |
| zlib-ng `trees.c` | 2501 | 1915 | 1.31 | 613 | 44 | 14× |
| zlib-ng `inftrees.c` | 666 | 322 | 2.07 | 201 | 67 | 3.0× |
| Expat `xmlparse.c` | 23086 | 11618 | 1.99 | 4118 | 1369 | 3.0× |
| Expat `xmltok.c` | 26263 | 11700 | 2.24 | 3741 | 320 | **12×** |
| Expat `xmlrole.c` | 3475 | 1840 | 1.89 | 42 | 7 | 6.0× |
| kernel Adler | 361 | 165 | 2.19 | 53 | 7 | 7.6× |
| kernel CRC | 128 | 54 | 2.37 | 2 | 0 | — |
| kernel Expat scan | 318 | 183 | 1.74 | 14 | 1 | 14× |

LCCC **always compiled** these TUs (gzip `gzip.c` failed only on missing
generated `version.h`, same as GCC without `make`). No RA ICE.

## 3. Hottest functions (RA smoking guns)

| Function | File | Δ insns | LCCC stack-mem | GCC stack-mem |
|----------|------|--------:|---------------:|--------------:|
| `inflate` | zlib-ng inflate.c | +2687 | **2589** | 154 |
| `doProlog` | expat xmlparse.c | +1644 | 1077 | 245 |
| `deflate` | zlib-ng deflate.c | +1238 | 437 | 42 |
| `gzip_inflate` | gzip inflate.c | +750 | 310 | 13 |
| `storeAtts` | expat xmlparse.c | +763 | 399 | 176 |
| `normal_prologTok` | expat xmltok.c | +758 | 159 | 37 |
| `little2_prologTok` | expat xmltok.c | +746 | 223 | **0** |
| **`longest_match`** | gzip deflate.c | **+192** | **118** | **0** |
| `pqdownheap` | gzip/zlib trees.c | +179 | 97 | 0 |
| `send_tree` | gzip trees.c | +293 | 308 | 12 |
| `zng_inflate_table` | zlib-ng inftrees.c | +344 | 201 | 67 |

`longest_match` is the canonical RA boss fight from the research plan.
**GCC: zero stack traffic in the function body. LCCC: 248-byte frame,
118 `(%rsp)` ops.**

## 4. `longest_match` (gzip 1.14 `deflate.c`) — RA autopsy

### LCCC (excerpt)

```asm
longest_match:
    pushq %rbx / %r12 / %r13 / %r14 / %r15 / %rbp
    subq $248, %rsp
    movq %rdi, 232(%rsp)
    movq max_chain_length@GOTPCREL(%rip), %rax
    movq %rax, 48(%rsp)
    movl (%rax), %eax
    movq %rax, 224(%rsp)
    movq window@GOTPCREL(%rip), %rax
    movq %rax, 48(%rsp)
    ; ... every global reloaded via GOT, parked on stack ...
```

### GCC (excerpt)

```asm
longest_match:
    movl strstart(%rip), %eax
    movl %edi, %edx
    pushq %r14 / %r12 / %rbp / %rbx
    movl prev_length(%rip), %r8d
    leaq window(%rip), %r9
    movl max_chain_length(%rip), %ecx
    leaq (%r9,%rax), %rbx          ; scan pointer in %rbx
    movzbl -1(%rbx,%rsi), %ebp     ; scan_end1 in %bpl
    movzbl (%rbx,%rsi), %r10d      ; scan_end in %r10b
    leaq 258(%r9,%rax), %r12       ; strend
    ; hot loop: cmpb %r10b, (%rax,%rsi) — no stack
```

### Failures (ordered)

1. **Globals not rematerialized.** `window`, `strstart`, `prev`,
   `max_chain_length`, `prev_length`, `good_match` are file-scope. GCC
   uses `%rip`-relative loads **into GPRs that stay live**. LCCC emits
   `foo@GOTPCREL(%rip)`, stores the **address** to `48(%rsp)`, reloads
   it. That is remat + addressing + RA.

2. **Whole-lifetime stack homes.** Loop-carried `chain_length`,
   `scan`, `match`, `best_len` never get caller-saved regs for the
   inner compare loop. 248 B frame while GCC uses **no locals**.

3. **Callee-saved always saved.** LCCC pushes rbx+r12–r15+rbp even
   when the body then spills to stack anyway (save cost without the
   benefit).

4. **GOT vs small PIC.** GCC `-O2` `leaq window(%rip)`. LCCC GOT
   indirection increases live values (got-address temp) and pressure.

This is exactly backlog items **B (call/global homes), C (remat), D
(second-chance)** in `WEAKNESSES_AND_BACKLOG.md`.

## 5. zlib-ng Adler-32 inner unroll — RA + ISel

LCCC NMAX loop (`5552/8 = 694` trips):

```asm
.LBB18:
    movzbl (%r9), %edx
    movq %r14, %r8
    addl %edx, %r8d
    movzbl 1(%r9), %eax
    movq %rax, %rdi      ; each byte parked in a distinct GPR
    addl %eax, %r8d
    ; ... bytes 2..7 in rsi,rbx,r12,r15,rbp,r13 ...
    movq %r14, %rax
    shll $3, %eax
    movq %rax, 24(%rsp)  ; STACK in the 8-byte body
    ...
    movl 72(%rsp), %edi  ; sum2 reload
    leaq 8(%r9), %r10
    movl 48(%rsp), %esi  ; trip count on stack
    subl $1, %esi
    ...
    movq %rdi, 72(%rsp)
    movq %rsi, 48(%rsp)
    jmp .LBB18
```

GCC keeps `sum1`/`sum2`/pointer/limit in GPRs; 24-byte frame is
outside the inner loop.

**RA reading:** eight live byte temps + two sums + pointer + trip
count ≈ 12 values. x86-64 has enough GPRs **if** bytes are consumed
into the two accumulators immediately. LCCC keeps all eight bytes
live for the `sum2 += 8*b0+7*b1+...` expansion, then **still** spills
`sum2` and `n`. That is:

- missing two-accumulator strength reduction (ISel/algebra), **and**
- eviction that demotes the **loop-carried** sums instead of the
  already-consumed bytes (cost model / next-use).

Kernel time **1.49×** is consistent with extra stack ops in a
cache-hot loop (not L1-miss bound).

## 6. gzip CRC-32 table loop

Kernel **1.47×**. Stack-mem 2 vs 0 (almost clean). Remaining gap is
mostly **indexed addressing / ILP**, not spills: GCC folds
`crc32_table[(crc^b)&255]` more tightly. RA is not the primary
suspect here — ISel/LEA is. Do not “fix RA” expecting CRC to match
Adler/`longest_match`.

## 7. Expat tokenizer (`xmltok.c`, `xmlparse.c`)

- `xmltok.c` stack-mem **12×** GCC. `little2_*` / `big2_*` clones:
  GCC often **0** stack-mem; LCCC ~110–220. Encoding-specialized
  copies are long, branchy, call-light — **ideal linear-scan
  victims**. Fat intervals from diamond `BYTE_TYPE` switches make
  mutually exclusive arms interfere (backlog **A: segment scan**).
- `doProlog` / `doContent`: both compilers save 7 callee-saved; LCCC
  still 2–4× stack-mem → spills of pointer+length pairs across
  `XmlTok` calls (backlog **B: call-site save vs callee-saved**).
- Kernel `expat_utf8_name_length`: LCCC does not fold
  `xml_name_continue` into a compact classify; many `setae`/`movzbl`
  to GPRs. Mix of inlining and RA. Runtime **1.95×**.

## 8. zlib-ng `inflate`

Worst TU: **2644 vs 172** stack-mem, `inflate` itself 2589 vs 154.
Huge state machine, many live `hold`/`bits`/`next`/`put` values
across cases. GCC colors the switch; LCCC’s envelope intervals
make every arm conflict. Segment-aware scan + switch lowering
interaction.

## 9. What this does *not* show

- Full `make check` of gzip/expat/zlib-ng under LCCC (link + tests).
- PGO.
- AVX zlib-ng kernels (we compiled **generic C** with `-DZLIB_COMPAT`).
- ICX/Clang on this VM (no ICX). Next: clang `-O2 -S` on the same TUs.

## 10. Ranked RA work from this measurement

| Rank | Action | Evidence | Expected |
|------|--------|----------|----------|
| 1 | Remat + RIP-relative for file-scope arrays/ints (`window`, `prev`, CRC table) | `longest_match` GOT+stack vs GCC `%rip` | gzip deflate |
| 2 | Keep loop-carried IV/pointer/len in GPR; evict dead byte temps | Adler `72(%rsp)`/`48(%rsp)` in DO8 | Adler 1.49× |
| 3 | Segment/hole-aware coloring of tokenizer/inflate switches | xmltok 12× stack-mem, inflate 15× | expat, zlib inflate |
| 4 | Call-site save of non-spanning holes vs always callee-saved | doProlog 1077 vs 245 | xmlparse |
| 5 | Do **not** chase CRC with RA | 2 vs 0 stack-mem, still 1.47× | ISel |

## 11. Repro

```sh
# kernels
lccc -O2 tests/benchmark/programs/zlib_ng_adler32.c
lccc -O2 tests/benchmark/programs/gzip_crc32.c
lccc -O2 tests/benchmark/programs/expat_xml_scan.c

# gzip TUs after ./configure
lccc -O2 -S -I. -Ilib -DHAVE_CONFIG_H deflate.c

# zlib-ng after ./configure --zlib-compat
lccc -O2 -S -I. -DZLIB_COMPAT deflate.c inflate.c adler32.c

# expat after ./configure
lccc -O2 -S -I. -Ilib -DHAVE_EXPAT_CONFIG_H lib/xmltok.c
```

Artifacts from this run: `/home/user/ra-bench/` (asm + binaries).
Not committed (generated).
