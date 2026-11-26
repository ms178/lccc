/*
 * SQLite 3.54.1 src/util.c sqlite3GetVarint control-flow shape (public domain).
 * Standalone differential/oracle stress for the constant-offset pointer-phi
 * optimization. Exhaustively exercises all 1..9-byte exit arms and checks an
 * independent iterative decoder, with negative cases for variable offsets.
 */
#include <stdint.h>
#include <stdio.h>

typedef uint8_t u8;
typedef uint64_t u64;

__attribute__((noinline))
u8 sqlite_varint_delta(const unsigned char *p, u64 *v){
  u64 key = p[0];
  const u8 *start = p;
  if( key>=0x80 ){
    u8 x;
    key = (key<<7) ^ (x = *++p);
    if( x>=0x80 ){
      key = (key<<7) ^ (x = *++p);
      if( x>=0x80 ){
        key = (key<<7) ^ 0x10204000 ^ (x = *++p);
        if( x>=0x80 ){
          key = (key<<7) ^ 0x4000 ^ (x = *++p);
          if( x>=0x80 ){
            key = (key<<7) ^ 0x4000 ^ (x = *++p);
            if( x>=0x80 ){
              key = (key<<7) ^ 0x4000 ^ (x = *++p);
              if( x>=0x80 ){
                key = (key<<7) ^ 0x4000 ^ (x = *++p);
                if( x>=0x80 ){
                  key = (key<<8) ^ 0x8000 ^ (*++p);
                }
              }
            }
          }
        }
      }else{
        key ^= 0x204000;
      }
    }else{
      key ^= 0x4000;
    }
  }
  *v = key;
  return (u8)(p - start) + 1;
}

static u8 reference_decode(const u8 *p, u64 *v){
  u64 result = 0;
  unsigned i;
  for(i=0; i<8; ++i){
    result = (result<<7) | (p[i]&0x7fU);
    if((p[i]&0x80U)==0){ *v=result; return (u8)(i+1); }
  }
  *v = (result<<8) | p[8];
  return 9;
}

__attribute__((noinline))
static long variable_delta(const u8 *p, unsigned offset, int take_variable){
  const u8 *q = take_variable ? p + offset : p + 3;
  return q - p;
}

__attribute__((noinline))
static long typed_delta(const int *p, int cond){
  const int *q = cond ? p + 3 : p + 7;
  return q - p;
}

static u64 rand_state = 0x20ce271ca755b509ULL;
static u8 next_byte(void){
  rand_state ^= rand_state<<13;
  rand_state ^= rand_state>>7;
  rand_state ^= rand_state<<17;
  return (u8)rand_state;
}

int main(void){
  u64 checksum = 1469598103934665603ULL;
  unsigned n, j, k;
  int a[16] = {0};
  for(n=1; n<=9; ++n){
    for(j=0; j<10000; ++j){
      u8 encoded[16] = {0};
      u64 actual = 0, reference = 0;
      u8 bytes, reference_bytes;
      for(k=0; k<n-1; ++k) encoded[k] = (next_byte()&0x7fU)|0x80U;
      encoded[n-1] = next_byte() & (n<9 ? 0x7fU : 0xffU);
      bytes = sqlite_varint_delta(encoded, &actual);
      reference_bytes = reference_decode(encoded, &reference);
      if((unsigned)bytes != n || (unsigned)reference_bytes != n || actual != reference){
        fprintf(stderr,"varint mismatch: n=%u iter=%u got=%u/%llu want=%u/%llu\n",
                n,j,bytes,(unsigned long long)actual,reference_bytes,
                (unsigned long long)reference);
        return 1;
      }
      checksum = (checksum ^ actual) * 1099511628211ULL;
      checksum = (checksum ^ bytes) * 1099511628211ULL;
    }
  }
  for(j=0; j<16; ++j){
    if(variable_delta((const u8 *)a, j, 1) != j
       || variable_delta((const u8 *)a, j, 0) != 3
       || typed_delta(a, j&1) != ((j&1) ? 3 : 7)){
      fprintf(stderr,"pointer-difference mismatch: %u\n",j);
      return 2;
    }
  }
  printf("ptr_delta_phi: 90000 varints verified; checksum=%llu\n",
         (unsigned long long)checksum);
  return 0;
}
