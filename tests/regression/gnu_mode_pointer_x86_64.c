/* `__attribute__((mode(...)))` on a POINTER type -- x86-64 verdict.
 *
 * On x86-64 the pointer mode is DI: `mode(DI)` is accepted and leaves the
 * typedef a `char *`, while `mode(SI)` is refused ("invalid pointer mode
 * 'SI'").  The compiled TU is a positive fixture (it must build and run);
 * the refusal of the SI spelling is pinned by the companion
 * `gnu_mode_pointer_si_on_x86_64.c`, and the mirror-image i686 verdicts by
 * `gnu_mode_pointer_i686.c` in check_gnu_declaration_corners.sh, so neither
 * target can inherit the other's rule.
 */
typedef char *P __attribute__((mode(DI)));

static char cell = 'x';

P identity(P p) { return p; }

int main(void) {
    char *q = identity(&cell);
    return (*q == 'x') ? 0 : 1;
}
