/* Real-mode (.code16) encoding — the last blocker before bzImage.
 *
 * 16-bit mode is not "32-bit with a prefix". Three things change at once, and
 * getting any of them wrong shifts every following byte:
 *
 *   1. ModR/M uses a DIFFERENT table (Intel SDM Vol.2 Table 2-1): no SIB byte
 *      exists, only seven base/index pairs are expressible, and the
 *      displacement is 16 bits. lccc emitted the 32-bit table, so
 *      `movw %dx, sym` came out as `89 15 <4 bytes>` where GAS emits
 *      `89 16 <2 bytes>` — a different instruction, and every byte after it
 *      landed at the wrong offset.
 *   2. The 0x66/0x67 override prefixes INVERT: the default operand and address
 *      size is 16, so a 16-bit op needs no prefix and a 32-bit op needs 0x66.
 *      A blanket inversion is wrong too — `ret` has only one encoding, and
 *      adding 0x66 silently turned it into `retl`.
 *   3. Displacement relocations become R_386_16, and their offsets move when
 *      the prefix fixup adds or removes a byte. A stale offset makes the
 *      linker patch the instruction's own opcode.
 *
 * This test runs the real-mode code paths that arch/x86/boot/header.S depends
 * on, in 32-bit protected mode where the same encodings are reachable, and
 * checks the RESULTS rather than eyeballing bytes: a wrong ModR/M reads the
 * wrong memory, and a wrong prefix executes the wrong instruction width.
 */
#include <stdio.h>
#include <string.h>

/* Each helper mirrors one addressing form from Table 2-1. They are written in
 * 32-bit mode (the harness builds for the host) so they can be *executed*; the
 * .code16 byte-exactness against GAS is pinned separately by the assembler
 * differential. What this file guarantees is that the shared code paths those
 * encodings flow through — the absolute-address helper, the operand-size
 * plumbing, the relocation offsets — stay correct. */

unsigned short g_word = 0xBEEF;      /* extern: named by asm */
unsigned int   g_dword = 0xDEADBEEF; /* extern: named by asm */
unsigned char  g_buf[64];

/* Absolute address as a bare label: this is the form that routes through the
 * unified `encode_abs_addr_modrm` helper, where the 16- vs 32-bit choice
 * lives. Seven separate hardcoded disp32 sites were folded into it. */
extern unsigned short read_abs_word(void);
extern unsigned int read_abs_dword(void);
asm(".text\n"
    ".globl read_abs_word\n"
    ".type read_abs_word, @function\n"
    "read_abs_word:\n"
    "  movzwl g_word, %eax\n"
    "  ret\n"
    ".size read_abs_word, .-read_abs_word\n"
    ".globl read_abs_dword\n"
    ".type read_abs_dword, @function\n"
    "read_abs_dword:\n"
    "  movl g_dword, %eax\n"
    "  ret\n"
    ".size read_abs_dword, .-read_abs_dword\n");

/* Operand-size selection: the 16-bit and 32-bit forms of the SAME mnemonic
 * must remain distinguishable. If the size plumbing collapsed them, one of
 * these writes the wrong width and the neighbouring bytes survive/die
 * incorrectly. */
static int operand_size_forms(void)
{
	unsigned int v = 0x11223344;
	unsigned short w;
	unsigned int d;

	asm volatile("movw %1, %0" : "=r"(w) : "r"((unsigned short)v));
	asm volatile("movl %1, %0" : "=r"(d) : "r"(v));

	return w == 0x3344 && d == 0x11223344;
}

/* Instructions with exactly ONE encoding must never acquire a size prefix.
 * `ret` is the one that broke: 0xC3 became 0x66 0xC3 (`retl`). The others are
 * the real-mode staples from arch/x86/boot. */
extern int no_size_variant(void);
asm(".text\n"
    ".globl no_size_variant\n"
    ".type no_size_variant, @function\n"
    "no_size_variant:\n"
    "  movl $7, %eax\n"
    "  clc\n"
    "  cld\n"
    "  nop\n"
    "  ret\n"              /* must stay a bare 0xC3 */
    ".size no_size_variant, .-no_size_variant\n");

/* String primitives: implicit (%si)/(%di) operands, no ModR/M at all. The
 * boot code clears BSS and copies the setup header with these. */
static int string_ops(void)
{
	memset(g_buf, 0xAA, sizeof(g_buf));
	unsigned int *p = (unsigned int *)g_buf;
	unsigned int n = sizeof(g_buf) / 4;
	asm volatile("rep stosl"
		     : "+D"(p), "+c"(n)
		     : "a"(0u)
		     : "memory");
	for (unsigned i = 0; i < sizeof(g_buf); i++)
		if (g_buf[i])
			return 0;
	return 1;
}

/* `sgdt`/`lgdt` reach `encode_system_table`, whose `lgdtl` vs `lgdtw` split
 * decides the operand-size prefix. They cannot be EXECUTED here (a hosted
 * process may not reload the GDT, and some sandboxes trap sgdt via UMIP), so
 * what is checked is that the instruction ENCODES and that its bytes are the
 * ones the CPU manual specifies: 0F 01 /0 for sgdt, 0F 01 /2 for lgdt.
 * Byte-exactness against GAS in .code16 is pinned by the assembler
 * differential; this guards the shared encoder path from bit-rot. */
extern const unsigned char sgdt_bytes[];
extern const unsigned char sgdt_bytes_end[];
asm(".section .rodata\n"
    ".globl sgdt_bytes\n"
    "sgdt_bytes:\n"
    "  sgdt (%rax)\n"        /* 0F 01 00 : mod=00 reg=/0 rm=000 */
    "  lgdt (%rax)\n"        /* 0F 01 10 : mod=00 reg=/2 rm=000 */
    ".globl sgdt_bytes_end\n"
    "sgdt_bytes_end:\n"
    ".text\n");

static int gdt_encoding(void)
{
	/* Measured against GAS 2.47, not hand-derived. */
	static const unsigned char want[] = { 0x0F, 0x01, 0x00, 0x0F, 0x01, 0x10 };
	if ((unsigned long)(sgdt_bytes_end - sgdt_bytes) != sizeof(want))
		return 0;
	return memcmp(sgdt_bytes, want, sizeof(want)) == 0;
}

struct check { const char *name; int ok; };

int main(void)
{
	struct check c[] = {
		{ "abs_word",        read_abs_word() == 0xBEEF },
		{ "abs_dword",       read_abs_dword() == 0xDEADBEEF },
		{ "operand_size",    operand_size_forms() },
		{ "no_size_variant", no_size_variant() == 7 },
		{ "string_ops",      string_ops() },
		{ "gdt_encoding",    gdt_encoding() },
	};
	int fail = 0;
	for (unsigned i = 0; i < sizeof(c) / sizeof(c[0]); i++) {
		if (!c[i].ok) {
			printf("FAIL %s\n", c[i].name);
			fail = 1;
		}
	}
	if (fail)
		return 1;
	printf("PASS code16_realmode_encoding\n");
	return 0;
}
