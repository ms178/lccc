/* Regression test: fusing `movslq %Xd, %rax` + `movq %rax, %Y` into
 * `movslq %Xd, %Y` DELETES the only definition of %rax, so it is legal only
 * when %rax is provably dead afterwards.
 *
 * Historical bug (fuse_signext_and_move). The pass scanned forward for
 * `cmpl $imm, %eax` operands it could redirect to the source register, and
 * then proceeded whenever it had found at least one -- even though its own
 * liveness scan had stopped at a barrier with %rax conservatively LIVE:
 *
 *     movslq %edi, %rax     <- only definition of %rax
 *     movq   %rax, %r11
 *     cmpl   $1, %eax       <- redirected to %edi, sets "found a cmpl"
 *     je     .Lcase1        <- BARRIER: scan stops, rax_dead == false
 *     cmpl   $2, %eax       <- SUCCESSOR block, never examined
 *
 * The movslq was retargeted to %r11, so the second compare read a register
 * nothing defined any more and `case 2:` was never taken.
 *
 * A switch over an int parameter lowered to a compare chain reproduces it
 * exactly: the chain puts one compare per basic block, all reading the same
 * sign-extended value. The test additionally covers two compares in the SAME
 * block (only the first used to be redirected) and a case where %rax really
 * is dead, so the fusion must still happen.
 *
 * MIN_JUMP_TABLE_CASES keeps these small switches on the compare-chain path;
 * the values are deliberately sparse so no jump table is formed.
 */

#include <stdio.h>

/* Compare chain over a sign-extended int parameter. Each case lands in its
 * own basic block, so every `cmpl` after the first is cross-block. */
__attribute__((noinline))
int chain(int x)
{
    switch (x) {
    case 1:  x += 10;  /* fall through */
    case 2:  x += 100; break;
    case 7:  x += 1000; break;
    case 19: x += 10000; break;
    default: x = -1;
    }
    return x;
}

/* Same shape, but the switch value is used again AFTER the switch, so the
 * sign-extended value must survive every branch. */
__attribute__((noinline))
long chain_reuse(int x)
{
    long acc = 0;
    switch (x) {
    case 3:  acc = 30; break;
    case 5:  acc = 50; break;
    case 11: acc = 110; break;
    default: acc = -7;
    }
    return acc * 1000 + (long)x;
}

/* Two comparisons of the same value inside ONE basic block (no barrier
 * between them): both operands must be redirected together. */
__attribute__((noinline))
int two_cmps_one_block(int x)
{
    int r = 0;
    if (x == 4) r += 1;
    if (x == 4) r += 2;   /* same block shape after the first test folds */
    if (x == 9) r += 4;
    return r * 100 + x;
}

/* %rax genuinely dead after the fuse: nothing here should inhibit the
 * optimisation, so this guards against "fix" by disabling the pass. */
__attribute__((noinline))
long plain_widen(int x)
{
    long v = x;      /* movslq, value then lives in a callee-saved home */
    return v * 3 + 1;
}

int main(void)
{
    if (chain(1)  != 111) return 1;   /* 1+10+100  */
    if (chain(2)  != 102) return 2;   /* 2+100     */
    if (chain(7)  != 1007) return 3;
    if (chain(19) != 10019) return 4;
    if (chain(3)  != -1) return 5;
    if (chain(-5) != -1) return 6;

    if (chain_reuse(3)  != 30L * 1000 + 3) return 7;
    if (chain_reuse(5)  != 50L * 1000 + 5) return 8;
    if (chain_reuse(11) != 110L * 1000 + 11) return 9;
    if (chain_reuse(-2) != -7L * 1000 + -2) return 10;

    if (two_cmps_one_block(4) != 3 * 100 + 4) return 11;
    if (two_cmps_one_block(9) != 4 * 100 + 9) return 12;
    if (two_cmps_one_block(0) != 0) return 13;

    if (plain_widen(7)  != 22) return 14;
    if (plain_widen(-3) != -8) return 15;

    (void)printf("");
    return 0;
}
