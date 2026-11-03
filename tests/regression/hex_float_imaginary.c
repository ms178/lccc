/* Hexadecimal floating literals with GCC's i/j imaginary suffixes. */
int main(void) {
    volatile _Complex double di = 0x1.8p+1i;
    volatile _Complex double dj = 0x1.8p+1j;
    volatile _Complex float fi = 0x1.8p+1fi;
    volatile _Complex float if_ = 0x1.8p+1if;
    volatile _Complex long double li = 0x1.8p+1Li;
    volatile _Complex long double il = 0x1.8p+1il;

    return __imag__ di == 3.0 && __imag__ dj == 3.0 &&
                   __imag__ fi == 3.0f && __imag__ if_ == 3.0f &&
                   __imag__ li == 3.0L && __imag__ il == 3.0L
               ? 0
               : 1;
}
