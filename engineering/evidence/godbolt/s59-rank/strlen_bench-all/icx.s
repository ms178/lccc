.LCPI0_0:
.LCPI0_1:
.LCPI0_3:
.LCPI0_4:
.LCPI0_5:
main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $296, %rsp
  vstmxcsr 96(%rsp)
  orl $32832, 96(%rsp)
  vldmxcsr 96(%rsp)
  movl $42, %edx
  movl $strings, %eax
  xorl %ecx, %ecx
  vpbroadcastd .LCPI0_0(%rip), %ymm0
  vpbroadcastd .LCPI0_1(%rip), %ymm1
  vpbroadcastb .LCPI0_5(%rip), %xmm2
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
  xorl %r14d, %r14d
  vpbroadcastd .LCPI0_3(%rip), %ymm0
  vmovdqu %ymm0, 64(%rsp)
  xorl %r15d, %r15d
.LBB0_12:
  movq %r15, 32(%rsp)
  leaq strings(%r14), %rdi
  leaq strings+200(%r14), %r12
  movq %r12, %rsi
  vzeroupper
  callq strcmp
  movl %eax, 24(%rsp)
  leaq strings+400(%r14), %r15
  movq %r12, %rdi
  movq %r15, %rsi
  callq strcmp
  movl %eax, 16(%rsp)
  leaq strings+600(%r14), %r12
  movq %r15, %rdi
  movq %r12, %rsi
  callq strcmp
  movl %eax, 8(%rsp)
  leaq strings+800(%r14), %r13
  movq %r12, %rdi
  movq %r13, %rsi
  callq strcmp
  movl %eax, 4(%rsp)
  leaq strings+1000(%r14), %rbp
  movq %r13, %rdi
  movq %rbp, %rsi
  callq strcmp
  movl %eax, %r13d
  leaq strings+1200(%r14), %r12
  movq %rbp, %rdi
  movq %r12, %rsi
  callq strcmp
  movl %eax, %ebp
  leaq strings+1400(%r14), %r15
  movq %r12, %rdi
  movq %r15, %rsi
  callq strcmp
  movl %eax, %r12d
  leaq strings+1600(%r14), %rsi
  movq %r15, %rdi
  movq 32(%rsp), %r15
  callq strcmp
  vmovd 24(%rsp), %xmm0
  vpinsrd $1, 16(%rsp), %xmm0, %xmm0
  vpinsrd $2, 8(%rsp), %xmm0, %xmm0
  vpinsrd $3, 4(%rsp), %xmm0, %xmm0
  vmovd %r13d, %xmm1
  vpinsrd $1, %ebp, %xmm1, %xmm1
  vpinsrd $2, %r12d, %xmm1, %xmm1
  vpinsrd $3, %eax, %xmm1, %xmm1
  vinserti128 $1, %xmm1, %ymm0, %ymm0
  vpsrad $31, %ymm0, %ymm1
  vpcmpeqd .LCPI0_4(%rip), %ymm0, %ymm0
  vpandn 64(%rsp), %ymm0, %ymm0
  vpor %ymm0, %ymm1, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpmovsxdq %xmm1, %ymm1
  vpmovsxdq %xmm0, %ymm0
  vpaddq %ymm1, %ymm0, %ymm0
  vextracti128 $1, %ymm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vpshufd $238, %xmm0, %xmm1
  vpaddq %xmm1, %xmm0, %xmm0
  vmovq %xmm0, %rax
  addq %rax, %r15
  addq $1600, %r14
  cmpq $19998400, %r14
  jne .LBB0_12
  movq %rbx, 56(%rsp)
  movl $19998400, %eax
  leaq strings(%rax), %rdi
  movl $19998600, %eax
  leaq strings(%rax), %r14
  movq %r14, %rsi
  vzeroupper
  callq strcmp
  movl %eax, 40(%rsp)
  movl $19998800, %eax
  leaq strings(%rax), %r13
  movq %r14, %rdi
  movq %r13, %rsi
  callq strcmp
  movl %eax, 44(%rsp)
  movl $19999000, %eax
  leaq strings(%rax), %r14
  movq %r13, %rdi
  movq %r14, %rsi
  callq strcmp
  movl %eax, 48(%rsp)
  movl $19999200, %eax
  leaq strings(%rax), %r13
  movq %r14, %rdi
  movq %r13, %rsi
  callq strcmp
  movl %eax, 52(%rsp)
  movl $19999400, %eax
  leaq strings(%rax), %r14
  movq %r13, %rdi
  movq %r14, %rsi
  callq strcmp
  movl %eax, 64(%rsp)
  movl $19999600, %eax
  leaq strings(%rax), %r13
  movq %r14, %rdi
  movq %r13, %rsi
  callq strcmp
  movl %eax, 4(%rsp)
  movl $19999800, %eax
  leaq strings(%rax), %rsi
  movq %r13, %rdi
  callq strcmp
  movl %eax, 8(%rsp)
  movl $6513249, 12(%rsp)
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
  movzbl 12(%rsp,%rdi), %r8d
  movl $1, %esi
  testb %r8b, %r8b
  je .LBB0_23
  cmpb %r8b, %dl
  jne .LBB0_21
  movzbl 1(%rcx,%rdi), %edx
  incq %rdi
  testb %dl, %dl
  jne .LBB0_17
  cmpb $0, 12(%rsp,%rdi)
  je .LBB0_23
