/*
 * Freestanding four-ISA runtime oracle: no libc, headers or cross sysroot.
 * Compile THIS translation unit with GCC -m32 on i686, Clang 23's independent
 * cross backend on AArch64/RISC-V 64. Compile the SQLite decoder separately
 * with GCC/Clang/LCCC pass on/off as available. A bad LCCC decoder cannot make
 * its own independent reference answer agree by mistake. Linux _start calls
 * main and exits with its result; exit 1..9 means a varint length failed,
 * 10..14 identifies a guarded Sub-encoded byte-delta failure, 20..24 a
 * GEP/Add-encoded backward-step failure.
 */
typedef unsigned char u8;
typedef unsigned long long u64;
extern u8 sqlite_varint_delta(const u8 *, u64 *);
extern long guarded_byte_delta(const u8 *, int, volatile u8 *);
extern long guarded_negative_add_delta(const u8 *, int, volatile u8 *);

static unsigned state = 0x20ce271c;
static u8 next_byte(void) {
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    return (u8)state;
}

static u8 reference(const u8 *p, u64 *v) {
    u64 result = 0;
    unsigned i;
    for (i=0; i<8; ++i) {
        result = (result<<7) | (p[i]&0x7fU);
        if ((p[i]&0x80U)==0) { *v = result; return (u8)(i+1); }
    }
    *v = (result<<8) | p[8];
    return 9;
}

int main(void) {
    unsigned n, j, k;
    for (n=1; n<=9; ++n) {
        for (j=0; j<10000; ++j) {
            u8 input[16] = {0};
            u64 got = 0, want = 0;
            u8 bytes, expected;
            for (k=0; k<n-1; ++k) input[k] = (next_byte()&0x7fU)|0x80U;
            input[n-1] = next_byte() & (n<9 ? 0x7fU : 0xffU);
            bytes = sqlite_varint_delta(input, &got);
            expected = reference(input, &want);
            if (bytes != expected || (unsigned)bytes != n || got != want) return (int)n;
        }
    }
    /* All pointer expressions remain inside input. No invalid-pointer C
     * arithmetic or assumed target address-space layout is needed. */
    for (j=0; j<10000; ++j) {
        u8 input[64];
        int choice;
        for (k=0; k<64; ++k) input[k] = next_byte();
        for (choice=0; choice<5; ++choice) {
            u8 sub_out = 0, add_out = 0;
            long expected = choice==1 ? -7 : choice==2 ? 11 : 0;
            long sub_delta = guarded_byte_delta(input+32, choice, &sub_out);
            long add_delta = guarded_negative_add_delta(input+32, choice, &add_out);
            if (sub_delta != expected || sub_out != input[32+expected]) return 10+choice;
            if (add_delta != expected || add_out != input[32+expected]) return 20+choice;
        }
    }
    return 0;
}
