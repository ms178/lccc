# Kernel-priority audit: source evidence, not a speedup claim

Scope: four selected, unmodified upstream Linux **v6.18.58** C files, fetched
through the GitHub API from `gregkh/linux`. The manifest records verified Git
blob identities and SHA256. No kernel code was compiled with GCC or Clang.
This is NOT the complete kernel, patched-package coverage, workload profiling,
or proof that the matched code is hot. Counts include inactive configurations.

The existing lexical inventory library found **223 loops, 22 bulk-memory calls,
11 atomic operations, 7 barriers, 100 locking operations, 53 branch hints,
8 bit-operation calls and 12 single-access operations**. Raw records, including
source line samples, are retained. The CLI correctly refused this partial
source collection as a complete kernel tree; the report explicitly uses the
library's selected-file inventory and is labeled partial.

## Priorities and alternatives to measure

| Workload / location | Candidate | Mandatory controls / rejected shortcut |
| --- | --- | --- |
| AMDGPU `amdgpu_ring_insert_nop`, `amdgpu_ring.c:132-149` | Two wrap-split 32-bit fills; min/count arithmetic; write-pointer masks | Compare actual scalar loops/string-store lowering by fill length and alignment. A 32-bit NOP is not generally a repeated byte: replacing it with byte memset is wrong. Do not recognize an arbitrary callee by basename or assume stores are ordinary cacheable memory. |
| X540/ixgbe `ixgbe_clean_tx_irq`, `ixgbe_main.c:1350-1450` | Descriptor addressing, increment/wrap branches, live-across-call spills | Preserve `smp_rmb`, descriptor-completion reads, DMA unmap/free sequencing, budget and zero-work exits. Rare ring wrap does NOT justify unconditional cmov; compare branch and predicated forms under measured wrap rates. No speculative loads beyond the ring. |
| AMDGPU VM `amdgpu_vm.c` | Page/address arithmetic and range checks around pointer traversals | Separate unsigned-wrap semantics from proven bounds. Preserve locking, refcounts and callback effects. Pointer-chasing is not a license for vectorization or prefetch beyond known valid objects. |
| `lib/bitmap.c:229-354` | Scalar Boolean/reduction loops, population counts, masked tails | Preserve target feature legality and tail masks. Test 0/1/63/64/65/127/128/129-bit boundaries, aliasing and unaligned storage. A lower static instruction count alone is not a runtime win. |

## Hardware and measurement limits

The requested i7-14700KF is not the measurement host. The sandbox reports an
Intel Xeon, family 6/model 106. Neither the NZXT Z690 N5 nor an X540 NIC or AMD
GPU is exposed here. No board-, NIC-, GPU- or 14700KF-specific speedup is claimed.
Hardware counters/latency/throughput and P/E-core scheduling require an actual
hardware run. The AMD GPU model and operating workload also affect priorities.

Ordinary x86 kernel compilation disallows unrestricted SIMD/FPU use. The
14700KF supports AVX2 but does not provide AVX-512 as an available target ISA;
source-level vectorization is not permission to introduce vector instructions
into interrupt or non-FPU-safe kernel paths. Explicit FPU-safe code and
non-kernel benchmarks need separate feature/state gates. AMDGPU host-driver C
optimization does not imply that LCCC is generating AMD GPU device ISA.

Before any code-generation change: establish a failing emitted-code/correctness
witness, compare multiple implementations, preserve MMIO/volatile/atomic/barrier
contracts, run architecture-specific negative controls and mutation tests, then
use the canonical paired benchmark runner with output/exit-code validation.
Use exact GCC16.2/Clang23.1.4/authenticated ICX2026.2 only on permitted isolated
non-kernel comparison fixtures. Kernel target objects remain LCCC-only. The gzip
`longest_match` stack-memory veto remains mandatory after codegen changes.

No backend optimization is justified solely by this lexical audit. Runtime and
code-size measurements for these candidates remain OPEN; no best-in-class or
universally optimal implementation claim is made.
