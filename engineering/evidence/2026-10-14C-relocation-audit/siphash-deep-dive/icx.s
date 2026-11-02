expat_siphash24:
  movq (%rdx), %rax
  movq 8(%rdx), %rcx
  movq %rax, %rdx
  jmp siphash24

.LCPI1_0:
siphash24:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %rbx
  subq $56, %rsp
  leaq 32(%rsp), %rax
  movq $0, 32(%rsp)
  vmovq %rcx, %xmm0
  vmovq %rdx, %xmm1
  vpunpcklqdq %xmm0, %xmm1, %xmm0
  vpermq $68, %ymm0, %ymm0
  vpxor .LCPI1_0(%rip), %ymm0, %ymm0
  vmovdqu %ymm0, (%rsp)
  leaq 40(%rsp), %rcx
  movq %rax, 40(%rsp)
  movq $0, 48(%rsp)
  leaq (%rdi,%rsi), %rdx
  movq %rdi, %r8
.LBB1_1:
  cmpq %rdx, %r8
  jae .LBB1_16
  movq %rdi, %rbx
  subq %r8, %rbx
  addq %rsi, %rbx
  movq %rax, %r11
  cmpq $2, %rbx
  jb .LBB1_8
  movq %rbx, %r9
  shrq %r9
  movl $1, %r10d
.LBB1_4:
  movq 40(%rsp), %r11
  cmpq %rcx, %r11
  jae .LBB1_12
  movzbl -1(%r8,%r10), %ebp
  leaq 1(%r11), %r14
  movq %r14, 40(%rsp)
  movb %bpl, (%r11)
  movq 40(%rsp), %r11
  cmpq %rcx, %r11
  jae .LBB1_13
  movzbl (%r8,%r10), %ebp
  leaq 1(%r11), %r14
  movq %r14, 40(%rsp)
  movb %bpl, (%r11)
  addq $2, %r10
  decq %r9
  jne .LBB1_4
  movq 40(%rsp), %r11
.LBB1_8:
  movq %rbx, %r10
  andq $-2, %r10
  movb $1, %r9b
  cmpq %rbx, %r10
  jae .LBB1_11
  cmpq %rcx, %r11
  jae .LBB1_13
  movzbl (%r8,%r10), %r10d
  leaq 1(%r11), %r14
  movq %r14, 40(%rsp)
  movb %r10b, (%r11)
  movq 40(%rsp), %r11
.LBB1_11:
  movq %rbx, %r10
  cmpq %rcx, %r11
  jae .LBB1_15
  jmp .LBB1_16
.LBB1_12:
  decq %r10
.LBB1_13:
  xorl %r9d, %r9d
  cmpq %rcx, %r11
  jb .LBB1_16
.LBB1_15:
  addq %r10, %r8
  movq 8(%rsp), %r11
  movq 32(%rsp), %r10
  movq 24(%rsp), %rbx
  xorq %r10, %rbx
  rorxq $51, %r11, %r14
  addq (%rsp), %r11
  xorq %r11, %r14
  rorxq $32, %r11, %r11
  rorxq $48, %rbx, %r15
  addq 16(%rsp), %rbx
  xorq %rbx, %r15
  addq %r15, %r11
  rorxq $43, %r15, %r15
  xorq %r11, %r15
  addq %r14, %rbx
  rorxq $47, %r14, %r14
  xorq %rbx, %r14
  rorxq $32, %rbx, %rbx
  addq %r14, %r11
  rorxq $51, %r14, %r14
  xorq %r11, %r14
  rorxq $32, %r11, %r11
  addq %r15, %rbx
  rorxq $48, %r15, %r15
  xorq %rbx, %r15
  addq %r15, %r11
  rorxq $43, %r15, %r15
  xorq %r11, %r15
  addq %r14, %rbx
  rorxq $47, %r14, %r14
  xorq %rbx, %r14
  rorxq $32, %rbx, %rbx
  movq %r14, 8(%rsp)
  movq %r15, 24(%rsp)
  movq %rbx, 16(%rsp)
  xorq %r10, %r11
  movq %r11, (%rsp)
  movq %rax, 40(%rsp)
  addq $8, 48(%rsp)
  testb %r9b, %r9b
  je .LBB1_1
.LBB1_16:
  movq %rsp, %rdi
  vzeroupper
  callq sip24_final
  addq $56, %rsp
  popq %rbx
  popq %r14
  popq %r15
  popq %rbp
  retq

