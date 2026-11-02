/* `.set alias, %reg` register aliases, including TRANSITIVE chains.
 *
 * The kernel vDSO ChaCha code (arch/x86/entry/vdso/vgetrandom-chacha.S) is
 * written entirely with .set register aliases:
 *     .set state0, %xmm1        (direct)
 * and crc-pclmul-template.S builds two-level chains through .irp:
 *     .set V7, %xmm7
 *     .set CONSTS, V7           (transitive)
 * lccc's expression path treated these as numeric symbols and failed with
 * "unsupported SSE mov operands". Aliases must resolve at DEFINITION time
 * (GAS evaluates .set eagerly): redefining V later must not retroactively
 * change CONSTS.
 *
 * Runtime check: shuffle data through aliased GP and XMM registers and
 * verify the values actually moved through the registers the aliases name.
 */
#include <stdio.h>
#include <string.h>

asm(".set\tSRC,\t%rdi\n"
    ".set\tDST,\t%rax\n"
    ".set\tVIN,\t%xmm1\n"
    ".set\tVOUT,\tVIN\n"          /* transitive: VOUT -> VIN -> %xmm1 */
    ".text\n"
    ".globl alias_roundtrip\n"
    ".type alias_roundtrip, @function\n"
    "alias_roundtrip:\n"
    "\tmovq (SRC), VIN\n"          /* load 8 bytes via aliased xmm */
    "\tmovq VOUT, DST\n"           /* store back via the transitive alias */
    "\tret\n"
    ".size alias_roundtrip, .-alias_roundtrip\n");

long alias_roundtrip(const long *p);

int main(void)
{
       long v = 0x1122334455667788L;
       long got = alias_roundtrip(&v);
       if (got != v) {
               printf("FAIL roundtrip got=%lx\n", got);
               return 1;
       }
       printf("PASS asm_set_register_alias\n");
       return 0;
}
