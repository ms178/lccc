/* The BMI2 builtin itself needs BMI2, as in GCC: without it this TU must be
 * rejected with a diagnostic naming the extension. */
unsigned long long f(unsigned long long x, unsigned long long n)
{
    return __builtin_ia32_bzhi_di(x, n);
}
