#ifndef INCOMING_BYVAL_COMMON_H
#define INCOMING_BYVAL_COMMON_H
struct Big { long a, b, c; };
typedef struct Big AlignedBig __attribute__((aligned(64)));
typedef _Float128 AlignedFloat128 __attribute__((aligned(64)));
long aligned_address(const void *, unsigned);
struct Small { long a, b; };
struct __attribute__((packed)) Packed { char a; long b; };
struct __attribute__((aligned(64))) Wide { long a[8]; };
long adjust(struct Big *, const struct Big *);
long read_big(struct Big *);
long tail_big(struct Big);
long consume_wide(struct Wide);
long sum_big(struct Big);
long mutate_big(struct Big, const struct Big *);
long dynamic_big(struct Big, unsigned);
long realign_big(struct Big, struct Wide);
long aligned_f128(long,long,long,long,long,long,long,
                  double,double,double,double,double,double,double,double,
                  AlignedFloat128,long);
long aligned_object(long,long,long,long,long,long,long,struct Big,long);
long variadic_big(struct Big, ...);
long exhausted_small(long,long,long,long,long,long,struct Small);
long volatile_big(volatile struct Big);
long packed_value(struct Packed);
struct Big return_big(struct Big);
#endif
