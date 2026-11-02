volatile int source;
__typeof__(source) from_expr;
__typeof__(volatile int) from_type;
__typeof__(__typeof__(volatile int)) from_nested;
__typeof_unqual__(source) unqual_expr;
__typeof_unqual__(volatile int) unqual_type;
__typeof__(__typeof_unqual__(volatile int)) nested_unqual;
const __typeof__(source) cv = 3;
__typeof__(source) *to_volatile;
volatile int *plain_pointer;
__typeof__(plain_pointer) pointer_copy;
void expr_discard(void) { from_expr; from_expr; }
int expr_sum(void) { return from_expr + from_expr; }
void expr_stores(void) { from_expr = 1; from_expr = 2; }
void type_discard(void) { from_type; from_type; }
void nested_discard(void) { from_nested; from_nested; }
int cv_reads(void) { return cv + cv; }
void unqual_discard(void) { unqual_expr; unqual_type; nested_unqual; }
void pointer_discard(void) { pointer_copy; pointer_copy; }
void indirect(void) { *to_volatile; *to_volatile; }
void block_extern(void) { extern __typeof__(source) block_ext; block_ext; block_ext; }
void block_local(void) { __typeof__(source) local = 0; local; local; }
void shadow(void) { __typeof__(source) *source = 0, local = 7; local; local; }
int parameter(__typeof__(source) p) { return p + p; }
volatile __typeof_unqual__(volatile int) outer_volatile;
void outer_volatile_reads(void) { outer_volatile; outer_volatile; }
int plain_source;
void shadow_nonvolatile(void) {
    __typeof__(plain_source) *plain_source = 0, local = 0;
    local; local;
}
