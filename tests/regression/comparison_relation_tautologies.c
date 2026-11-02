/* Side-effect-free comparison pairs can be modeled over the three ordering
 * outcomes {less,equal,greater}. Empty intersections are && contradictions;
 * full unions are || tautologies and must leave no dead symbol reference. */
extern void unreachable_comparison_path(void);

__attribute__((noinline))
static void check(int x, int y)
{
    if ((x == y) && (x != y))
        unreachable_comparison_path();
    if ((x < y) && (y < x))
        unreachable_comparison_path();
    if (!((x >= y) || (x < y)))
        unreachable_comparison_path();
    if (!((x <= y) || (y < x)))
        unreachable_comparison_path();
}

int main(void)
{
    check(0, 0);
    check(1, 2);
    check(4, 3);
    return 0;
}
