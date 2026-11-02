/* expect: reject */
_Static_assert(sizeof(char) == 2, "must fail");
int main(void) { return 0; }
