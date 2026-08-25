/* i686 natural-width spill slots and Copy-width propagation.
 *
 * i686 used to give every scalar SSA temporary an 8-byte spill slot even
 * though the backend accesses I8/I16/I32/U8/U16/U32/Ptr/F32 values with at
 * most one 32-bit word.  Enabling natural 4-byte slots exposed a latent Copy
 * bug: IR integer constants use an IrConst::I64 container even for semantic
 * U32 values.  emit_copy_value treated the container type as semantic width
 * and wrote a zero high word, corrupting the adjacent 4-byte slot.
 *
 * Keep both 64-bit and 32-bit loop-carried phi webs live so the regression
 * exercises fixed-point Copy-width inference, exact-width slot partitioning,
 * and the I64-container/U32-destination distinction.  This test is libc-free
 * and exits through the i386 ABI, so it runs on any x86-64 Linux host with an
 * ELF32-capable kernel and needs no 32-bit userspace loader.
 */
typedef unsigned int u32;
typedef unsigned long long u64;

__attribute__((noinline))
static int check_mixed_phi_slots(void)
{
    u64 wide = 0x1122334455667788ULL;
    u32 narrow = 0xAABBCCDDU;
    u64 wide2 = 0x8877665544332211ULL;
    u32 narrow2 = 0x1E2D3C4BU;

    for (u32 i = 0; i < 32U; ++i) {
        narrow ^= i;
        wide += narrow;
        narrow2 += i * 7U;
        wide2 ^= narrow2;
    }

    return wide == 0x11223359ace01248ULL
        && wide2 == 0x8877665544332cf1ULL
        && narrow == 0xAABBCCDDU
        && narrow2 == 0x1E2D49DBU;
}

__attribute__((noreturn))
void _start(void)
{
    int status = check_mixed_phi_slots() ? 0 : 1;
    __asm__ volatile ("int $0x80" : : "a"(1), "b"(status) : "memory");
    __builtin_unreachable();
}
