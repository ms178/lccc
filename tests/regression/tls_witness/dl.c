/* dlopen'ed after startup: its TLS needs the dynamic model for real. The
   1 MiB block exceeds glibc's static-TLS surplus, so an Initial-Exec (or
   DF_STATIC_TLS) build fails dlopen with "cannot allocate memory in static
   TLS block" -- only GD/LD (descriptor or __tls_get_addr) loads it. */
__thread int dv = 100;
static __thread int ds;
static __thread char big[1 << 20];
int dget(void) { ds++; big[ds * 4096] += (char)ds; return dv + ds + big[4096] - 1; }
