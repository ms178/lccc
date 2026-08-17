/* Hex float still parses (exp path uses digit accumulate). */
int main(void) {
    double d = 0x1.0p+0;
    if (d != 1.0) return 1;
    double e = 0x1p4;
    if (e != 16.0) return 2;
    return 0;
}
