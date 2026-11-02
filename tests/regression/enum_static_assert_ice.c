/* _Static_assert evaluated by sema with the full type context: sizeof /
 * _Alignof of enum tags (and records containing them) must match the
 * GCC-compatible layout, and a false condition must be a compile error.
 * The negative controls live in check_enum_layout_semantics.sh (ephemeral). */
enum Big { B0 = 0x100000000LL };
enum Neg { N1 = -1, N2 = 4000000000u };
struct WithEnum { char c; enum Big b; };

_Static_assert(_Alignof(enum Big) == 8, "align8");
_Static_assert(sizeof(enum Neg) == 8, "neg8");
_Static_assert(sizeof(struct WithEnum) == 16, "we16");
_Static_assert(_Alignof(struct WithEnum) == 8, "alignwe8");
_Static_assert(sizeof(enum Big) == 8, "big8");

int main(void) { return 0; }
