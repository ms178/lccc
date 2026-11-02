/* trip-5 F64 accumulation — nbody-shaped (F64, not F32 map remainder). */
int main(void) {
    double s = 0.0;
    for (int i = 0; i < 5; i++)
        s += (double)(i + 1) * 0.5;
    /* 0.5+1.0+1.5+2.0+2.5 = 7.5 */
    return (s > 7.499 && s < 7.501) ? 0 : 1;
}
