int main(void) {
    double s = 0.0;
    for (int i = 0; i < 4; i++) s += (double)(i + 1) * 0.5;
    return (s > 4.999 && s < 5.001) ? 0 : 1;
}
