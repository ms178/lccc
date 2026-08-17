double madd(double a, double b, double c) { return a * b + c; }
float maddf(float a, float b, float c) { return a * b + c; }
int main(void) {
    double d = madd(1.5, 2.5, 3.5);
    if (d < 7.24 || d > 7.26) return 1;
    float f = maddf(1.5f, 2.5f, 3.5f);
    if (f < 7.24f || f > 7.26f) return 2;
    double dx = 3.0, dy = 4.0;
    double r2 = dx * dx + dy * dy;
    if (r2 < 24.9 || r2 > 25.1) return 3;
    return 0;
}
