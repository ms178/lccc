/* Force a body block between header and latch via opaque barrier... 
 * Actually a simple counted store loop often lowers to 3 blocks. */
int main(void) {
    int a[4];
    for (int i = 0; i < 4; i++)
        a[i] = i * i;
    int s = 0;
    for (int i = 0; i < 4; i++)
        s += a[i];
    return s == 14 ? 0 : 1; /* 0+1+4+9 */
}
