energy:
  vmovsd bodies+32(%rip), %xmm1
  vmovsd bodies+24(%rip), %xmm0
  movl $bodies+56, %ecx
  movl $4, %edi
  vmovsd bodies+48(%rip), %xmm6
  vmovsd .LC0(%rip), %xmm11
  movl $1, %r8d
  vmulsd %xmm1, %xmm1, %xmm1
  vmovsd bodies+40(%rip), %xmm12
  vmulsd %xmm11, %xmm6, %xmm2
  vfmadd132sd %xmm0, %xmm1, %xmm0
  vfmadd132sd %xmm12, %xmm0, %xmm12
  vxorpd %xmm0, %xmm0, %xmm0
  vfmadd132sd %xmm2, %xmm0, %xmm12
.L2:
  vmovsd -56(%rcx), %xmm4
  vmovsd -48(%rcx), %xmm5
  vmovsd -40(%rcx), %xmm3
  cmpl $4, %r8d
  je .L8
  movl %edi, %edx
  vmovddup %xmm6, %xmm10
  vmovddup %xmm4, %xmm9
  movq %rcx, %rax
  shrl %edx
  vmovddup %xmm5, %xmm8
  vmovddup %xmm3, %xmm7
  movl %edx, %esi
  imulq $112, %rsi, %rsi
  addq %rcx, %rsi
.L4:
  vmovupd (%rax), %xmm14
  vmovupd 48(%rax), %xmm2
  addq $112, %rax
  vmovupd -48(%rax), %xmm0
  vmovsd %xmm14, %xmm2, %xmm1
  vsubpd %xmm1, %xmm9, %xmm13
  vmovhps -8(%rax), %xmm2, %xmm2
  vmulpd %xmm10, %xmm2, %xmm2
  vshufpd $1, %xmm0, %xmm14, %xmm1
  vsubpd %xmm1, %xmm8, %xmm1
  vmovlpd -96(%rax), %xmm0, %xmm0
  vsubpd %xmm0, %xmm7, %xmm0
  vmulpd %xmm1, %xmm1, %xmm1
  vfmadd231pd %xmm13, %xmm13, %xmm1
  vfmadd132pd %xmm0, %xmm1, %xmm0
  vsqrtpd %xmm0, %xmm0
  vdivpd %xmm0, %xmm2, %xmm0
  vsubsd %xmm0, %xmm12, %xmm12
  vunpckhpd %xmm0, %xmm0, %xmm0
  vsubsd %xmm0, %xmm12, %xmm12
  cmpq %rsi, %rax
  jne .L4
  addl %edx, %edx
  leal (%rdx,%r8), %eax
  cmpl %edx, %edi
  je .L12
.L3:
  imulq $56, %rax, %rax
  vmovsd 24(%rcx), %xmm2
  vmovsd 40(%rcx), %xmm1
  vsubsd bodies+8(%rax), %xmm5, %xmm5
  vsubsd bodies(%rax), %xmm4, %xmm4
  vsubsd bodies+16(%rax), %xmm3, %xmm3
  vmulsd bodies+48(%rax), %xmm6, %xmm0
  vmovsd 48(%rcx), %xmm6
  vmulsd %xmm5, %xmm5, %xmm5
  vfmadd132sd %xmm4, %xmm5, %xmm4
  vfmadd132sd %xmm3, %xmm4, %xmm3
  vsqrtsd %xmm3, %xmm3, %xmm3
  vdivsd %xmm3, %xmm0, %xmm0
  vmovsd 32(%rcx), %xmm3
  vmulsd %xmm3, %xmm3, %xmm3
  vfmadd132sd %xmm2, %xmm3, %xmm2
  vfmadd132sd %xmm1, %xmm2, %xmm1
  vsubsd %xmm0, %xmm12, %xmm0
  vmulsd %xmm11, %xmm6, %xmm12
  vfmadd132sd %xmm1, %xmm0, %xmm12
  subl $1, %edi
  je .L13
  addl $1, %r8d
  addq $56, %rcx
  jmp .L2