.LCPI2_1:
.LCPI2_2:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $248, %rsp
  vstmxcsr 96(%rsp)
  orl $32832, 96(%rsp)
  vldmxcsr 96(%rsp)
  vmovups .L__const.main.key(%rip), %xmm0
  vmovups %xmm0, 64(%rsp)
  movl $376, %ecx
  xorl %eax, %eax
  vpbroadcastd .LCPI2_2(%rip), %xmm0
.LBB2_1:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  vmovd %ecx, %xmm1
  vpinsrd $1, %edx, %xmm1, %xmm1
  vpinsrd $2, %esi, %xmm1, %xmm1
  vpinsrd $3, %edi, %xmm1, %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, 96(%rsp,%rax,8)
  imull $1664525, %edi, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %ecx
  vmovd %edx, %xmm1
  vpinsrd $1, %esi, %xmm1, %xmm1
  vpinsrd $2, %edi, %xmm1, %xmm1
  addl $1013904223, %ecx
  vpinsrd $3, %ecx, %xmm1, %xmm1
  vpshufb %xmm0, %xmm1, %xmm1
  vmovd %xmm1, 100(%rsp,%rax,8)
  incq %rax
  cmpq $18, %rax
  jne .LBB2_1
  movabsq $506097522914230528, %rbx
  xorl %r14d, %r14d
  movq %rsp, %r15
  movabsq $1084818905618843912, %r12
.LBB2_3:
  movb %r14b, (%rsp,%r14)
  movq %r15, %rdi
  movq %r14, %rsi
  movq %rbx, %rdx
  movq %r12, %rcx
  callq siphash24
  cmpq sip24_valid.vectors(,%r14,8), %rax
  jne .LBB2_29
  incq %r14
  cmpq $64, %r14
  jne .LBB2_3
  leaq 32(%rsp), %r12
  movl $1, %r14d
  leaq 40(%rsp), %r13
  xorl %ebx, %ebx
  leaq 96(%rsp), %rdi
  leaq 64(%rsp), %rdx
.LBB2_6:
  movq %rbx, %rsi
  callq expat_siphash24
  movq %rax, %rbp
  movq %rbx, 88(%rsp)
  leaq (%rsp,%rbx), %r15
  addq $96, %r15
  xorl %ebx, %ebx
.LBB2_7:
  movq $0, 32(%rsp)
  vmovdqu .LCPI2_1(%rip), %ymm0
  vmovdqu %ymm0, (%rsp)
  movq %r12, 40(%rsp)
  movq $0, 48(%rsp)
  leaq (%rsp,%rbx), %rax
  addq $96, %rax
  leaq 96(%rsp), %rcx
  jmp .LBB2_8
.LBB2_12:
  movq %rdx, %rsi
  cmpq %r13, %rsi
  jb .LBB2_15
.LBB2_13:
  movq 8(%rsp), %rdi
  movq 32(%rsp), %rsi
  movq 24(%rsp), %r8
  xorq %rsi, %r8
  rorxq $51, %rdi, %r9
  addq (%rsp), %rdi
  xorq %rdi, %r9
  rorxq $32, %rdi, %rdi
  rorxq $48, %r8, %r10
  addq 16(%rsp), %r8
  xorq %r8, %r10
  addq %r10, %rdi
  rorxq $43, %r10, %r10
  xorq %rdi, %r10
  addq %r9, %r8
  rorxq $47, %r9, %r9
  xorq %r8, %r9
  rorxq $32, %r8, %r8
  addq %r9, %rdi
  rorxq $51, %r9, %r9
  xorq %rdi, %r9
  rorxq $32, %rdi, %rdi
  addq %r10, %r8
  rorxq $48, %r10, %r10
  xorq %r8, %r10
  addq %r10, %rdi
  rorxq $43, %r10, %r10
  xorq %rdi, %r10
  addq %r9, %r8
  rorxq $47, %r9, %r9
  xorq %r8, %r9
  addq $8, 48(%rsp)
  xorq %rsi, %rdi
  rorxq $32, %r8, %rsi
  movq %r9, 8(%rsp)
  movq %r10, 24(%rsp)
  movq %rsi, 16(%rsp)
  movq %rdi, (%rsp)
  movq %r12, 40(%rsp)
  cmpq %r13, %rdx
  jb .LBB2_14
.LBB2_8:
  cmpq %rax, %rcx
  jae .LBB2_14
.LBB2_9:
  movq 40(%rsp), %rdx
  cmpq %r13, %rdx
  jae .LBB2_12
  movzbl (%rcx), %esi
  incq %rcx
  leaq 1(%rdx), %rdi
  movq %rdi, 40(%rsp)
  movb %sil, (%rdx)
  cmpq %rax, %rcx
  jb .LBB2_9
  movq 40(%rsp), %rsi
  cmpq %r13, %rsi
  jae .LBB2_13
  jmp .LBB2_15
