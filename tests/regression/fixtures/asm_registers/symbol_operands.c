/* Memory operands that name a symbol, on x86-64 and i686.

   What the operand prints depends on what the symbol is:

   * another module may define it (default visibility, -fPIC): its address
     is in the GOT. lccc printed `counter(%rip)` on x86-64, a PC-relative
     relocation no linker makes a shared object of, and the absolute
     `counter` on i686, a text relocation;
   * local to the module: relative to the GOT pointer on i686
     (`local@GOTOFF(%ebx)`), unless the statement takes %ebx, and then
     through a register of its own; `local(%rip)` on x86-64;
   * thread-local: the thread's object (`%fs:tls@tpoff`), not the image
     every thread is initialised from. `incl tls_var(%rip)` incremented
     that image and left this thread's variable alone.

   Offsets into arrays and structures ride along (`arr+8`). */
#include <stdio.h>
#include <string.h>

#define NOINLINE __attribute__((noinline))

#if defined(__x86_64__)
#define BX "%%rbx"
#else
#define BX "%%ebx"
#endif

int counter = 5;
static int local_counter = 11;
int arr[8] = { 1, 2, 3, 4, 5, 6, 7, 8 };
static struct {
    char pad[3];
    int field;
    short tail;
} rec = { "ab", 77, 9 };
__thread int tls_global = 21;
static __thread int tls_local = 31;
__thread int tls_array[4] = { 100, 200, 300, 400 };
char text[16] = "abcdefgh";

/* the address of a global as the %ebx input */
NOINLINE int address_in_bx(void) {
    int r;
    __asm__ volatile("movl (" BX "), %0" : "=r"(r) : "b"(&counter));
    return r + (int)strlen(text) + local_counter;
}

/* offsets, the register free */
NOINLINE int offsets(void) {
    int r;
    __asm__ volatile("movl %1, %0\n\taddl %2, %0\n\taddl %3, %0"
                     : "=&r"(r)
                     : "m"(arr[2]), "m"(rec.field), "m"(arr[7]));
    return r + (int)strlen(text);
}

/* the same with %ebx taken */
NOINLINE int offsets_with_bx_taken(int x) {
    int r;
    __asm__ volatile("movl %1, %0\n\taddl %2, %0\n\taddl %%ebx, %0"
                     : "=&r"(r)
                     : "m"(arr[3]), "m"(rec.field), "b"(x));
    return r + (int)strlen(text) + counter;
}

/* operands that are read and written */
NOINLINE int read_write(void) {
    __asm__ volatile("addl $3, %0\n\taddl $4, %1" : "+m"(counter), "+m"(local_counter));
    __asm__ volatile("incl %0" : "+m"(arr[5]));
    return counter + local_counter + arr[5] + (int)strlen(text);
}

/* thread-local operands: read, written, and with an offset */
NOINLINE int thread_objects(void) {
    int r;
    __asm__ volatile("movl %1, %0\n\taddl %2, %0" : "=&r"(r) : "m"(tls_global), "m"(tls_local));
    __asm__ volatile("incl %0" : "+m"(tls_local));
    int scratch;
    __asm__ volatile("movl %2, %1\n\taddl $5, %1\n\taddl %1, %0"
                     : "+m"(tls_array[2]), "=&r"(scratch)
                     : "m"(tls_array[3]));
    return r + tls_local + tls_array[2] + (int)strlen(text);
}

/* two statements: %ebx taken, then a local symbol through the GOT pointer */
NOINLINE int two_statements(int x) {
    int a, b;
    __asm__ volatile("movl %%ebx, %0" : "=r"(a) : "b"(x));
    __asm__ volatile("movl %1, %0" : "=r"(b) : "m"(local_counter));
    return a + b + counter;
}

int main(void) {
    printf("address_in_bx %d\n", address_in_bx());
    printf("offsets %d\n", offsets());
    printf("offsets_with_bx_taken %d\n", offsets_with_bx_taken(1000));
    printf("read_write %d\n", read_write());
    printf("thread_objects %d\n", thread_objects());
    printf("thread_objects again %d\n", thread_objects());
    printf("two_statements %d\n", two_statements(300));
    return 0;
}
