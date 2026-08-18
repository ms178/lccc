struct s { int a; int pad[9]; int c; };
void f(struct s *p) { p->c |= 1; p->c <<= 2; }
int g(struct s *p) { return p->c + p->a; }
int main(void) {
    struct s x = {5, {0}, 8};
    f(&x);           /* c = (8|1)<<2 = 36 */
    return g(&x) == 41 ? 0 : 1;
}
