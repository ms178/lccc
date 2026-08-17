/* Typedef name as label (shadow) must parse as label, not type. */
typedef int myint;
int main(void) {
    int x = 0;
myint:
    x = 1;
    if (x != 1) return 1;
    return 0;
}
