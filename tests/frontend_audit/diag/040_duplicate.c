/* expect: reject */
int f(void) { return 1; }
int f(void) { return 2; }
int main(void) { return f(); }
