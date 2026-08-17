/* Freestanding: links to a complete executable with no libc, so the fuzzer
   drives the FULL pipeline (resolution -> layout -> reloc -> emit). */
static char msg[16] = "hello-lccc\n";
static int table[8] = {1,2,3,4,5,6,7,8};
__thread int tls_var = 7;
int helper(int x) { return x * 3 + table[x & 7]; }
static long wr(long fd, const void *b, long n) {
    long r; __asm__ volatile("syscall" : "=a"(r) : "a"(1),"D"(fd),"S"(b),"d"(n)
                             : "rcx","r11","memory"); return r;
}
void _start(void) {
    wr(1, msg, 11);
    long code = helper(5) + tls_var;
    __asm__ volatile("syscall" :: "a"(60), "D"(code));
    __builtin_unreachable();
}