.LBB2_14:
  movq %r12, %rsi
  jmp .LBB2_15
.LBB2_19:
  xorl %ecx, %ecx
  cmpq %r13, %rsi
  jb .LBB2_22
.LBB2_21:
  movq 8(%rsp), %rsi
  movq 32(%rsp), %rdx
  movq 24(%rsp), %rdi
  xorq %rdx, %rdi
  rorxq $51, %rsi, %r8
  addq (%rsp), %rsi
  xorq %rsi, %r8
  rorxq $32, %rsi, %rsi
  rorxq $48, %rdi, %r9
  addq 16(%rsp), %rdi
  xorq %rdi, %r9
  addq %r9, %rsi
  rorxq $43, %r9, %r9
  xorq %rsi, %r9
  addq %r8, %rdi
  rorxq $47, %r8, %r8
  xorq %rdi, %r8
  rorxq $32, %rdi, %rdi
  addq %r8, %rsi
  rorxq $51, %r8, %r8
  xorq %rsi, %r8
  rorxq $32, %rsi, %rsi
  addq %r9, %rdi
  rorxq $48, %r9, %r9
  xorq %rdi, %r9
  addq %r9, %rsi
  rorxq $43, %r9, %r9
  xorq %rsi, %r9
  addq %r8, %rdi
  rorxq $47, %r8, %r8
  xorq %rdi, %r8
  rorxq $32, %rdi, %rdi
  movq %r8, 8(%rsp)
  movq %r9, 24(%rsp)
  movq %rdi, 16(%rsp)
  xorq %rdx, %rsi
  movq %rsi, (%rsp)
  movq %r12, 40(%rsp)
  addq $8, 48(%rsp)
  movq %r12, %rsi
  testb %cl, %cl
  je .LBB2_22
.LBB2_15:
  cmpq %r15, %rax
  jae .LBB2_19
.LBB2_16:
  movq 40(%rsp), %rsi
  cmpq %r13, %rsi
  jae .LBB2_20
  movzbl (%rax), %ecx
  incq %rax
  leaq 1(%rsi), %rdx
  movq %rdx, 40(%rsp)
  movb %cl, (%rsi)
  cmpq %r15, %rax
  jb .LBB2_16
  movq 40(%rsp), %rsi
  jmp .LBB2_19
.LBB2_20:
  movb $1, %cl
  cmpq %r13, %rsi
  jae .LBB2_21
.LBB2_22:
  movq %rsp, %rdi
  vzeroupper
  callq sip24_final
  cmpq %rbp, %rax
  jne .LBB2_29
  incq %rbx
  cmpq %r14, %rbx
  jne .LBB2_7
  movq 88(%rsp), %rbx
  incq %rbx
  incq %r14
  cmpq $129, %rbx
  leaq 96(%rsp), %rdi
  leaq 64(%rsp), %rdx
  jne .LBB2_6
  movabsq $-7046029254386353131, %rax
  xorl %ebx, %ebx
  leaq 64(%rsp), %r14
  movabsq $506097522914230528, %r12
.LBB2_26:
  movq %rax, %r15
  movq %rax, %rsi
  shrq $32, %rsi
  andl $127, %esi
  andl $7, %eax
  movq %r15, %rcx
  orq $1, %rcx
  addq %rcx, %r12
  movq %r12, 64(%rsp)
  movq %r15, %rcx
  shrq $48, %rcx
  movl %ebx, %edx
  andl $127, %edx
  xorb %cl, 96(%rsp,%rdx)
  leaq (%rsp,%rax), %rdi
  addq $96, %rdi
  movq %r14, %rdx
  callq expat_siphash24
  xorq %r15, %rax
  incl %ebx
  cmpl $2000000, %ebx
  jne .LBB2_26
  xorl %ebx, %ebx
  movl $.L.str, %edi
  movq %rax, %rsi
  xorl %eax, %eax
  callq printf
  jmp .LBB2_30
.LBB2_29:
  movl $1, %ebx
.LBB2_30:
  movl %ebx, %eax
  addq $248, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

sip24_final:
  movl 40(%rdi), %edx
  leaq 32(%rdi), %rcx
  subl %ecx, %edx
  movl 48(%rdi), %eax
  addl %edx, %eax
  shlq $56, %rax
  movsbl %dl, %edx
  decl %edx
  cmpl $6, %edx
  ja .LBB3_9
  jmpq *.LJTI3_0(,%rdx,8)
