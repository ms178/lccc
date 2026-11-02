/* Conditional swap through pointers. */
void swapmax(int *a, int *b) {
    if (*b > *a) { int t = *a; *a = *b; *b = t; }
}
