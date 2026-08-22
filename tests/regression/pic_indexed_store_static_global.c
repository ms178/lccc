/* -fPIC indexed store to a static global with an acc-resident value.
 *
 * SQLite speedtest1 (HashUpdate/HashFinal, RC4-style state swap) ICE'd:
 *   x86 codegen: operand_to_rax: value N in function 'speedtest1_final'
 *   has no register home, no stack slot and no acc-cache entry
 *
 * Root cause: `can_indexed_addr_fold` computed the GOT verdict on the
 * symbol *basename* ("gh"), but `emit_store_indexed_sym_impl` /
 * `emit_load_indexed_sym_impl` passed the composed displacement string
 * ("gh+2") to `needs_got_for_addr`, which then missed `local_symbols`
 * under -fPIC and refused the fold the allocator had committed to. The
 * skipped GEP was rematerialised at the store site, clobbering %rax
 * while it held the acc-resident (nohome) store value.
 *
 * The kernel below is the exact byte-swap shape: s[i]=s[j] / s[j]=t with
 * byte-typed wrapping indices loaded from the global itself.
 */
#include <stdio.h>
#include <string.h>

static struct {
    unsigned char i, j;
    unsigned char s[256];
    unsigned char r[32];
} gh;

static void HashInitX(void) {
    unsigned int k;
    gh.i = 0;
    gh.j = 0;
    for (k = 0; k < 256; k++)
        gh.s[k] = (unsigned char)k;
}

static void HashUpdateX(const unsigned char *aData, unsigned int nData) {
    unsigned char t, i = gh.i, j = gh.j;
    unsigned int k;
    for (k = 0; k < nData; k++) {
        j += gh.s[i] + aData[k];
        t = gh.s[j];
        gh.s[j] = gh.s[i];
        gh.s[i] = t;
        i++;
    }
    gh.i = i;
    gh.j = j;
}

static void HashFinalX(void) {
    unsigned int k;
    unsigned char t, i = gh.i, j = gh.j;
    for (k = 0; k < 32; k++) {
        i++;
        t = gh.s[i];
        j += t;
        gh.s[i] = gh.s[j];
        gh.s[j] = t;
        t += gh.s[i];
        gh.r[k] = gh.s[t];
    }
}

int main(void) {
    const char *m = "The quick brown fox jumps over the lazy dog, 1234567890.";
    int i;
    HashInitX();
    HashUpdateX((const unsigned char *)m, (unsigned int)strlen(m));
    HashFinalX();
    for (i = 0; i < 24; i++)
        printf("%02x", gh.r[i]);
    printf("\n");
    return 0;
}