.LBB3_2:
  movzbl 38(%rdi), %edx
  shlq $48, %rdx
  orq %rdx, %rax
.LBB3_3:
  movzbl 37(%rdi), %edx
  shlq $40, %rdx
  orq %rdx, %rax
.LBB3_4:
  movzbl 36(%rdi), %edx
  shlq $32, %rdx
  orq %rdx, %rax
.LBB3_5:
  movzbl 35(%rdi), %edx
  shlq $24, %rdx
  orq %rdx, %rax
.LBB3_6:
  movzbl 34(%rdi), %edx
  shlq $16, %rdx
  orq %rdx, %rax
.LBB3_7:
  movzbl 33(%rdi), %edx
  shlq $8, %rdx
  orq %rdx, %rax
.LBB3_8:
  movzbl (%rcx), %ecx
  orq %rcx, %rax
.LBB3_9:
  movq 8(%rdi), %rcx
  movq (%rdi), %rdx
  addq %rcx, %rdx
  rorxq $51, %rcx, %rcx
  xorq %rdx, %rcx
  movq 24(%rdi), %rsi
  xorq %rax, %rsi
  rorxq $48, %rsi, %r8
  addq 16(%rdi), %rsi
  rorxq $47, %rcx, %r9
  addq %rsi, %rcx
  xorq %rcx, %r9
  rorxq $32, %rdx, %rdx
  xorq %r8, %rsi
  addq %rsi, %rdx
  rorxq $51, %r9, %r8
  addq %rdx, %r9
  xorq %r9, %r8
  rorxq $43, %rsi, %rsi
  xorq %rdx, %rsi
  rorxq $32, %rcx, %rdx
  addq %rsi, %rdx
  leaq (%r8,%rdx), %r10
  rorxq $32, %r10, %rcx
  xorq $255, %rcx
  rorxq $32, %r9, %r9
  rorxq $48, %rsi, %rsi
  xorq %rdx, %rsi
  addq %rsi, %r9
  xorq %r9, %rax
  rorxq $47, %r8, %rdx
  xorq %r10, %rdx
  rorxq $43, %rsi, %rsi
  xorq %r9, %rsi
  addq %rdx, %rax
  rorxq $51, %rdx, %rdx
  xorq %rax, %rdx
  rorxq $32, %rax, %rax
  addq %rsi, %rcx
  rorxq $48, %rsi, %rsi
  xorq %rcx, %rsi
  addq %rsi, %rax
  rorxq $43, %rsi, %rsi
  xorq %rax, %rsi
  addq %rdx, %rcx
  rorxq $47, %rdx, %rdx
  xorq %rcx, %rdx
  rorxq $32, %rcx, %rcx
  addq %rdx, %rax
  rorxq $51, %rdx, %rdx
  xorq %rax, %rdx
  rorxq $32, %rax, %rax
  addq %rsi, %rcx
  rorxq $48, %rsi, %rsi
  xorq %rcx, %rsi
  addq %rsi, %rax
  rorxq $43, %rsi, %rsi
  xorq %rax, %rsi
  addq %rdx, %rcx
  rorxq $47, %rdx, %rdx
  xorq %rcx, %rdx
  rorxq $32, %rcx, %rcx
  addq %rdx, %rax
  rorxq $51, %rdx, %rdx
  xorq %rax, %rdx
  rorxq $32, %rax, %rax
  addq %rsi, %rcx
  rorxq $48, %rsi, %rsi
  xorq %rcx, %rsi
  addq %rsi, %rax
  rorxq $43, %rsi, %rsi
  xorq %rax, %rsi
  addq %rdx, %rcx
  rorxq $47, %rdx, %rdx
  xorq %rcx, %rdx
  rorxq $32, %rcx, %rcx
  addq %rdx, %rax
  rorxq $51, %rdx, %rdx
  xorq %rax, %rdx
  rorxq $32, %rax, %r8
  addq %rsi, %rcx
  rorxq $48, %rsi, %rax
  xorq %rcx, %rax
  addq %rax, %r8
  rorxq $43, %rax, %rax
  xorq %r8, %rax
  addq %rdx, %rcx
  rorxq $47, %rdx, %rdx
  xorq %rcx, %rdx
  rorxq $32, %rcx, %rcx
  movq %rdx, 8(%rdi)
  movq %r8, (%rdi)
  movq %rax, 24(%rdi)
  movq %rcx, 16(%rdi)
  xorq %r8, %rdx
  xorq %rcx, %rax
  xorq %rdx, %rax
  retq
.LJTI3_0:

.L__const.main.key:

.L.str:

sip24_valid.vectors:
