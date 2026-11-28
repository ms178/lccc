/* A statement that takes the frame pointer register, in a function that
   keeps its frame pointer.

   GCC refuses it: "bp cannot be used in 'asm' here", reported at the end
   of the function. lccc compiled it, gave the function a frame pointer
   *because* of the clobber, and addressed every slot behind the statement
   through the destroyed register.

   check_asm_registers.py compiles one function at a time (GCC stops at the
   first it refuses):

   * `kept_for_the_array` is refused everywhere: a variable-length array
     needs a frame pointer;
   * `kept_by_request` is refused where the frame pointer was asked for
     (-O0, -fno-omit-frame-pointer);
   * `omitted` is the valid use, at -O2: with the frame pointer omitted the
     register is one more callee-saved register. */
int sink(int *p, int n);

int kept_for_the_array(int n) {
    int a[n];
    a[0] = n;
    __asm__ volatile("xorl %%ebp, %%ebp" : : : "ebp");
    return sink(a, n);
}

int kept_by_request(int x) {
    volatile int v = x;
    __asm__ volatile("xorl %%ebp, %%ebp" : : : "bp");
    return v;
}

int omitted(int x) {
    volatile int v = x;
    __asm__ volatile("xorl %%ebp, %%ebp" : : : "%ebp");
    return v;
}
