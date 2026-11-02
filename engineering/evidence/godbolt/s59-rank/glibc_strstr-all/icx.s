.LCPI0_0:
.LCPI0_1:
.LCPI0_6:
.LCPI0_3:
.LCPI0_5:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r12
  pushq %rbx
  subq $272, %rsp
  vstmxcsr 16(%rsp)
  orl $32832, 16(%rsp)
  vldmxcsr 16(%rsp)
  movl $2084213038, %ecx
  movq $-524288, %rax
  vpbroadcastd .LCPI0_0(%rip), %ymm0
  vpbroadcastd .LCPI0_1(%rip), %ymm1
  vpbroadcastb .LCPI0_5(%rip), %xmm2
.LBB0_1:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %r8d
  addl $1013904223, %r8d
  imull $1664525, %r8d, %r9d
  addl $1013904223, %r9d
  imull $1664525, %r9d, %r10d
  addl $1013904223, %r10d
  vmovd %ecx, %xmm3
  vpinsrd $1, %edx, %xmm3, %xmm3
  vpinsrd $2, %esi, %xmm3, %xmm3
  imull $1664525, %r10d, %ecx
  vpinsrd $3, %edi, %xmm3, %xmm3
  vmovd %r8d, %xmm4
  vpinsrd $1, %r9d, %xmm4, %xmm4
  vpinsrd $2, %r10d, %xmm4, %xmm4
  addl $1013904223, %ecx
  vpinsrd $3, %ecx, %xmm4, %xmm4
  vinserti128 $1, %xmm4, %ymm3, %ymm3
  vpshufd $245, %ymm3, %ymm4
  vpmuludq %ymm0, %ymm4, %ymm4
  vpmuludq %ymm0, %ymm3, %ymm5
  vpshufd $245, %ymm5, %ymm5
  vpblendd $170, %ymm4, %ymm5, %ymm4
  vpsrld $3, %ymm4, %ymm4
  vpmulld %ymm1, %ymm4, %ymm4
  vpsubd %ymm4, %ymm3, %ymm3
  vextracti128 $1, %ymm3, %xmm4
  vpackusdw %xmm4, %xmm3, %xmm3
  vpackuswb %xmm3, %xmm3, %xmm3
  vpaddb %xmm2, %xmm3, %xmm3
  vmovq %xmm3, haystack+524288(%rax)
  addq $8, %rax
  jne .LBB0_1
  movb $0, haystack+524288(%rip)
  leaq 16(%rsp), %rax
  vmovq %rax, %xmm3
  vpbroadcastq %xmm3, %ymm3
  movl $523124044, %eax
  xorl %ecx, %ecx
  vmovdqu .LCPI0_3(%rip), %ymm4
  vpbroadcastd .LCPI0_6(%rip), %xmm5
  xorl %esi, %esi
.LBB0_3:
  xorl %edx, %edx
.LBB0_4:
  movl %edx, %r8d
  movl %edx, %edi
  andl $7, %edi
  imull $1664525, %eax, %eax
  addl $1013904223, %eax
  testb $1, %dl
  jne .LBB0_9
  movl %edx, %r9d
  andl $7, %r9d
  leaq 4(%r9), %r11
  movq %r11, %r10
  andq $8, %r10
  jne .LBB0_28
  xorl %r10d, %r10d
  jmp .LBB0_7
.LBB0_9:
  imull $31337, %eax, %r10d
  movl %r10d, %r9d
  shrl $5, %r9d
  imulq $134225921, %r9, %r9
  shrq $41, %r9
  imull $524256, %r9d, %r9d
  subl %r9d, %r10d
  movl %edx, %r9d
  andl $7, %r9d
  leaq 4(%r9), %rbx
  movq %rbx, %r11
  andq $8, %r11
  jne .LBB0_25
  xorl %r11d, %r11d
  jmp .LBB0_11
.LBB0_28:
  vmovd %eax, %xmm6
  vpbroadcastd %xmm6, %ymm6
  xorl %ebx, %ebx
  xorl %r14d, %r14d
.LBB0_29:
  vmovd %ebx, %xmm7
  vpbroadcastd %xmm7, %ymm7
  vpaddd %ymm4, %ymm7, %ymm7
  vpsrlvd %ymm7, %ymm6, %ymm7
  vpshufd $245, %ymm7, %ymm8
  vpmuludq %ymm0, %ymm8, %ymm8
  vpmuludq %ymm0, %ymm7, %ymm9
  vpshufd $245, %ymm9, %ymm9
  vpblendd $170, %ymm8, %ymm9, %ymm8
  vpsrld $3, %ymm8, %ymm8
  vpmulld %ymm1, %ymm8, %ymm8
  vpsubd %ymm8, %ymm7, %ymm7
  vextracti128 $1, %ymm7, %xmm8
  vpackusdw %xmm8, %xmm7, %xmm7
  vpackuswb %xmm7, %xmm7, %xmm7
  vpaddb %xmm2, %xmm7, %xmm7
  vmovq %xmm7, (%rsp,%r14)
  addq $8, %r14
  addl $24, %ebx
  cmpq %r10, %r14
  jb .LBB0_29
  cmpq %r10, %r11
  je .LBB0_13
