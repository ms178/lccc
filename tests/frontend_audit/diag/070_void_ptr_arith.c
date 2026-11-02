/* expect: accept */
int main(void) { void *p = 0; p = p + 1; return p != 0; }
