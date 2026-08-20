.LCPI0_0:
  .long 1321528399
.LCPI0_1:
  .long 26
.LCPI0_3:
  .byte 97
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $264, %rsp
  vstmxcsr 64(%rsp)
  orl $32832, 64(%rsp)
  vldmxcsr 64(%rsp)
  movl $42, %edx
  movl $strings, %eax
  xorl %ecx, %ecx
  vpbroadcastd .LCPI0_0(%rip), %ymm0
  vpbroadcastd .LCPI0_1(%rip), %ymm1
  vpbroadcastb .LCPI0_3(%rip), %xmm2
  jmp .LBB0_1
.LBB0_6:
  imulq $200, %rcx, %rdi
  movb $0, strings(%rdi,%rsi)
  leaq 1(%rcx), %rsi
  addq $200, %rax
  cmpq $99999, %rcx
  movq %rsi, %rcx
  je .LBB0_7
.LBB0_1:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  movl %edx, %esi
  shrl $2, %esi
  imulq $381774871, %rsi, %rsi
  shrq $34, %rsi
  imull $180, %esi, %esi
  movl %edx, %edi
  subl %esi, %edi
  leaq 10(%rdi), %rsi
  movl %esi, %r8d
  shrl $3, %r8d
  xorl %r9d, %r9d
.LBB0_2:
  imull $1664525, %edx, %r10d
  addl $1013904223, %r10d
  imull $1664525, %r10d, %r11d
  addl $1013904223, %r11d
  imull $1664525, %r11d, %ebx
  addl $1013904223, %ebx
  imull $1664525, %ebx, %ebp
  addl $1013904223, %ebp
  imull $1664525, %ebp, %r14d
  addl $1013904223, %r14d
  imull $1664525, %r14d, %r15d
  addl $1013904223, %r15d
  imull $1664525, %r15d, %r12d
  addl $1013904223, %r12d
  imull $1664525, %r12d, %edx
  vmovd %r10d, %xmm3
  vpinsrd $1, %r11d, %xmm3, %xmm3
  vpinsrd $2, %ebx, %xmm3, %xmm3
  addl $1013904223, %edx
  vpinsrd $3, %ebp, %xmm3, %xmm3
  vmovd %r14d, %xmm4
  vpinsrd $1, %r15d, %xmm4, %xmm4
  vpinsrd $2, %r12d, %xmm4, %xmm4
  vpinsrd $3, %edx, %xmm4, %xmm4
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
  vmovq %xmm3, (%rax,%r9,8)
  incq %r9
  cmpq %r9, %r8
  jne .LBB0_2
  movl %esi, %r8d
  andl $504, %r8d
  cmpq %rsi, %r8
  jae .LBB0_6
  subq %r8, %rdi
  addq $10, %rdi
  addq %rax, %r8
  xorl %r9d, %r9d
.LBB0_5:
  imull $1664525, %edx, %edx
  addl $1013904223, %edx
  imulq $1321528399, %rdx, %r10
  shrq $35, %r10
  leal (%r10,%r10,4), %r11d
  leal (%r11,%r11,4), %r11d
  addl %r10d, %r11d
  movl %edx, %r10d
  subl %r11d, %r10d
  addb $97, %r10b
  movb %r10b, (%r8,%r9)
  incq %r9
  cmpq %r9, %rdi
  jne .LBB0_5
  jmp .LBB0_6
.LBB0_7:
  xorl %r13d, %r13d
  xorl %ebx, %ebx
.LBB0_8:
  xorl %ebp, %ebp
.LBB0_9:
  leaq strings(%rbp), %rdi
  vzeroupper
  callq strlen
  movq %rax, %r14
  addq %rbx, %r14
  leaq strings+200(%rbp), %rdi
  callq strlen
  movq %rax, %rbx
  leaq strings+400(%rbp), %rdi
  callq strlen
  movq %rax, %r15
  addq %rbx, %r15
  addq %r14, %r15
  leaq strings+600(%rbp), %rdi
  callq strlen
  movq %rax, %rbx
  leaq strings+800(%rbp), %rdi
  callq strlen
  movq %rax, %r14
  addq %rbx, %r14
  leaq strings+1000(%rbp), %rdi
  callq strlen
  movq %rax, %r12
  addq %r14, %r12
  addq %r15, %r12
  leaq strings+1200(%rbp), %rdi
  callq strlen
  movq %rax, %r14
  leaq strings+1400(%rbp), %rdi
  callq strlen
  movq %rax, %rbx
  addq %r14, %rbx
  addq %r12, %rbx
  addq $1600, %rbp
  cmpq $20000000, %rbp
  jne .LBB0_9
  leal 1(%r13), %eax
  cmpl $49, %r13d
  movl %eax, %r13d
  jne .LBB0_8
  xorl %r12d, %r12d
  xorl %r15d, %r15d
