/* trip-3 F64: triangular residual after outer i=2 style. */
int main(void) {
    double s = 0.0;
    for (int j = 2; j < 5; j++)
        s += (double)j;
    /* 2+3+4 = 9 */
    return (s > 8.999 && s < 9.001) ? 0 : 1;
}
