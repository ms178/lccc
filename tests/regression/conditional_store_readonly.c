/* A same-address load does not make a skipped store safe. The page is
   readable but not writable, and every guard is false. A blend-and-store
   rewrite faults even though the original program performs no writes. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

__attribute__((noinline))
static void clamp_negative(int32_t *p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int32_t v = p[i];
        if (v < 0) p[i] = 0;
    }
}

__attribute__((noinline))
static void fold_upper(unsigned char *p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c >= 'A' && c <= 'Z') p[i] = (unsigned char)(c + 32);
    }
}

__attribute__((noinline))
static void replace_outliers(unsigned char *p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c == 0 || c > 250) p[i] = 255;
    }
}

__attribute__((noinline))
static void update_three(int32_t *p, unsigned long n) {
    for (unsigned long i = 0; i < n; ++i) {
        int32_t v = p[i];
        if (v > 1 && v < 10 && v != 5) p[i] = v + 3;
    }
}

/* A genuinely private object remains a valid optimization opportunity. */
__attribute__((noinline))
static unsigned private_array(unsigned n) {
    unsigned char p[80];
    unsigned sum = 0;
    for (unsigned i = 0; i < 80; ++i) p[i] = (unsigned char)(i + 48);
    for (unsigned i = 0; i < n; ++i) {
        unsigned char c = p[i];
        if (c >= 'A' && c <= 'Z') p[i] = (unsigned char)(c + 32);
    }
    for (unsigned i = 0; i < 80; ++i) sum += p[i];
    return sum;
}

int main(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page < 2048) return 1;
    unsigned char *mem = mmap(0, (size_t)page, PROT_READ | PROT_WRITE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mem == MAP_FAILED) return 2;
    for (long i = 0; i < page; ++i) mem[i] = 'q';
    if (mprotect(mem, (size_t)page, PROT_READ) != 0) return 3;
    /* 257 crosses SSE, AVX, and scalar-tail boundaries. 'q' is neither
       uppercase nor an outlier, and 0x71717171 is a positive int32. */
    clamp_negative((int32_t *)mem, 257);
    fold_upper(mem, 257);
    replace_outliers(mem, 257);
    update_three((int32_t *)mem, 257);
    if (munmap(mem, (size_t)page) != 0) return 4;

    for (unsigned n = 0; n <= 65; ++n) {
        int32_t nums[80];
        unsigned char bytes[80];
        for (unsigned i = 0; i < 80; ++i) {
            nums[i] = (int32_t)(i % 17) - 8;
            bytes[i] = (unsigned char)(i + 48);
        }
        clamp_negative(nums, n);
        fold_upper(bytes, n);
        unsigned expected_sum = 0;
        for (unsigned i = 0; i < 80; ++i) {
            int32_t v = (int32_t)(i % 17) - 8;
            unsigned char c = (unsigned char)(i + 48);
            if (i < n && v < 0) v = 0;
            if (i < n && c >= 'A' && c <= 'Z') c = (unsigned char)(c + 32);
            if (nums[i] != v || bytes[i] != c) return 5;
            expected_sum += c;
        }
        if (private_array(n) != expected_sum) return 6;
    }
    puts("conditional_store_readonly: PASS");
    return 0;
}