.LBB0_12:
  leaq strings(%r12), %rdi
  leaq strings+200(%r12), %r14
  movq %r14, %rsi
  callq strcmp
  movslq %eax, %rbp
  addq %r15, %rbp
  leaq strings+400(%r12), %r15
  movq %r14, %rdi
  movq %r15, %rsi
  callq strcmp
  cltq
  movq %rax, (%rsp)
  leaq strings+600(%r12), %r14
  movq %r15, %rdi
  movq %r14, %rsi
  callq strcmp
  movslq %eax, %r13
  addq (%rsp), %r13
  addq %rbp, %r13
  leaq strings+800(%r12), %r15
  movq %r14, %rdi
  movq %r15, %rsi
  callq strcmp
  cltq
  movq %rax, (%rsp)
  leaq strings+1000(%r12), %r14
  movq %r15, %rdi
  movq %r14, %rsi
  callq strcmp
  movslq %eax, %rbp
  addq (%rsp), %rbp
  leaq strings+1200(%r12), %rsi
  movq %rsi, (%rsp)
  movq %r14, %rdi
  callq strcmp
  movslq %eax, %r15
  addq %rbp, %r15
  addq %r13, %r15
  leaq strings+1400(%r12), %r14
  movq (%rsp), %rdi
  movq %r14, %rsi
  callq strcmp
  movslq %eax, %r13
  leaq strings+1600(%r12), %rsi
  movq %r14, %rdi
  callq strcmp
  cltq
  addq %r13, %rax
  addq %r15, %rax
  movq %rax, %r15
  addq $1600, %r12
  cmpq $19998400, %r12
  jne .LBB0_12
  movq %r15, 48(%rsp)
  movl $19998400, %eax
  leaq strings(%rax), %rdi
  movl $19998600, %eax
  leaq strings(%rax), %r14
  movq %r14, %rsi
  callq strcmp
  movl %eax, 36(%rsp)
  movl $19998800, %eax
  leaq strings(%rax), %r12
  movq %r14, %rdi
  movq %r12, %rsi
  callq strcmp
  movl %eax, 32(%rsp)
  movl $19999000, %eax
  leaq strings(%rax), %r14
  movq %r12, %rdi
  movq %r14, %rsi
  callq strcmp
  movl %eax, 28(%rsp)
  movl $19999200, %eax
  leaq strings(%rax), %r12
  movq %r14, %rdi
  movq %r12, %rsi
  callq strcmp
  movl %eax, 24(%rsp)
  movl $19999400, %eax
  leaq strings(%rax), %r14
  movq %r12, %rdi
  movq %r14, %rsi
  callq strcmp
  movl %eax, 20(%rsp)
  movl $19999600, %eax
  leaq strings(%rax), %r12
  movq %r14, %rdi
  movq %r12, %rsi
  callq strcmp
  movl %eax, 16(%rsp)
  movl $19999800, %eax
  leaq strings(%rax), %rsi
  movq %r12, %rdi
  callq strcmp
  movl %eax, 12(%rsp)
  movl $6513249, 8(%rsp)
  xorl %eax, %eax
  xorl %r9d, %r9d
  jmp .LBB0_14
.LBB0_22:
  xorl %esi, %esi
.LBB0_23:
  addq %rsi, %r9
  incq %rax
  cmpq $100000, %rax
  je .LBB0_24
.LBB0_14:
  imulq $200, %rax, %rcx
  movzbl strings(%rcx), %edx
  testb %dl, %dl
  je .LBB0_22
  leaq strings(%rcx), %rcx
.LBB0_16:
  xorl %edi, %edi
.LBB0_17:
  movzbl 8(%rsp,%rdi), %r8d
  movl $1, %esi
  testb %r8b, %r8b
  je .LBB0_23
  cmpb %r8b, %dl
  jne .LBB0_21
  movzbl 1(%rcx,%rdi), %edx
  incq %rdi
  testb %dl, %dl
  jne .LBB0_17
  cmpb $0, 8(%rsp,%rdi)
  je .LBB0_23
.LBB0_21:
  movzbl 1(%rcx), %edx
  incq %rcx
  testb %dl, %dl
  jne .LBB0_16
  jmp .LBB0_22
.LBB0_24:
  movq %r9, 40(%rsp)
  movq %rbx, 56(%rsp)
  movabsq $4294967296, %r12
  xorl %r13d, %r13d
  leaq 64(%rsp), %rbx
  xorl %eax, %eax
.LBB0_25:
  movq %rax, (%rsp)
  xorl %r15d, %r15d
.LBB0_26:
  leaq strings(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %rbp
  addq %r13, %rbp
  leaq strings+200(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %rbx
  leaq strings+400(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  leaq 64(%rsp), %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %r13
  addq %rbx, %r13
  addq %rbp, %r13
  leaq strings+600(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  leaq 64(%rsp), %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %rbp
  leaq strings+800(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  leaq 64(%rsp), %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %rbx
  addq %rbp, %rbx
  leaq strings+1000(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  leaq 64(%rsp), %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %rbp
  addq %rbx, %rbp
  addq %r13, %rbp
  leaq strings+1200(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  leaq 64(%rsp), %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %rbx
  leaq strings+1400(%r15), %r14
  movq %r14, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  leaq 64(%rsp), %rdi
  movq %r14, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 64(%rsp), %r13
  addq %rbx, %r13
  leaq 64(%rsp), %rbx
  addq %rbp, %r13
  addq $1600, %r15
  cmpq $20000000, %r15
  jne .LBB0_26
  movq (%rsp), %rcx
  leal 1(%rcx), %eax
  cmpl $49, %ecx
  jne .LBB0_25
  movslq 36(%rsp), %rax
  movq 48(%rsp), %rdx
  addq %rax, %rdx
  movslq 32(%rsp), %rax
  movslq 28(%rsp), %rcx
  addq %rax, %rcx
  addq %rdx, %rcx
  movslq 24(%rsp), %rax
  movslq 20(%rsp), %rdx
  addq %rax, %rdx
  movslq 16(%rsp), %rax
  addq %rdx, %rax
  addq %rcx, %rax
  movslq 12(%rsp), %rdx
  addq %rax, %rdx
  movl $.L.str, %edi
  movq 56(%rsp), %rsi
  movq 40(%rsp), %rcx
  movq %r13, %r8
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $264, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "strlen total: %ld, cmp_sum: %ld, found: %ld, copy_sum: %ld\n"