.LBB0_21:
  movzbl 1(%rcx), %edx
  incq %rcx
  testb %dl, %dl
  jne .LBB0_16
  jmp .LBB0_22
.LBB0_24:
  movq %r9, 16(%rsp)
  movq %r15, 32(%rsp)
  movabsq $4294967296, %r12
  xorl %ebp, %ebp
  leaq 96(%rsp), %rbx
  xorl %eax, %eax
.LBB0_25:
  movq %rax, 24(%rsp)
  xorl %r14d, %r14d
.LBB0_26:
  leaq strings(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  leaq (%rax,%r12), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %r12
  addq %rbp, %r12
  leaq strings+200(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %r13
  leaq strings+400(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %rbp
  addq %r13, %rbp
  addq %r12, %rbp
  leaq strings+600(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %r12
  leaq strings+800(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %r13
  addq %r12, %r13
  leaq strings+1000(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %r12
  addq %r13, %r12
  addq %rbp, %r12
  leaq strings+1200(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %r13
  leaq strings+1400(%r14), %r15
  movq %r15, %rdi
  callq strlen
  shlq $32, %rax
  movabsq $4294967296, %rcx
  leaq (%rax,%rcx), %rdx
  sarq $32, %rdx
  movq %rbx, %rdi
  movq %r15, %rsi
  callq _intel_fast_memcpy@PLT
  movsbq 96(%rsp), %rbp
  addq %r13, %rbp
  addq %r12, %rbp
  movabsq $4294967296, %r12
  addq $1600, %r14
  cmpq $20000000, %r14
  jne .LBB0_26
  movq 24(%rsp), %rcx
  leal 1(%rcx), %eax
  cmpl $49, %ecx
  jne .LBB0_25
  movl 40(%rsp), %edx
  movl %edx, %eax
  sarl $31, %eax
  xorl %ecx, %ecx
  testl %edx, %edx
  setne %cl
  orl %eax, %ecx
  movslq %ecx, %rax
  movq 32(%rsp), %rsi
  addq %rax, %rsi
  movl 44(%rsp), %edx
  movl %edx, %eax
  sarl $31, %eax
  xorl %ecx, %ecx
  testl %edx, %edx
  setne %cl
  orl %eax, %ecx
  movslq %ecx, %rcx
  movl 48(%rsp), %edi
  movl %edi, %eax
  sarl $31, %eax
  xorl %edx, %edx
  testl %edi, %edi
  setne %dl
  orl %eax, %edx
  movslq %edx, %rax
  addq %rcx, %rax
  addq %rsi, %rax
  movl 52(%rsp), %esi
  movl %esi, %ecx
  sarl $31, %ecx
  xorl %edx, %edx
  testl %esi, %esi
  setne %dl
  orl %ecx, %edx
  movslq %edx, %rcx
  movl 64(%rsp), %edi
  movl %edi, %edx
  sarl $31, %edx
  xorl %esi, %esi
  testl %edi, %edi
  setne %sil
  orl %edx, %esi
  movslq %esi, %rdx
  addq %rcx, %rdx
  movl 4(%rsp), %edi
  movl %edi, %ecx
  sarl $31, %ecx
  xorl %esi, %esi
  testl %edi, %edi
  setne %sil
  orl %ecx, %esi
  movslq %esi, %rcx
  addq %rdx, %rcx
  addq %rax, %rcx
  movl 8(%rsp), %esi
  movl %esi, %eax
  sarl $31, %eax
  xorl %edx, %edx
  testl %esi, %esi
  setne %dl
  orl %eax, %edx
  movslq %edx, %rdx
  addq %rcx, %rdx
  movl $.L.str, %edi
  movq 56(%rsp), %rsi
  movq 16(%rsp), %rcx
  movq %rbp, %r8
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  addq $296, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:

