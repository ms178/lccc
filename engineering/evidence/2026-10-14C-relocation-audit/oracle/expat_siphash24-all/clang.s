expat_siphash24:
  jmp siphash24

.LCPI1_0:
siphash24:
  pushq %rbx
  subq $64, %rsp
  leaq 40(%rsp), %rax
  movq $0, 40(%rsp)
  vbroadcastf128 (%rdx), %ymm0
  vxorps .LCPI1_0(%rip), %ymm0, %ymm0
  vmovups %ymm0, 8(%rsp)
  leaq 48(%rsp), %rcx
  movq %rax, 48(%rsp)
  movq $0, 56(%rsp)
  addq %rdi, %rsi
  jmp .LBB1_1
.LBB1_6:
  movq 16(%rsp), %r9
  movq 40(%rsp), %r8
  movq 32(%rsp), %r10
  xorq %r8, %r10
  rorxq $51, %r9, %r11
  addq 8(%rsp), %r9
  xorq %r9, %r11
  rorxq $32, %r9, %r9
  rorxq $48, %r10, %rbx
  addq 24(%rsp), %r10
  xorq %r10, %rbx
  addq %rbx, %r9
  rorxq $43, %rbx, %rbx
  xorq %r9, %rbx
  addq %r11, %r10
  rorxq $47, %r11, %r11
  xorq %r10, %r11
  rorxq $32, %r10, %r10
  addq %r11, %r9
  rorxq $51, %r11, %r11
  xorq %r9, %r11
  rorxq $32, %r9, %r9
  addq %rbx, %r10
  rorxq $48, %rbx, %rbx
  xorq %r10, %rbx
  addq %rbx, %r9
  rorxq $43, %rbx, %rbx
  xorq %r9, %rbx
  addq %r11, %r10
  rorxq $47, %r11, %r11
  xorq %r10, %r11
  addq $8, 56(%rsp)
  xorq %r8, %r9
  rorxq $32, %r10, %r8
  movq %r11, 16(%rsp)
  movq %rbx, 32(%rsp)
  movq %r8, 24(%rsp)
  movq %r9, 8(%rsp)
  movq %rax, 48(%rsp)
  cmpq %rcx, %rdx
  jb .LBB1_7
.LBB1_1:
  cmpq %rsi, %rdi
  jae .LBB1_7
  movq %rsi, %r8
  subq %rdi, %r8
.LBB1_3:
  movq 48(%rsp), %rdx
  cmpq %rcx, %rdx
  jae .LBB1_6
  movzbl (%rdi), %r9d
  incq %rdi
  leaq 1(%rdx), %r10
  movq %r10, 48(%rsp)
  movb %r9b, (%rdx)
  decq %r8
  jne .LBB1_3
  movq %rsi, %rdi
  cmpq %rcx, 48(%rsp)
  jae .LBB1_6
.LBB1_7:
  leaq 8(%rsp), %rdi
  vzeroupper
  callq sip24_final
  addq $64, %rsp
  popq %rbx
  retq

.LCPI2_0:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $280, %rsp
  vmovups .L__const.main.key(%rip), %xmm0
  vmovaps %xmm0, 96(%rsp)
  movl $376, %eax
  movl $7, %ecx
.LBB2_1:
  imull $1664525, %eax, %edx
  addl $1013904223, %edx
  shrl $24, %edx
  movb %dl, 121(%rsp,%rcx)
  imull $389569705, %eax, %edx
  addl $1196435762, %edx
  shrl $24, %edx
  movb %dl, 122(%rsp,%rcx)
  imull $-1354167659, %eax, %edx
  addl $-775096599, %edx
  shrl $24, %edx
  movb %dl, 123(%rsp,%rcx)
  imull $158984081, %eax, %edx
  addl $-1426500812, %edx
  shrl $24, %edx
  movb %dl, 124(%rsp,%rcx)
  imull $-1432516515, %eax, %edx
  addl $1649599747, %edx
  shrl $24, %edx
  movb %dl, 125(%rsp,%rcx)
  imull $-1083573575, %eax, %edx
  addl $-1624324474, %edx
  shrl $24, %edx
  movb %dl, 126(%rsp,%rcx)
  imull $1851289957, %eax, %edx
  addl $1476291629, %edx
  shrl $24, %edx
  movb %dl, 127(%rsp,%rcx)
  imull $-360120287, %eax, %eax
  addl $-1546035288, %eax
  movl %eax, %edx
  shrl $24, %edx
  movb %dl, 128(%rsp,%rcx)
  addq $8, %rcx
  cmpq $151, %rcx
  jne .LBB2_1
  movabsq $506097522914230528, %rax
  movq %rax, 112(%rsp)
  movabsq $1084818905618843912, %rax
  movq %rax, 120(%rsp)
  leaq sip24_valid.vectors(%rip), %r12
  xorl %ebx, %ebx
  movq %rsp, %r14
  leaq 112(%rsp), %r15
.LBB2_3:
  movb %bl, (%rsp,%rbx)
  movq %r14, %rdi
  movq %rbx, %rsi
  movq %r15, %rdx
  callq siphash24
  cmpq (%r12), %rax
  jne .LBB2_32
  incq %rbx
  addq $8, %r12
  cmpq $64, %rbx
  jne .LBB2_3
  leaq 32(%rsp), %r15
  leaq 40(%rsp), %rbp
  movl $1, %eax
  movq %rax, 72(%rsp)
  leaq 128(%rsp), %rbx
  xorl %r14d, %r14d
  leaq .L__const.main.key(%rip), %rdx
.LBB2_6:
  leaq 128(%rsp), %rdi
  movq %r14, %rsi
  callq expat_siphash24
  leaq 128(%rsp), %rdx
  movq %rax, 88(%rsp)
  movq %r14, 80(%rsp)
  leaq (%rsp,%r14), %r12
  addq $128, %r12
  movq %rdx, %r13
  xorl %r14d, %r14d
.LBB2_7:
  movq $0, 32(%rsp)
  vmovaps .LCPI2_0(%rip), %ymm0
  vmovups %ymm0, (%rsp)
  movq %r15, 40(%rsp)
  movq $0, 48(%rsp)
  leaq (%rsp,%r14), %rax
  addq $128, %rax
  leaq (%rsp,%r14), %rcx
  addq $128, %rcx
  jmp .LBB2_8
.LBB2_13:
  movq %rsi, %rdi
  cmpq %rbp, %rdi
  jb .LBB2_16
.LBB2_14:
  movq 8(%rsp), %r8
  movq 32(%rsp), %rdi
  movq 24(%rsp), %r9
  xorq %rdi, %r9
  rorxq $51, %r8, %r10
  addq (%rsp), %r8
  xorq %r8, %r10
  rorxq $32, %r8, %r8
  rorxq $48, %r9, %r11
  addq 16(%rsp), %r9
  xorq %r9, %r11
  addq %r11, %r8
  rorxq $43, %r11, %r11
  xorq %r8, %r11
  addq %r10, %r9
  rorxq $47, %r10, %r10
  xorq %r9, %r10
  rorxq $32, %r9, %r9
  addq %r10, %r8
  rorxq $51, %r10, %r10
  xorq %r8, %r10
  rorxq $32, %r8, %r8
  addq %r11, %r9
  rorxq $48, %r11, %r11
  xorq %r9, %r11
  addq %r11, %r8
  rorxq $43, %r11, %r11
  xorq %r8, %r11
  addq %r10, %r9
  rorxq $47, %r10, %r10
  xorq %r9, %r10
  addq $8, 48(%rsp)
  xorq %rdi, %r8
  rorxq $32, %r9, %rdi
  movq %r10, 8(%rsp)
  movq %r11, 24(%rsp)
  movq %rdi, 16(%rsp)
  movq %r8, (%rsp)
  movq %r15, 40(%rsp)
  cmpq %rbp, %rsi
  jb .LBB2_15
.LBB2_8:
  cmpq %rax, %rdx
  jae .LBB2_15
  movq %r13, %rdi
  subq %rdx, %rdi
