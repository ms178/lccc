count_primes:
  stp x29, x30, [sp, -32]!
  mov x2, 38529
  movk x2, 0x98, lsl 16
  mov x29, sp
  mov w1, 1
  str x19, [sp, 16]
  adrp x19, sieve
  add x0, x19, :lo12:sieve
  bl memset
  mov w2, 38528
  mov x3, x0
  strh wzr, [x19, #:lo12:sieve]
  mov x1, 2
  mov w0, 4
  movk w2, 0x98, lsl 16
  b .L4
.L2:
  add w0, w4, 1
  add x1, x1, 1
  mul w0, w0, w0
  cmp w0, w2
  bgt .L17
.L4:
  ldrb w5, [x3, x1]
  mov w4, w1
  cbz w5, .L2
  sxtw x0, w0
.L3:
  strb wzr, [x3, x0]
  add x0, x0, x1
  cmp w0, w2
  ble .L3
  add w0, w4, 1
  add x1, x1, 1
  mul w0, w0, w0
  cmp w0, w2
  ble .L4
.L17:
  add x3, x19, :lo12:sieve
  mov w0, 0
  add x1, x3, 2
  add x3, x3, 9998336
  add x3, x3, 1665
.L6:
  ldrb w2, [x1], 1
  cmp w2, 0
  cinc w0, w0, ne
  cmp x3, x1
  bne .L6
  ldr x19, [sp, 16]
  ldp x29, x30, [sp], 32
  ret
.LC0:
  .string "primes up to %d: %d\n"