.L12:
  vmovsd 32(%rcx), %xmm2
  addl $1, %r8d
  subl $1, %edi
  addq $56, %rcx
  vmovsd -32(%rcx), %xmm1
  vmovsd -8(%rcx), %xmm6
  vmulsd %xmm2, %xmm2, %xmm2
  vmovsd -16(%rcx), %xmm0
  vmulsd %xmm11, %xmm6, %xmm3
  vfmadd132sd %xmm1, %xmm2, %xmm1
  vfmadd132sd %xmm0, %xmm1, %xmm0
  vfmadd231sd %xmm0, %xmm3, %xmm12
  jmp .L2
.L13:
  vmovapd %xmm12, %xmm0
  ret
.L8:
  movl $4, %eax
  jmp .L3
.LC4:
  .string "%.9f\n"
main:
  subq $8, %rsp
  movl $bodies, %eax
  vxorpd %xmm6, %xmm6, %xmm6
  vxorpd %xmm5, %xmm5, %xmm5
.L15:
  vmovupd 24(%rax), %xmm1
  vmovupd 40(%rax), %xmm7
  vmovhps 80(%rax), %xmm1, %xmm2
  vmovq 104(%rax), %xmm3
  vmovupd 88(%rax), %xmm0
  addq $112, %rax
  vshufpd $1, %xmm3, %xmm7, %xmm3
  vmulpd %xmm3, %xmm2, %xmm2
  vshufpd $1, %xmm0, %xmm1, %xmm1
  vmulpd %xmm3, %xmm1, %xmm1
  vmovsd %xmm7, %xmm0, %xmm0
  vmulpd %xmm3, %xmm0, %xmm0
  vunpcklpd %xmm1, %xmm2, %xmm4
  vunpckhpd %xmm1, %xmm2, %xmm2
  vaddpd %xmm6, %xmm4, %xmm4
  vaddsd %xmm0, %xmm5, %xmm1
  vunpckhpd %xmm0, %xmm0, %xmm0
  vaddpd %xmm2, %xmm4, %xmm6
  vaddsd %xmm1, %xmm0, %xmm5
  cmpq $bodies+224, %rax
  jne .L15
  vmovupd bodies+248(%rip), %xmm7
  vmovlpd bodies+272(%rip), %xmm7, %xmm1
  vmovhps bodies+272(%rip), %xmm7, %xmm2
  vmovsd bodies+272(%rip), %xmm0
  vfnmsub132sd bodies+264(%rip), %xmm5, %xmm0
  vfnmsub132pd %xmm2, %xmm6, %xmm1
  vmovddup .LC3(%rip), %xmm2
  vdivsd .LC3(%rip), %xmm0, %xmm0
  vmovsd %xmm0, bodies+40(%rip)
  vdivpd %xmm2, %xmm1, %xmm1
  vmovupd %xmm1, bodies+24(%rip)
  call energy
  movl $.LC4, %edi
  movl $1, %eax
  call printf
  vmovsd .LC5(%rip), %xmm7
  movl $5000000, %edi
  vmovddup %xmm7, %xmm8
.L16:
  movl $bodies+56, %edx
  movl $1, %esi
.L21:
  vmovsd -8(%rdx), %xmm4
  vmovupd -56(%rdx), %xmm6
  movq %rdx, %rax
  movq %rsi, %rcx
  vmovsd -40(%rdx), %xmm12
  vmovddup %xmm4, %xmm5