.LBB2_10:
  movq 40(%rsp), %rsi
  cmpq %rbp, %rsi
  jae .LBB2_13
  movzbl (%rdx), %r8d
  incq %rdx
  leaq 1(%rsi), %r9
  movq %r9, 40(%rsp)
  movb %r8b, (%rsi)
  decq %rdi
  jne .LBB2_10
  movq 40(%rsp), %rdi
  movq %rcx, %rdx
  cmpq %rbp, %rdi
  jae .LBB2_14
  jmp .LBB2_16
.LBB2_15:
  movq %r15, %rdi
.LBB2_16:
  movq 80(%rsp), %rcx
  subq %r14, %rcx
  addq %rax, %rcx
  jmp .LBB2_17
.LBB2_22:
  xorl %edx, %edx
  cmpq %rbp, %rdi
  jb .LBB2_25
.LBB2_24:
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
  rorxq $32, %r8, %r8
  movq %r9, 8(%rsp)
  movq %r10, 24(%rsp)
  movq %r8, 16(%rsp)
  xorq %rsi, %rdi
  movq %rdi, (%rsp)
  movq %r15, 40(%rsp)
  addq $8, 48(%rsp)
  movq %r15, %rdi
  testb %dl, %dl
  je .LBB2_25
.LBB2_17:
  cmpq %r12, %rax
  jae .LBB2_22
  movq %rbx, %rdx
  subq %rax, %rdx
.LBB2_19:
  movq 40(%rsp), %rdi
  cmpq %rbp, %rdi
  jae .LBB2_23
  movzbl (%rax), %esi
  incq %rax
  leaq 1(%rdi), %r8
  movq %r8, 40(%rsp)
  movb %sil, (%rdi)
  decq %rdx
  jne .LBB2_19
  movq 40(%rsp), %rdi
  xorl %edx, %edx
  movq %rcx, %rax
  cmpq %rbp, %rdi
  jae .LBB2_24
  jmp .LBB2_25
.LBB2_23:
  movb $1, %dl
  cmpq %rbp, %rdi
  jae .LBB2_24
.LBB2_25:
  movq %rsp, %rdi
  vzeroupper
  callq sip24_final
  cmpq 88(%rsp), %rax
  jne .LBB2_32
  incq %r14
  incq %r13
  cmpq 72(%rsp), %r14
  leaq 128(%rsp), %rdx
  jne .LBB2_7
  movq 80(%rsp), %r14
  incq %r14
  incq 72(%rsp)
  incq %rbx
  cmpq $129, %r14
  leaq .L__const.main.key(%rip), %rdx
  jne .LBB2_6
  movabsq $-7046029254386353131, %rax
  xorl %ebp, %ebp
  leaq 96(%rsp), %rbx
  movabsq $506097522914230528, %r15
.LBB2_29:
  movq %rax, %r14
  movq %rax, %rsi
  shrq $32, %rsi
  andl $127, %esi
  andl $7, %eax
  movq %r14, %rcx
  orq $1, %rcx
  addq %rcx, %r15
  movq %r15, 96(%rsp)
  movq %r14, %rcx
  shrq $48, %rcx
  movl %ebp, %edx
  andl $127, %edx
  xorb %cl, 128(%rsp,%rdx)
  leaq (%rsp,%rax), %rdi
  addq $128, %rdi
  movq %rbx, %rdx
  callq expat_siphash24
  xorq %r14, %rax
  incl %ebp
  cmpl $2000000, %ebp
  jne .LBB2_29
  leaq .L.str(%rip), %rdi
  movq %rax, %rsi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  jmp .LBB2_33
.LBB2_32:
  movl $1, %eax
.LBB2_33:
  addq $280, %rsp
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
  movzbl %dl, %edx
  decl %edx
  cmpl $6, %edx
  ja .LBB3_9
  leaq .LJTI3_0(%rip), %rsi
  movslq (%rsi,%rdx,4), %rdx
  addq %rsi, %rdx
  jmpq *%rdx
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
  shll $24, %edx
  orq %rdx, %rax