.LBB0_7:
  leaq 4(%rdi), %r11
  leal (%r10,%r10,2), %ebx
.LBB0_8:
  shrxl %ebx, %eax, %r14d
  imulq $1321528399, %r14, %r15
  shrq $35, %r15
  leal (%r15,%r15,4), %r12d
  leal (%r12,%r12,4), %ebp
  addl %r15d, %ebp
  subl %ebp, %r14d
  addb $97, %r14b
  movb %r14b, (%rsp,%r10)
  incq %r10
  addl $3, %ebx
  cmpq %r10, %r11
  jne .LBB0_8
  jmp .LBB0_13
.LBB0_25:
  xorl %r14d, %r14d
.LBB0_26:
  movq haystack(%r10,%r14), %r15
  movq %r15, (%rsp,%r14)
  addq $8, %r14
  cmpq %r11, %r14
  jb .LBB0_26
  cmpq %r11, %rbx
  je .LBB0_13
.LBB0_11:
  movq %rdi, %rbx
  subq %r11, %rbx
  addq $4, %rbx
  addq %r11, %r10
  addq $haystack, %r10
  addq %rsp, %r11
  xorl %r14d, %r14d
.LBB0_12:
  movzbl (%r10,%r14), %ebp
  movb %bpl, (%r11,%r14)
  incq %r14
  cmpq %r14, %rbx
  jne .LBB0_12
.LBB0_13:
  andb $7, %r8b
  leal 4(%rdi), %r10d
  vmovd %r10d, %xmm6
  vpbroadcastb %xmm6, %ymm6
  vmovdqu %ymm6, 240(%rsp)
  vmovdqu %ymm6, 208(%rsp)
  vmovdqu %ymm6, 176(%rsp)
  vmovdqu %ymm6, 144(%rsp)
  vmovdqu %ymm6, 112(%rsp)
  vmovdqu %ymm6, 80(%rsp)
  vmovdqu %ymm6, 48(%rsp)
  vmovdqu %ymm6, 16(%rsp)
  addq $3, %r9
  movq %r9, %r11
  andq $12, %r11
  je .LBB0_14
  movl %r8d, %ebx
  xorl %r14d, %r14d
.LBB0_32:
  vpmovzxbq (%rsp,%r14), %ymm6
  vpaddq %ymm6, %ymm3, %ymm6
  vmovd %ebx, %xmm7
  vpbroadcastb %xmm7, %xmm7
  vpaddb %xmm5, %xmm7, %xmm7
  vmovq %xmm6, %r15
  vpextrb $0, %xmm7, (%r15)
  vpextrq $1, %xmm6, %r15
  vpextrb $1, %xmm7, (%r15)
  vextracti128 $1, %ymm6, %xmm6
  vmovq %xmm6, %r15
  vpextrb $2, %xmm7, (%r15)
  vpextrq $1, %xmm6, %r15
  vpextrb $3, %xmm7, (%r15)
  addq $4, %r14
  addb $-4, %bl
  cmpq %r11, %r14
  jb .LBB0_32
  cmpq %r11, %r9
  jne .LBB0_15
  jmp .LBB0_34
.LBB0_14:
  xorl %r11d, %r11d
.LBB0_15:
  movq %rdi, %r9
  subq %r11, %r9
  addq $3, %r9
  subb %r11b, %r8b
  addb $3, %r8b
  addq %rsp, %r11
  xorl %ebx, %ebx
.LBB0_16:
  movzbl (%r11,%rbx), %r14d
  movb %r8b, 16(%rsp,%r14)
  incq %rbx
  decb %r8b
  cmpq %rbx, %r9
  jne .LBB0_16
.LBB0_34:
  movl $524284, %r8d
  subl %edi, %r8d
  addl $3, %edi
  xorl %r9d, %r9d
.LBB0_35:
  movl %edi, %r11d
.LBB0_18:
  movl %r11d, %ebx
  movzbl (%rsp,%rbx), %ebx
  leal (%r9,%r11), %r14d
  cmpb haystack(%r14), %bl
  jne .LBB0_19
  leal -1(%r11), %ebx
  testl %r11d, %r11d
  movl %ebx, %r11d
  jg .LBB0_18
  jmp .LBB0_21
.LBB0_19:
  leal (%r10,%r9), %r11d
  decl %r11d
  movzbl haystack(%r11), %r11d
  movzbl 16(%rsp,%r11), %r11d
  addl %r9d, %r11d
  movl %r11d, %r9d
  cmpl %r8d, %r11d
  jbe .LBB0_35
  imulq $104729, %rdx, %rdi
  jmp .LBB0_22
.LBB0_21:
  movl %r9d, %r9d
  movq %rdx, %rdi
  shlq $24, %rdi
  xorq %r9, %rdi
.LBB0_22:
  addq %rdi, %rsi
  incq %rdx
  cmpq $2048, %rdx
  jne .LBB0_4
  leal 97(%rcx), %edx
  imulq $16381, %rcx, %rdi
  movb %dl, haystack(%rdi)
  incq %rcx
  cmpq $8, %rcx
  jne .LBB0_3
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
  xorl %eax, %eax
  addq $272, %rsp
  popq %rbx
  popq %r12
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

