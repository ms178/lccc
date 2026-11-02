/* expect: reject */
struct A { int x; };
int main(void) { struct A a; struct B b; a = b; return 0; }