.LBB3_6:
  movzbl 34(%rdi), %edx
  shll $16, %edx
  orq %rdx, %rax
.LBB3_7:
  movzbl 33(%rdi), %edx
  shll $8, %edx
  orq %rdx, %rax
.LBB3_8:
  movzbl (%rcx), %ecx
  orq %rcx, %rax
.LBB3_9:
  movq 24(%rdi), %rcx
  xorq %rax, %rcx
  movq 8(%rdi), %rdx
  movq (%rdi), %rsi
  addq %rdx, %rsi
  rorxq $51, %rdx, %rdx
  xorq %rsi, %rdx
  rorxq $48, %rcx, %r8
  addq 16(%rdi), %rcx
  rorxq $32, %rsi, %rsi
  xorq %rcx, %r8
  addq %r8, %rsi
  rorxq $43, %r8, %r8
  xorq %rsi, %r8
  addq %rdx, %rcx
  rorxq $47, %rdx, %rdx
  xorq %rcx, %rdx
  rorxq $32, %rcx, %rcx
  addq %rdx, %rsi
  rorxq $51, %rdx, %rdx
  xorq %rsi, %rdx
  rorxq $32, %rsi, %rsi
  addq %r8, %rcx
  rorxq $48, %r8, %r8
  xorq %rcx, %r8
  addq %r8, %rsi
  rorxq $43, %r8, %r8
  xorq %rsi, %r8
  addq %rdx, %rcx
  rorxq $47, %rdx, %rdx
  xorq %rcx, %rdx
  rorxq $32, %rcx, %rcx
  xorq %rax, %rsi
  xorq $255, %rcx
  addq %rdx, %rsi
  rorxq $51, %rdx, %rax
  xorq %rsi, %rax
  rorxq $32, %rsi, %rdx
  addq %r8, %rcx
  rorxq $48, %r8, %rsi
  xorq %rcx, %rsi
  addq %rsi, %rdx
  rorxq $43, %rsi, %rsi
  xorq %rdx, %rsi
  addq %rax, %rcx
  rorxq $47, %rax, %rax
  xorq %rcx, %rax
  rorxq $32, %rcx, %rcx
  addq %rax, %rdx
  rorxq $51, %rax, %rax
  xorq %rdx, %rax
  rorxq $32, %rdx, %rdx
  addq %rsi, %rcx
  rorxq $48, %rsi, %rsi
  xorq %rcx, %rsi
  addq %rsi, %rdx
  rorxq $43, %rsi, %rsi
  xorq %rdx, %rsi
  addq %rax, %rcx
  rorxq $47, %rax, %rax
  xorq %rcx, %rax
  rorxq $32, %rcx, %rcx
  addq %rax, %rdx
  rorxq $51, %rax, %rax
  xorq %rdx, %rax
  rorxq $32, %rdx, %rdx
  addq %rsi, %rcx
  rorxq $48, %rsi, %rsi
  xorq %rcx, %rsi
  addq %rsi, %rdx
  rorxq $43, %rsi, %rsi
  xorq %rdx, %rsi
  addq %rax, %rcx
  rorxq $47, %rax, %rax
  xorq %rcx, %rax
  rorxq $32, %rcx, %rcx
  addq %rax, %rdx
  rorxq $51, %rax, %r8
  xorq %rdx, %r8
  rorxq $32, %rdx, %rdx
  addq %rsi, %rcx
  rorxq $48, %rsi, %rax
  xorq %rcx, %rax
  addq %rax, %rdx
  rorxq $43, %rax, %rax
  xorq %rdx, %rax
  addq %r8, %rcx
  rorxq $47, %r8, %rsi
  xorq %rcx, %rsi
  rorxq $32, %rcx, %rcx
  movq %rsi, 8(%rdi)
  movq %rdx, (%rdi)
  movq %rax, 24(%rdi)
  movq %rcx, 16(%rdi)
  xorq %rdx, %rsi
  xorq %rcx, %rax
  xorq %rsi, %rax
  retq
.LJTI3_0:

.L__const.main.key:

.L.str:

sip24_valid.vectors:

