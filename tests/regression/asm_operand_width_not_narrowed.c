/* Regression: the `narrow` pass must never change the width of a value that
 * an `asm` operand names.
 *
 * ── The contract ─────────────────────────────────────────────────────────
 * `Instruction::InlineAsm` carries `operand_types`, recorded by the front end
 * from the C expression's type.  The backend sizes each operand's home — and
 * picks the move width that loads it into the constrained register — from
 * that declared type.  Nothing re-derives it later.  A value named by an asm
 * operand therefore has a width that is part of an opaque ABI contract with
 * hand-written assembly, not an implementation detail the optimizer may pick.
 *
 * ── The defect ───────────────────────────────────────────────────────────
 * `narrow`'s Phase 5 rewrites a single-use 64-bit bitwise BinOp to 32 bits
 * when both operands are provably sub-64-bit.  It counted the asm as an
 * ordinary use, so `n & 7` below — used exactly once, as the "g" input %4 —
 * was narrowed to I32.  That changed only the STORE; the asm's own load still
 * used the declared 64-bit width:
 *
 *     movl %eax, 28(%rsp)     <- narrowed producer writes 4 bytes
 *     ...
 *     movq 28(%rsp), %rdx     <- asm operand reads 8 bytes
 *
 * The upper four bytes were unrelated stack contents, so `%rdx` — and hence
 * `%rcx` for the trailing `rep movsb` — held a garbage count near 2^32.
 *
 * ── How it manifested ────────────────────────────────────────────────────
 * This is the exact shape of `arch/x86/boot/compressed/string.c`'s
 * `____memcpy` in linux-cachymod 6.18.47.  With lccc the kernel built and
 * linked cleanly but died in the decompression stub:
 *
 *     Decompressing Linux...
 *     ZSTD-compressed data is corrupt
 *      -- System halted
 *
 * (`decompress_kernel -> zstd_decompress_dctx -> handle_zstd_error -> error`,
 * ZSTD error 20 = corruption_detected.)  The compressed payload was proven
 * byte-identical to the source through .incbin, the object, the linked
 * decompressor and the bzImage, and a userspace oracle decompressed it
 * perfectly — because that oracle supplied its OWN memcpy.  The corruption
 * came from the boot stub's memcpy overrunning by ~4 GiB.
 *
 * In userspace the same miscompile is an immediate SIGSEGV, which is what
 * this test detects.
 *
 * Only reproducible at -O2 and above (Phase 5 is an -O2 pass) and only with
 * `narrow` enabled: CCC_DISABLE_PASSES=narrow made it pass, which is how the
 * pass was identified.
 *
 * Expected output: memcpy/memmove OK (fails=0)
 */
#include <stdio.h>
#include <string.h>
typedef unsigned long size_t_;

static void *____memcpy(void *dest, const void *src, unsigned long n)
{
	long d0, d1, d2;
	__asm__ volatile(
		"rep movsq\n\t"
		"movq %4,%%rcx\n\t"
		"rep movsb"
		: "=&c" (d0), "=&D" (d1), "=&S" (d2)
		: "0" (n >> 3), "g" (n & 7), "1" (dest), "2" (src)
		: "memory");
	return dest;
}

static void *my_memmove(void *dest, const void *src, unsigned long n)
{
	unsigned char *d = dest;
	const unsigned char *s = src;
	if (d <= s || (unsigned long)(d - s) >= n)
		return ____memcpy(dest, src, n);
	while (n-- > 0)
		d[n] = s[n];
	return dest;
}

static unsigned char a[8192], b[8192], ref[8192];

int main(void)
{
	unsigned fails = 0;
	for (unsigned n = 0; n <= 1200; n++) {
		for (unsigned i = 0; i < n + 32; i++) { a[i] = (unsigned char)(i * 31 + n); ref[i] = a[i]; }
		memset(b, 0xAA, sizeof b);
		____memcpy(b, a, n);
		if (memcmp(b, ref, n) != 0) { printf("memcpy FAIL n=%u\n", n); if (++fails > 5) return 1; }
		if (b[n] != 0xAA) { printf("memcpy OVERRUN n=%u\n", n); if (++fails > 5) return 1; }
	}
	/* overlapping moves, both directions */
	for (unsigned n = 1; n <= 600; n++) {
		for (unsigned off = 1; off <= 40; off++) {
			for (unsigned i = 0; i < sizeof a; i++) a[i] = (unsigned char)(i * 7 + n);
			memcpy(ref, a, sizeof ref);
			my_memmove(a + off, a, n);
			memmove(ref + off, ref, n);
			if (memcmp(a, ref, sizeof a) != 0) { printf("memmove-fwd FAIL n=%u off=%u\n", n, off); if (++fails > 5) return 1; }

			for (unsigned i = 0; i < sizeof a; i++) a[i] = (unsigned char)(i * 7 + n);
			memcpy(ref, a, sizeof ref);
			my_memmove(a, a + off, n);
			memmove(ref, ref + off, n);
			if (memcmp(a, ref, sizeof a) != 0) { printf("memmove-bwd FAIL n=%u off=%u\n", n, off); if (++fails > 5) return 1; }
		}
	}
	printf("memcpy/memmove OK (fails=%u)\n", fails);
	return fails != 0;
}
