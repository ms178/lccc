/*
 * The value of a GNU statement expression, tail item by tail item.
 *
 * GCC 16.2 searches the tail of the compound statement backwards for the item
 * that produces the value, and three kinds of item are transparent to that
 * search:
 *
 *   - `_Static_assert` (the kernel's `sizeof(struct { _Static_assert(...); })`
 *     idiom queues one after the very item that must supply the value);
 *   - the null statement `;`;
 *   - declarations that reserve no block storage: `typedef`, `extern` objects,
 *     function and `inline` prototypes, and bare struct/enum declarations.
 *
 * Any definition of a block-scope object (`int y;`, `static int q;`,
 * `int y = 2;`) and any statement other than a bare expression makes the
 * statement expression `void`, as does a label around a null statement. A label
 * around an expression statement carries that expression's value, which is the
 * one shape where GCC and Clang agree.
 *
 * Clang 23.1.4 answers `void` for every transparent item, so this file pins the
 * GCC reading deliberately: GCC is this project's reference compiler and the
 * kernel compiles only because of it. check_stmt_expr_tail_parity.sh re-derives
 * every expectation below from the oracle and adds the value and diagnostic
 * halves of the contract that a compile-only fixture cannot express.
 */

#define TAIL_INT(x) \
    _Static_assert(__builtin_types_compatible_p(typeof(x), int), #x)
#define TAIL_VOID(x) \
    _Static_assert(__builtin_types_compatible_p(typeof(x), void), #x)

/* A statement expression is only valid inside a function, so the type matrix
 * lives in one (the assertions themselves are block-scope C11). */
static void tail_types(void) {
/* Transparent tails: the value is the last expression statement. */
TAIL_INT(({ 1; }));
TAIL_INT(({ 1; _Static_assert(1, "assert tail"); }));
TAIL_INT(({ 1; _Static_assert(1, "a"); _Static_assert(1, "b"); }));
TAIL_INT(({ _Static_assert(1, "assert first"); 2; }));
TAIL_INT(({ 1; ; }));
TAIL_INT(({ 1; ; ; }));
TAIL_INT(({ 1; struct TailTag { int member; }; }));
TAIL_INT(({ 1; enum TailEnum { TailEnumA }; }));
TAIL_INT(({ 1; typedef int TailTypedef; }));
TAIL_INT(({ 1; extern int tail_extern_object; }));
TAIL_INT(({ 1; int tail_prototype(void); }));
TAIL_INT(({ 1; inline int tail_inline_prototype(void); }));
TAIL_INT(({ 1; tail_label: 2; }));

/* Opaque tails: the statement expression is void. */
TAIL_VOID(({ 1; int tail_local; }));
TAIL_VOID(({ 1; static int tail_static; }));
TAIL_VOID(({ 1; int tail_local_init = 2; }));
TAIL_VOID(({ 1; { 2; } }));
TAIL_VOID(({ 1; if (1) ; }));
TAIL_VOID(({ 1; do { 2; } while (0); }));
TAIL_VOID(({ 1; tail_null_label: ; }));
TAIL_VOID(({ _Static_assert(1, "assert only"); }));
TAIL_VOID(({ ; }));
TAIL_VOID(({}));
}

struct tail_node { int value; };
static struct tail_node tail_nodes[2];

/* The values themselves, which a compile-only assertion cannot pin: the
 * lowering must deliver the expression that sema typed, not a zeroed slot. */
static int tail_values(void) {
    int value = ({ 7; _Static_assert(1, "assert tail"); });
    value += ({ 1; ; });                            /* null statement     */
    value += ({ 5; typedef int TailTypedef; }) * 10; /* typedef            */
    value += ({ 3; tail_label: 4; });               /* label wrapper      */
    /* The kernel shape: a member assertion inside a type computation. */
    value += ({ (typeof(tail_nodes[sizeof(struct { _Static_assert(1, "member"); })]) *)0 == 0; });
    return value;
}

int main(void) {
    tail_types();
    return tail_values() == 7 + 1 + 50 + 4 + 1 ? 0 : 1;
}
