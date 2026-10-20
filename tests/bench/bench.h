/* Contract between the shared driver and each benchmark kernel. */
#ifndef LCCC_BENCH_H
#define LCCC_BENCH_H

/* Prepare deterministic input data. Called once, untimed. */
void bench_setup(void);

/* One unit of work. Must return a value derived from the whole computation so
 * the driver's checksum detects an arm that computed something different. */
unsigned long long bench_run(void);

/* Optional algorithm oracle, called once by the driver after bench_setup
 * (weak default in driver.c returns 0).  Return non-zero to fail the run.
 * The cross-compiler checksum comparison alone cannot catch a kernel whose
 * ALGORITHM is wrong-but-deterministic -- every arm compiles the same
 * source and agrees on the wrong answer -- so kernels with canonical test
 * vectors (CRC-32 check value, RFC 4648 encodings, encode/decode
 * round-trips) assert them here. */
int bench_verify(void);

#endif
