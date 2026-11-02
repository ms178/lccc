/* Wide / narrow mix stays well-formed after capacity changes. */
int main(void) {
    const char *a = "xy";
    if (a[0] != 'x' || a[1] != 'y') return 1;
    return 0;
}
