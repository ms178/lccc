/* A member assertion queued for sema must not erase the GNU statement
   expression's source-level tail. Linux's hash/RCU typeof expansion has this
   shape; cover both unevaluated typeof and an evaluated result. */
struct node { int value; };
static struct node nodes[] = {{ 7 }, { 11 }};
static int touches;

int main(void) {
    typeof(({ (typeof(nodes[sizeof(struct { _Static_assert(1, "ok"); })]) *)0; })) p = &nodes[0];
    _Static_assert(__builtin_types_compatible_p(typeof(*p), struct node), "pointer type");
    int value = ({ nodes[sizeof(struct { _Static_assert(sizeof(int) > 0, "ok"); })].value + ++touches; });
    return p->value != 7 || value != 8 || touches != 1;
}
