/* Jump tables under -fno-pie read their targets through an absolute
 * R_X86_64_32S displacement against a local .rodata label:
 * `jmpq *.Ljt(,%rax,8)`.  The assembler must fold that local label to
 * section-symbol + offset exactly as GAS does — a revision that
 * misclassified reloc 11 (32S) as the GOT-slot class @GOT64 skipped the
 * fold, the object writer resolved the unknown `.L` symbol to index 0,
 * and the table was read from absolute address 0: every -fno-pie switch
 * in the corpus SIGSEGVed (switch_dispatch, sqlite_varint,
 * expat_siphash24, zlib_ng_adler32_combine) at every -O level.
 *
 * The dispatcher is big enough (16 dense cases) that every backend tier
 * picks a table over a compare chain, and the checksum pins the case
 * bodies so a silently-wrong target cannot pass.
 */
static int dispatch(int op, int a, int b) {
    switch (op) {
        case 0: return a + b;
        case 1: return a - b;
        case 2: return a * b;
        case 3: return a ^ b;
        case 4: return a | b;
        case 5: return a & b;
        case 6: return a << (b & 7);
        case 7: return a >> (b & 7);
        case 8: return ~a + b;
        case 9: return a - ~b;
        case 10: return (a + b) * 3;
        case 11: return (a - b) * 5;
        case 12: return (a ^ b) + 1;
        case 13: return (a | b) - 1;
        case 14: return (a & b) + 2;
        case 15: return a + b + 1;
        default: return 0;
    }
}

int main(void) {
    long sum = 0;
    unsigned int seed = 777;
    for (int i = 0; i < 4096; i++) {
        seed = seed * 1664525u + 1013904223u;
        int op = (seed >> 16) & 15;
        sum += dispatch(op, i, (int)(seed & 0xffff));
    }
    /* Expected value computed with GCC 16.2 -O2 (and identical under
     * lccc once the fold is correct): any wrong jump target perturbs
     * the LCG-driven mix and misses it. */
    return sum == 16170737881L ? 0 : 1;
}
