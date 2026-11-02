branch_index_store:
  testq %rdi, %rdi
  je .LBB4_6
  leaq branch_slots(%rip), %rax
  xorl %ecx, %ecx
  movq %rax, %rdx
  jmp .LBB4_2
.LBB4_3:
  movl %ecx, %esi
.LBB4_5:
  movl %esi, (%rdx)
  incq %rcx
  leaq -1(%rdi), %rsi
  addq $4, %rdx
  andq %rsi, %rdi
  je .LBB4_6
.LBB4_2:
  rep bsfq %rdi, %rsi
  cmpq %rsi, %rcx
  je .LBB4_3
  movl (%rax,%rsi,4), %esi
  jmp .LBB4_5
.LBB4_6:
  retq

.LCPI5_1:
  .long 0
  .long 1
  .long 2
  .long 3
.LCPI5_2:
  .zero 16,15
.LCPI5_4:
  .zero 16,63
.LCPI5_5:
  .short 13
  .short 13
  .short 13
  .short 13
  .zero 2
  .zero 2
  .zero 2
  .zero 2
.LCPI5_6:
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
  .short 255
.LCPI5_7:
  .long 4294967278
  .long 4294967278
  .long 4294967278
  .long 4294967278
.LCPI5_8:
  .long 4
  .long 4
  .long 4
  .long 4
.LCPI5_9:
  .byte 4
  .byte 4
  .byte 4
  .byte 4
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
  .zero 1
.LCPI5_10:
  .quad 1
  .quad 1
.LCPI5_11:
  .long 100
  .long 101
  .long 102
  .long 103
.LCPI5_12:
  .long 104
  .long 105
  .long 106
  .long 107
.LCPI5_13:
  .long 108
  .long 109
  .long 110
  .long 111
.LCPI5_14:
  .long 112
  .long 113
  .long 114
  .long 115
.LCPI5_15:
  .long 116
  .long 117
  .long 118
  .long 119
.LCPI5_16:
  .long 120
  .long 121
  .long 122
  .long 123
.LCPI5_17:
  .long 124
  .long 125
  .long 126
  .long 127
.LCPI5_18:
  .long 128
  .long 129
  .long 130
  .long 131
.LCPI5_19:
  .long 132
  .long 133
  .long 134
  .long 135
.LCPI5_20:
  .long 136
  .long 137
  .long 138
  .long 139
.LCPI5_21:
  .long 140
  .long 141
  .long 142
  .long 143
.LCPI5_22:
  .long 144
  .long 145
  .long 146
  .long 147
.LCPI5_23:
  .long 148
  .long 149
  .long 150
  .long 151
.LCPI5_24:
  .long 152
  .long 153
  .long 154
  .long 155
.LCPI5_25:
  .long 156
  .long 157
  .long 158
  .long 159
.LCPI5_26:
  .long 160
  .long 161
  .long 162
  .long 163
.LCPI5_27:
  .long 1
  .long 8
  .long 15
  .long 22
.LCPI5_28:
  .long 29
  .long 36
  .long 43
  .long 50
.LCPI5_29:
  .long 57
  .long 64
  .long 71
  .long 78
.LCPI5_30:
  .long 85
  .long 92
  .long 99
  .long 106
.LCPI5_31:
  .long 113
  .long 120
  .long 127
  .long 134
.LCPI5_32:
  .long 141
  .long 148
  .long 155
  .long 162
.LCPI5_33:
  .long 169
  .long 176
  .long 183
  .long 190
.LCPI5_34:
  .long 197
  .long 204
  .long 211
  .long 218
.LCPI5_35:
  .long 225
  .long 232
  .long 239
  .long 246
.LCPI5_36:
  .long 253
  .long 260
  .long 267
  .long 274
.LCPI5_37:
  .long 281
  .long 288
  .long 295
  .long 302
.LCPI5_38:
  .long 309
  .long 316
  .long 323
  .long 330
.LCPI5_39:
  .long 337
  .long 344
  .long 351
  .long 358
.LCPI5_40:
  .long 365
  .long 372
  .long 379
  .long 386
.LCPI5_41:
  .long 393
  .long 400
  .long 407
  .long 414
.LCPI5_42:
  .long 421
  .long 428
  .long 435
  .long 442
.LCPI5_43:
  .long 55
  .long 66
  .long 77
  .long 88
.LCPI5_44:
  .long 11
  .long 22
  .long 33
  .long 44
.LCPI5_45:
  .long 270544960
  .long 1348497536
  .long 2426450112
  .long 3504402432
.LCPI5_46:
  .byte 0
  .byte 1
  .byte 2
  .byte 3
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
.LCPI5_47:
  .byte 79
  .byte 0
  .byte 79
  .byte 0
  .byte 79
  .byte 0
  .byte 79
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
  .byte 0
