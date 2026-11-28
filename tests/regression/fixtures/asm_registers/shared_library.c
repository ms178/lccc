/* A shared library whose assembler statements name its globals.

   `counter` has default visibility: the program that links against the
   library refers to it too, and the dynamic linker gives both one object
   (the executable's copy). The statement has to address that object, which
   it finds through the GOT. `hidden_counter` is the library's own. */
int counter = 5;
static int hidden_counter = 11;
__thread int library_tls = 40;

int library_bump(void) {
    int r;
    __asm__ volatile("addl $1, %1\n\tmovl %1, %0\n\taddl %2, %0" : "=&r"(r), "+m"(counter) : "m"(hidden_counter));
    return r;
}

int library_tls_bump(void) {
    __asm__ volatile("addl $2, %0" : "+m"(library_tls));
    return library_tls;
}

int library_read(void) {
    return counter;
}
