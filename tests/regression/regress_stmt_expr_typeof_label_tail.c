/* Differential-testing regression case, ported from John Regehr's
 * claudes-c-compiler "yarpgen" branch (CC0, https://github.com/regehr/claudes-c-compiler).
 * Reduced by yarpgen/csmith differential testing against gcc. The expected
 * output is asserted by the lccc regression runner via the GCC oracle. */

int printf(const char *, ...);
int main(void) {
  int out = ({
    unsigned _b = ({
      __typeof__(({
        _b:
          _b;
      })) _b = ({
        __typeof__(0) _a = -80;
        _a;
      });
      0 > _b;
    });
    _b;
  });
  printf("%d\n", out);
  return 0;
}
