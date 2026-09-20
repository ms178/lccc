branch_index_store:
  testq %rdi, %rdi
  je .LBB4_6
  xorl %eax, %eax
  jmp .LBB4_2
.LBB4_4:
  movl branch_slots(,%rcx,4), %ecx
.LBB4_5:
  movl %ecx, branch_slots(,%rax,4)
  incq %rax
  leaq -1(%rdi), %rcx
  andq %rcx, %rdi
  je .LBB4_6
.LBB4_2:
  rep bsfq %rdi, %rcx
  cmpq %rcx, %rax
  jne .LBB4_4
  movl %eax, %ecx
  jmp .LBB4_5
.LBB4_6:
  retq

.LCPI5_0:
  .long 1
  .long 8
  .long 15
  .long 22
.LCPI5_1:
  .long 29
  .long 36
  .long 43
  .long 50
.LCPI5_2:
  .long 57
  .long 64
  .long 71
  .long 78
.LCPI5_3:
  .long 85
  .long 92
  .long 99
  .long 106
.LCPI5_4:
  .long 1020
  .long 1275
  .long 1530
  .long 1785
.LCPI5_5:
  .long 2040
  .long 2295
  .long 2550
  .long 2805
.LCPI5_6:
  .long 3060
  .long 3315
  .long 3570
  .long 3825
.LCPI5_7:
  .long 0
  .long 255
  .long 510
  .long 765
.LCPI5_8:
  .zero 16,15
.LCPI5_9:
  .byte 0
  .byte 1
  .byte 2
  .byte 3
  .byte 4
  .byte 5
  .byte 6
  .byte 7
  .byte 8
  .byte 9
  .byte 10
  .byte 11
  .byte 12
  .byte 13
  .byte 14
  .byte 15
.LCPI5_10:
  .short 79
  .short 79
  .short 79
  .short 79
  .short 79
  .short 79
  .short 79
  .short 79
.LCPI5_11:
  .zero 16,63
.LCPI5_12:
  .short 13
  .short 13
  .short 13
  .short 13
  .short 13
  .short 13
  .short 13
  .short 13
.LCPI5_13:
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
.LCPI5_14:
  .long 4294967290
  .long 4294967290
  .long 4294967290
  .long 4294967290
.LCPI5_15:
  .short 205
  .short 205
  .short 205
  .short 205
  .short 205
  .short 205
  .short 205
  .short 205
.LCPI5_16:
  .zero 16,252
.LCPI5_17:
  .quad -2
  .quad -2
.LCPI5_18:
  .short 117
  .short 117
  .short 117
  .short 117
  .short 117
  .short 117
  .short 117
  .short 117
.LCPI5_19:
  .zero 16,127
.LCPI5_20:
  .zero 16,31
.LCPI5_21:
  .short 11
  .short 11
  .short 11
  .short 11
  .short 11
  .short 11
  .short 11
  .short 11
.LCPI5_22:
  .quad -5
  .quad -5
.LCPI5_23:
  .byte 0
  .byte 1
  .byte 2
  .byte 3
  .byte 4
  .byte 5
  .byte 6
  .byte 7
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
.LCPI5_24:
  .long 4294967278
  .long 4294967278
  .long 4294967278
  .long 4294967278
.LCPI5_25:
  .quad 1
  .quad 1
.LCPI5_26:
  .long 100
  .long 101
  .long 102
  .long 103
.LCPI5_27:
  .long 104
  .long 105
  .long 106
  .long 107
.LCPI5_28:
  .zero 4
  .long 8
  .long 15
  .long 22
.LCPI5_29:
  .long 344
  .long 351
  .long 358
  .long 365
.LCPI5_30:
  .long 316
  .long 323
  .long 330
  .long 337
.LCPI5_31:
  .long 288
  .long 295
  .long 302
  .long 309
.LCPI5_32:
  .long 260
  .long 267
  .long 274
  .long 281
.LCPI5_33:
  .long 400
  .long 407
  .long 414
  .long 421
.LCPI5_34:
  .long 372
  .long 379
  .long 386
  .long 393
.LCPI5_35:
  .long 55
  .long 66
  .long 77
  .long 88
.LCPI5_36:
  .long 270544960
  .long 1348497536
  .long 2426450112
  .long 3504402432
.LCPI5_37:
  .long 11
  .long 22
  .long 33
  .long 44