.L18:
  vsubpd (%rax), %xmm6, %xmm11
  vsubsd 16(%rax), %xmm12, %xmm3
  addq $1, %rcx
  addq $56, %rax
  vmovsd -8(%rax), %xmm2
  vmulpd %xmm11, %xmm5, %xmm9
  vunpckhpd %xmm11, %xmm11, %xmm0
  vmovapd %xmm11, %xmm1
  vmulsd %xmm0, %xmm0, %xmm0
  vfmadd132sd %xmm11, %xmm0, %xmm1
  vfmadd231sd %xmm3, %xmm3, %xmm1
  vsqrtsd %xmm1, %xmm1, %xmm0
  vmulsd %xmm1, %xmm0, %xmm0
  vmovddup %xmm2, %xmm1
  vmulpd %xmm11, %xmm1, %xmm1
  vmulsd %xmm2, %xmm3, %xmm2
  vmulsd %xmm4, %xmm3, %xmm3
  vdivsd %xmm0, %xmm7, %xmm0
  vfnmadd213sd -16(%rdx), %xmm0, %xmm2
  vmovddup %xmm0, %xmm10
  vfnmadd213pd -32(%rdx), %xmm10, %xmm1
  vmovsd %xmm2, -16(%rdx)
  vmovupd %xmm1, -32(%rdx)
  vfmadd213pd -32(%rax), %xmm9, %xmm10
  vfmadd213sd -16(%rax), %xmm3, %xmm0
  vmovupd %xmm10, -32(%rax)
  vmovsd %xmm0, -16(%rax)
  cmpl $5, %ecx
  jne .L18
  addq $1, %rsi
  addq $56, %rdx
  cmpq $5, %rsi
  jne .L21
  movl $bodies, %eax
.L17:
  vmovapd 16(%rax), %xmm2
  vmovapd 32(%rax), %xmm3
  vmovhps 56(%rax), %xmm2, %xmm4
  addq $112, %rax
  vmovapd -32(%rax), %xmm0
  vshufpd $1, %xmm3, %xmm2, %xmm2
  vfmadd213pd -112(%rax), %xmm8, %xmm2
  vshufpd $1, %xmm0, %xmm3, %xmm1
  vshufpd $1, -16(%rax), %xmm0, %xmm0
  vfmadd132pd %xmm8, %xmm4, %xmm1
  vfmadd213pd -48(%rax), %xmm8, %xmm0
  vmovapd %xmm2, -112(%rax)
  vmovlpd %xmm1, -96(%rax)
  vshufpd $1, %xmm0, %xmm1, %xmm1
  vmovhpd %xmm0, -40(%rax)
  vmovupd %xmm1, -56(%rax)
  cmpq $bodies+224, %rax
  jne .L17
  vmovupd bodies+248(%rip), %xmm0
  vfmadd213pd bodies+224(%rip), %xmm8, %xmm0
  vmovapd %xmm0, bodies+224(%rip)
  vmovsd bodies+264(%rip), %xmm0
  vfmadd213sd bodies+240(%rip), %xmm7, %xmm0
  vmovsd %xmm0, bodies+240(%rip)
  subl $1, %edi
  jne .L16
  call energy
  movl $.LC4, %edi
  movl $1, %eax
  call printf
  xorl %eax, %eax
  addq $8, %rsp
  ret
bodies:
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long 0
  .long -910277154
  .long 1078181180
  .long 876402988
  .long 1075010976
  .long -1071654020
  .long -1074622293
  .long 1814424560
  .long -1078294791
  .long -1684812612
  .long 1071867654
  .long -176319333
  .long 1074167538
  .long -1705375979
  .long -1080438057
  .long -643091496
  .long 1067666581
  .long -1020081561
  .long 1075883981
  .long 836633008
  .long 1074823115
  .long -504674692
  .long -1076243629
  .long -1199074238
  .long -1074779103
  .long -1088450797
  .long 1073559017
  .long 1594958772
  .long 1065434184
  .long 218613303
  .long 1065819465
  .long -827860529
  .long 1076480490
  .long -702126466
  .long -1070712600
  .long -1104839264
  .long -1077111465
  .long -1450107921
  .long 1072780060
  .long 1045740485
  .long 1072417919
  .long -84787588
  .long -1081725077
  .long -1661722957
  .long 1063009746
  .long -1459267798
  .long 1076806247
  .long 868786720
  .long -1069946024
  .long -1817451200
  .long 1070002675
  .long 374979658
  .long 1072649398
  .long 834993059
  .long 1071843270
  .long 1484154358
  .long -1079915640
  .long 1394055596
  .long 1063299315
.LC0:
  .long 0
  .long 1071644672
.LC3:
  .long -910277154
  .long 1078181180
.LC5:
  .long 1202590843
  .long 1065646817
