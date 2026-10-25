# SSSE3 / SSE4 semantic red-team follow-up

## Merge context and preserved work

The first observed latest merge was the APX source-model campaign. Main then
merged its revert during this audit. Re-fetch full history before rebasing:
the initial checkout was shallow, and a naïve rebase attempted to replay the
root history. That attempt was aborted; the full-history rebase completed on
the current session branch. No other branch was created or checked out.

The inherited dirty checkout was preserved in a named Git stash and a binary
diff outside the repository before reconciling it with the published branch.
The inherited content was already represented by the published work; do not
apply it blindly over the rebased files or discard the saved copy.

Do not restore the reverted APX source-model campaign wholesale. Its useful
architectural boundary fix is independently justified below. A model agreeing
with GAS does not establish that the modeled instruction exists.

## Architectural sources actually consulted

Consulted instruction descriptions and operation pseudocode in these GitHub
transcriptions (older editions, not claimed to be today's latest PDFs):

* Intel SDM combined revision 092, file
  `RestrictedWTF/grimoire/325462-092-intel-sdm-vol-1-2abcd-3abcd-4.md`, blob
  `3af30e3cac947bfa87314c5925d28cc2001f9015`, text SHA256
  `dd4b665a353f6f4b9987495705e1e07cf1649c52b00f2defabe2bae547e1d5ca`.
* AMD APM combined publication 40332 revision 4.09 (March 2026), file
  `RestrictedWTF/grimoire/40332_4.09_AMD_APM_PUB.md`, blob
  `e45f99647653bd5726f03b6fa7b99a794edc3d5d`, text SHA256
  `bd0119aceaba28849f8ed2cbf3f6045f6e85ba36e8bb4eb936b5430739567eb8`.
  Its Vol.4 instruction entries identify publication 26568 rev3.26.

Read PHSUBW/PHSUBD, PSIGNB/W/D, PMULHRSW, PMULLD and PCMPESTRI/M, especially
the latter's effective-operand-size table. Direct primary PDF acquisition and
independent verification of Intel 093 / AMD 3.27 remain blocked by the network
allowlist. Transcriptions are not authenticated primary PDF bytes. Preserve
that qualification; neither a mirror nor an opcode-catalogue absence is a
substitute for checking an architectural instruction definition.

The existing pinned official Intel XED descriptions corroborate the seven
VEX-only horizontal/sign classes. Importantly, the currently selected XED
catalogue also lacks the **real** EVEX VPMULHRSW forms listed in the Intel
manual. Therefore do not generalize "absent from this catalogue" into "invalid
architecture". Retain VPMULHRSW EVEX and add a positive assembler witness.

## Findings and repairs

1. **Undefined EVEX instruction emission.** Remove seven dispatch arms for
   VPHADDW/D, VPHSUBW/D and VPSIGNB/W/D. Their architectural forms are legacy,
   VEX.128 and VEX.256, not EVEX. Reject ZMM, high-bank and masked requests;
   preserve ordinary XMM/YMM VEX forms. Do not reject real EVEX VPMULHRSW.
2. **Public SSSE3 scalar fallback / signed-overflow paths.** PHSUBD and PSIGND
   wrappers used signed C subtraction/negation that can overflow. PMULLD's
   wrapper used signed C multiplication. Replace those with native operations,
   and finish the native SSSE3 binary family (horizontal subtract, saturating
   horizontal add/subtract, sign, rounded high multiply). Wire builtin/public
   names, arities, IR conversion, 16-byte result/home metadata, promotion,
   register clobber metadata and both x86 backends. Convert PALIGNR's pointer
   result with the same bundled-header helper used by the other operations.
3. **AVX domain inefficiency.** Use legal three-operand VEX versions for this
   x86-64 family when AVX is permitted, rather than inserting legacy operations
   between VEX loads/stores. Without AVX retain legacy SSSE3. No EVEX promotion
   is inferred merely from a mnemonic's initial `v`.
4. **Missing permission checks.** Add the native 128-bit SSSE3 family and the
   existing unambiguous SSE4.1 insert/extract/blend/widen/multiply/test intrinsics
   to the shared permission table. Public/raw aliases share one requirement.
   Add vendor-checked negative and last-option-precedence cases to the existing
   targeted feature gate; do not manufacture a feature guard using CRC32.
   A red-team per-function target probe exposed that invocation-only permission
   is insufficient. Keep normalized requests in per-function lowering state,
   honor local legacy SSE grants/denials and mark header wrappers with their
   actual extension. Positive `target("ssse3")` / `target("sse4.1")` probes
   and an unrelated baseline function prove grants do not leak globally.
   This is legacy 128-bit permission handling, not a claim of complete
   per-function AVX/AVX-512 type/ABI reconfiguration.
5. **Missing 64-bit explicit string lengths.** Support the explicit REX.W
   spelling and its i686 rejection. SDM says REX.W changes EAX/EDX lengths to
   RAX/RDX, while the index remains ECX. Runtime cases use lengths whose low
   32 bits are zero but whose 64-bit magnitudes saturate to 16. Index and mask
   tests distinguish both length registers and compare to architectural answers.
6. **Misplaced explicit REX before F2/F3.** Merge/insert REX after mandatory
   legacy prefixes, not before them, where a later F2/F3 makes it ineffective.
   Reject named AH/BH/CH/DH operands under explicit REX instead of silently
   encoding different low-byte registers. Golden bytes cover CRC and SIMD.
7. **Optimization-dependent vector asm miscompile.** The new length witness
   exposed O0 loading vector ADDRESS-slot bytes rather than vector data from
   an aggregate member. Lower 16-byte x86 vector register lvalues with a typed
   I128 load, preserving volatile/segment metadata. Do the same for read-write
   inputs without evaluating their lvalue twice. Memory constraints retain
   addresses, and non-x86 lowering is unchanged. No unreserved scratch register
   is introduced. Add O0 wrappers alongside normal optimized witnesses.
8. **GAS is not the architectural authority.** GAS2.47 rejects
   `rex64 crc32b %al,%rax` as a duplicate prefix even though a single F2 48
   encoding is legal. Keep the legal SDM-byte Rust golden and REX coalescing;
   do not copy that rejection into our encoder. The GAS differential fixture
   explicitly records this syntax-policy disagreement rather than treating
   GAS acceptance as the only source of legal architectural encodings.
9. **Defensive SSE4a helper arity.** Check immediate-form arity before indexing
   operands, even though the ordinary dispatch already validates it.

## Evidence discipline and remaining scope

The new scalar expectations avoid signed overflow. They include unsigned
modulo subtraction/multiplication, INT_MIN sign negation, saturation extrema,
PMULHRSW's wrapping MIN*MIN result and randomized lanes. PALIGNR witnesses cover
both concatenation halves and zero results beyond 31 bytes. Check direct and
indirect wrapper calls, and native instruction presence independently of values.

Use restored, digest-verified GCC16.2, mold2.42.1 and Rust1.97 MSRV locally;
bootstrap GAS/objdump2.47 from the verified cache. A root-enabled 4 GiB swap file
is active in this fresh sandbox. Warning-denied two-job fastbuild only; no local
CI sweep. Current stable Rust and Clang23.1.4 acceptance remain hosted checks.
Do not count mocked oracle tests as LLVM execution or instruction count as a
wall-time/Callgrind speedup. Hosted CI must finish on the rebased pushed head
before calling it green.

This does not finish every public SSE4.1/SSE4.2 API. In particular the bundled
string-intrinsic header is still incomplete, and a full primary latest-manual
read is not certified. Scalar fallback coverage and optimal allocation for every
SSE4.1 operation remain work, not an unsupported "perfect everything" claim.
