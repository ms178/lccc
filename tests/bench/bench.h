/* Contract between the shared driver and each benchmark kernel. */
#ifndef LCCC_BENCH_H
#define LCCC_BENCH_H

/* Prepare deterministic input data. Called once, untimed. */
void bench_setup(void);

/* One unit of work. Must return a value derived from the whole computation so
 * the driver's checksum detects an arm that computed something different. */
unsigned long long bench_run(void);

#endif
