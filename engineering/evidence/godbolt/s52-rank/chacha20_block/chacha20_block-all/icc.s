main:
..B1.1: # Preds ..B1.0
  pushq %rbp #80.1
  movq %rsp, %rbp #80.1
  andq $-128, %rsp #80.1
  pushq %r12 #80.1
  pushq %r13 #80.1
  pushq %r14 #80.1
  pushq %r15 #80.1
  pushq %rbx #80.1
  subq $216, %rsp #80.1
  movl $3, %edi #80.1
  xorl %esi, %esi #80.1
  call __intel_new_feature_proc_init #80.1
..B1.17: # Preds ..B1.1
  movdqu test_in.146.0.2(%rip), %xmm4 #71.27
  movdqu 16+test_in.146.0.2(%rip), %xmm2 #71.27
  movdqa %xmm4, %xmm0 #40.5
  movdqu 48+test_in.146.0.2(%rip), %xmm12 #71.27
  movdqa %xmm2, %xmm1 #40.5
  psrldq $4, %xmm0 #40.5
  movdqa %xmm12, %xmm3 #40.5
  psrldq $4, %xmm1 #40.5
  movdqa %xmm4, %xmm6 #40.5
  psrldq $4, %xmm3 #40.5
  movdqa %xmm2, %xmm8 #40.5
  movd %xmm0, %esi #45.5
  movdqa %xmm12, %xmm9 #40.5
  movd %xmm1, %ebx #45.5
  movdqa %xmm4, %xmm11 #40.5
  movdqu 32+test_in.146.0.2(%rip), %xmm7 #71.27
  movdqa %xmm2, %xmm13 #40.5
  movdqa %xmm7, %xmm5 #40.5
  movdqa %xmm7, %xmm10 #40.5
  movd %xmm3, %r10d #45.5
  addl %ebx, %esi #45.5
  psrldq $4, %xmm5 #40.5
  movdqa %xmm12, %xmm14 #40.5
  psrldq $8, %xmm6 #40.5
  movdqa %xmm7, %xmm15 #40.5
  xorl %esi, %r10d #45.5
  movd %xmm5, %ecx #45.5
  shldl $16, %r10d, %r10d #45.5
  movd %xmm6, %r9d #46.5
  psrldq $8, %xmm8 #40.5
  psrldq $8, %xmm9 #40.5
  addl %r10d, %ecx #45.5
  xorl %ecx, %ebx #45.5
  shldl $12, %ebx, %ebx #45.5
  movd %xmm8, %r8d #46.5
  movd %xmm9, %r12d #46.5
  psrldq $8, %xmm10 #40.5
  psrldq $12, %xmm11 #40.5
  addl %ebx, %esi #45.5
  addl %r8d, %r9d #46.5
  xorl %esi, %r10d #45.5
  xorl %r9d, %r12d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $16, %r12d, %r12d #46.5
  movd %xmm4, %edx #44.5
  movd %xmm2, %eax #44.5
  psrldq $12, %xmm13 #40.5
  psrldq $12, %xmm14 #40.5
  addl %r10d, %ecx #45.5
  addl %eax, %edx #44.5
  movl %ecx, 88(%rsp) #45.5[spill]
  xorl %ecx, %ebx #45.5
  movd %xmm10, %ecx #46.5
  movd %xmm12, %edi #44.5
  movd %xmm11, %r13d #47.5
  movd %xmm14, %r14d #47.5
  movd %xmm7, %r11d #44.5
  psrldq $12, %xmm15 #40.5
  shldl $7, %ebx, %ebx #45.5
  movd %xmm15, %r15d #47.5
  addl %r12d, %ecx #46.5
  xorl %edx, %edi #44.5
  xorl %ecx, %r8d #46.5
  shldl $12, %r8d, %r8d #46.5
  shldl $16, %edi, %edi #44.5
  stmxcsr 8(%rsp) #80.1
  addl %r8d, %r9d #46.5
  addl %edi, %r11d #44.5
  xorl %r9d, %r12d #46.5
  xorl %r11d, %eax #44.5
  shldl $8, %r12d, %r12d #46.5
  shldl $12, %eax, %eax #44.5
  movl %r12d, 96(%rsp) #46.5[spill]
  addl %r12d, %ecx #46.5
  movd %xmm13, %r12d #47.5
  addl %eax, %edx #44.5
  addl %r12d, %r13d #47.5
  xorl %ecx, %r8d #46.5
  xorl %r13d, %r14d #47.5
  xorl %edx, %edi #44.5
  shldl $16, %r14d, %r14d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %edi, %edi #44.5
  addl %r14d, %r15d #47.5
  addl %r8d, %esi #51.5
  xorl %r15d, %r12d #47.5
  addl %edi, %r11d #44.5
  shldl $12, %r12d, %r12d #47.5
  addl %r12d, %r13d #47.5
  xorl %esi, %edi #51.5
  xorl %r13d, %r14d #47.5
  addl %ebx, %edx #50.5
  shldl $8, %r14d, %r14d #47.5
  shldl $16, %edi, %edi #51.5
  addl %r14d, %r15d #47.5
  xorl %edx, %r14d #50.5
  xorl %r15d, %r12d #47.5
  addl %edi, %r15d #51.5
  xorl %r15d, %r8d #51.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %eax, %eax #44.5
  addl %r14d, %ecx #50.5
  addl %r8d, %esi #51.5
  xorl %ecx, %ebx #50.5
  xorl %esi, %edi #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #51.5
  addl %r12d, %r9d #52.5
  addl %ebx, %edx #50.5
  xorl %r9d, %r10d #52.5
  addl %edi, %r15d #51.5
  addl %eax, %r13d #53.5
  xorl %edx, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %r15d, 104(%rsp) #51.5[spill]
  xorl %r15d, %r8d #51.5
  movl 96(%rsp), %r15d #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %r15d #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r15d, %r15d #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %ecx #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %r15d, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %eax #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %eax, %eax #53.5
  shldl $7, %ebx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %eax, %r13d #53.5
  addl %ebx, %esi #45.5
  xorl %r13d, %r15d #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %r15d, %r15d #53.5
  xorl %esi, %r10d #45.5
  addl %r15d, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %eax #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %ebx #45.5
  xorl %r9d, %r15d #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #45.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %eax, %eax #53.5
  shldl $7, %r12d, %r12d #52.5
  addl %ebx, %esi #45.5
  addl %r15d, %ecx #46.5
  xorl %esi, %r10d #45.5
  xorl %ecx, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %eax, %edx #44.5
  addl %r10d, %r14d #45.5
  xorl %edx, %edi #44.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %ebx #45.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %r15d #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edi, %r11d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %eax #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %eax, %eax #44.5
  movl %r15d, 96(%rsp) #46.5[spill]
  addl %r15d, %ecx #46.5
  movl 104(%rsp), %r15d #47.5[spill]
  addl %eax, %edx #44.5
  addl %r14d, %r15d #47.5
  xorl %ecx, %r8d #46.5
  xorl %r15d, %r12d #47.5
  xorl %edx, %edi #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %edi, %edi #44.5
  addl %r12d, %r13d #47.5
  addl %r8d, %esi #51.5
  xorl %r13d, %r14d #47.5
  addl %edi, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %esi, %edi #51.5
  addl %r14d, %r15d #47.5
  shldl $16, %edi, %edi #51.5
  xorl %r15d, %r12d #47.5
  addl %ebx, %edx #50.5
  addl %edi, %r15d #51.5
  xorl %edx, %r14d #50.5
  xorl %r15d, %r8d #51.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %eax, %eax #44.5
  addl %r14d, %ecx #50.5
  addl %r8d, %esi #51.5
  xorl %ecx, %ebx #50.5
  xorl %esi, %edi #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #51.5
  addl %r12d, %r9d #52.5
  addl %ebx, %edx #50.5
  xorl %r9d, %r10d #52.5
  addl %edi, %r15d #51.5
  addl %eax, %r13d #53.5
  xorl %edx, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %r15d, 104(%rsp) #51.5[spill]
  xorl %r15d, %r8d #51.5
  movl 96(%rsp), %r15d #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %r15d #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r15d, %r15d #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %ecx #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %r15d, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %eax #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %eax, %eax #53.5
  shldl $7, %ebx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %eax, %r13d #53.5
  addl %ebx, %esi #45.5
  xorl %r13d, %r15d #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %r15d, %r15d #53.5
  xorl %esi, %r10d #45.5
  addl %r15d, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %eax #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %ebx #45.5
  xorl %r9d, %r15d #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #45.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %r12d, %r12d #52.5
  shldl $7, %eax, %eax #53.5
  addl %ebx, %esi #45.5
  addl %r15d, %ecx #46.5
  xorl %esi, %r10d #45.5
  xorl %ecx, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %ebx #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %r15d #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %eax, %edx #44.5
  xorl %r13d, %r14d #47.5
  xorl %edx, %edi #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %r15d, 96(%rsp) #46.5[spill]
  addl %r15d, %ecx #46.5
  movl 104(%rsp), %r15d #47.5[spill]
  addl %edi, %r11d #44.5
  addl %r14d, %r15d #47.5
  xorl %r11d, %eax #44.5
  xorl %r15d, %r12d #47.5
  xorl %ecx, %r8d #46.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %eax, %eax #44.5
  shldl $7, %r8d, %r8d #46.5
  addl %r12d, %r13d #47.5
  addl %eax, %edx #44.5
  xorl %r13d, %r14d #47.5
  xorl %edx, %edi #44.5
  shldl $8, %r14d, %r14d #47.5
  shldl $8, %edi, %edi #44.5
  addl %ebx, %edx #50.5
  addl %r14d, %r15d #47.5
  xorl %edx, %r14d #50.5
  addl %r8d, %esi #51.5
  shldl $16, %r14d, %r14d #50.5
  addl %edi, %r11d #44.5
  xorl %esi, %edi #51.5
  shldl $16, %edi, %edi #51.5
  addl %r14d, %ecx #50.5
  xorl %r15d, %r12d #47.5
  xorl %ecx, %ebx #50.5
  addl %edi, %r15d #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $7, %r12d, %r12d #47.5
  xorl %r15d, %r8d #51.5
  addl %ebx, %edx #50.5
  shldl $12, %r8d, %r8d #51.5
  xorl %r11d, %eax #44.5
  xorl %edx, %r14d #50.5
  shldl $7, %eax, %eax #44.5
  shldl $8, %r14d, %r14d #50.5
  addl %r8d, %esi #51.5
  addl %eax, %r13d #53.5
  xorl %esi, %edi #51.5
  addl %r14d, %ecx #50.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r12d, %r9d #52.5
  shldl $8, %edi, %edi #51.5
  movl 96(%rsp), %r14d #53.5[spill]
  addl %edi, %r15d #51.5
  xorl %r13d, %r14d #53.5
  xorl %r15d, %r8d #51.5
  shldl $16, %r14d, %r14d #53.5
  shldl $7, %r8d, %r8d #51.5
  movl %r15d, 104(%rsp) #51.5[spill]
  xorl %r9d, %r10d #52.5
  movl 88(%rsp), %r15d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %r14d, %r15d #53.5
  xorl %r15d, %eax #53.5
  shldl $12, %eax, %eax #53.5
  shldl $16, %r10d, %r10d #52.5
  shldl $7, %ebx, %ebx #50.5
  addl %eax, %r13d #53.5
  addl %r10d, %r11d #52.5
  xorl %r13d, %r14d #53.5
  xorl %r11d, %r12d #52.5
  shldl $8, %r14d, %r14d #53.5
  shldl $12, %r12d, %r12d #52.5
  addl %r14d, %r15d #53.5
  addl %r12d, %r9d #52.5
  xorl %r15d, %eax #53.5
  xorl %r9d, %r10d #52.5
  shldl $7, %eax, %eax #53.5
  shldl $8, %r10d, %r10d #52.5
  addl %ebx, %esi #45.5
  addl %eax, %edx #44.5
  addl %r10d, %r11d #52.5
  xorl %esi, %r10d #45.5
  xorl %edx, %edi #44.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r10d, %r10d #45.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %r12d, %r12d #52.5
  addl %r10d, %r15d #45.5
  addl %edi, %r11d #44.5
  xorl %r15d, %ebx #45.5
  xorl %r11d, %eax #44.5
  shldl $12, %ebx, %ebx #45.5
  shldl $12, %eax, %eax #44.5
  addl %ebx, %esi #45.5
  addl %r8d, %r9d #46.5
  addl %eax, %edx #44.5
  xorl %esi, %r10d #45.5
  xorl %r9d, %r14d #46.5
  xorl %edx, %edi #44.5
  shldl $8, %r10d, %r10d #45.5
  shldl $16, %r14d, %r14d #46.5
  shldl $8, %edi, %edi #44.5
  addl %r10d, %r15d #45.5
  addl %r14d, %ecx #46.5
  addl %r12d, %r13d #47.5
  addl %edi, %r11d #44.5
  movl %r15d, 88(%rsp) #45.5[spill]
  xorl %r15d, %ebx #45.5
  movl 80(%rsp), %r15d #47.5[spill]
  xorl %ecx, %r8d #46.5
  xorl %r13d, %r15d #47.5
  xorl %r11d, %eax #44.5
  shldl $12, %r8d, %r8d #46.5
  shldl $16, %r15d, %r15d #47.5
  shldl $7, %eax, %eax #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %eax, (%rsp) #44.5[spill]
  addl %r8d, %r9d #46.5
  movl 104(%rsp), %eax #47.5[spill]
  xorl %r9d, %r14d #46.5
  addl %r15d, %eax #47.5
  addl %ebx, %edx #50.5
  xorl %eax, %r12d #47.5
  shldl $8, %r14d, %r14d #46.5
  shldl $12, %r12d, %r12d #47.5
  addl %r14d, %ecx #46.5
  addl %r12d, %r13d #47.5
  xorl %ecx, %r8d #46.5
  xorl %r13d, %r15d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %r15d, %r15d #47.5
  addl %r15d, %eax #47.5
  xorl %edx, %r15d #50.5
  addl %r8d, %esi #51.5
  xorl %eax, %r12d #47.5
  shldl $16, %r15d, %r15d #50.5
  shldl $7, %r12d, %r12d #47.5
  xorl %esi, %edi #51.5
  addl %r15d, %ecx #50.5
  shldl $16, %edi, %edi #51.5
  xorl %ecx, %ebx #50.5
  addl %edi, %eax #51.5
  shldl $12, %ebx, %ebx #50.5
  xorl %eax, %r8d #51.5
  addl %ebx, %edx #50.5
  shldl $12, %r8d, %r8d #51.5
  xorl %edx, %r15d #50.5
  addl %r8d, %esi #51.5
  shldl $8, %r15d, %r15d #50.5
  xorl %esi, %edi #51.5
  addl %r15d, %ecx #50.5
  shldl $8, %edi, %edi #51.5
  orl $32832, 8(%rsp) #80.1
  xorl %ecx, %ebx #50.5
  addl %edi, %eax #51.5
  ldmxcsr 8(%rsp) #80.1
  movq $0, 64(%rsp) #83.16[spill]
  xorl %eax, %r8d #51.5
  movl %edx, 72(%rsp) #50.5[spill]
  movl %r15d, 80(%rsp) #50.5[spill]
  shldl $7, %ebx, %ebx #50.5
..B1.16: # Preds ..B1.17
  movl %edx, %r15d #44.5
  movl (%rsp), %edx #53.5[spill]
  addl %edx, %r13d #53.5
  xorl %r13d, %r14d #53.5
  addl %r12d, %r9d #52.5
  shldl $16, %r14d, %r14d #53.5
  shldl $7, %r8d, %r8d #51.5
  xorl %r9d, %r10d #52.5
  addl %ebx, %esi #45.5
  movl %eax, 104(%rsp) #[spill]
  movl 88(%rsp), %eax #53.5[spill]
  shldl $16, %r10d, %r10d #52.5
  addl %r14d, %eax #53.5
  addl %r10d, %r11d #52.5
  xorl %eax, %edx #53.5
  xorl %r11d, %r12d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $12, %r12d, %r12d #52.5
  addl %edx, %r13d #53.5
  addl %r12d, %r9d #52.5
  xorl %r13d, %r14d #53.5
  xorl %r9d, %r10d #52.5
  shldl $8, %r14d, %r14d #53.5
  shldl $8, %r10d, %r10d #52.5
  addl %r8d, %r9d #46.5
  addl %r14d, %eax #53.5
  xorl %r9d, %r14d #46.5
  addl %r10d, %r11d #52.5
  shldl $16, %r14d, %r14d #46.5
  xorl %esi, %r10d #45.5
  addl %r14d, %ecx #46.5
  shldl $16, %r10d, %r10d #45.5
  xorl %ecx, %r8d #46.5
  xorl %eax, %edx #53.5
  addl %r10d, %eax #45.5
  xorl %r11d, %r12d #52.5
  shldl $12, %r8d, %r8d #46.5
  shldl $7, %edx, %edx #53.5
  shldl $7, %r12d, %r12d #52.5
  xorl %eax, %ebx #45.5
  addl %r8d, %r9d #46.5
  shldl $12, %ebx, %ebx #45.5
  xorl %r9d, %r14d #46.5
  addl %edx, %r15d #44.5
  addl %ebx, %esi #45.5
  shldl $8, %r14d, %r14d #46.5
  xorl %r15d, %edi #44.5
  xorl %esi, %r10d #45.5
  addl %r12d, %r13d #47.5
  addl %r14d, %ecx #46.5
  shldl $16, %edi, %edi #44.5
  shldl $8, %r10d, %r10d #45.5
  movl %r14d, 96(%rsp) #46.5[spill]
  addl %edi, %r11d #44.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %r10d, %eax #45.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %edx, %edx #44.5
  movl %eax, 88(%rsp) #45.5[spill]
  xorl %eax, %ebx #45.5
  movl 104(%rsp), %eax #47.5[spill]
  addl %edx, %r15d #44.5
  addl %r14d, %eax #47.5
  xorl %ecx, %r8d #46.5
  xorl %eax, %r12d #47.5
  xorl %r15d, %edi #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  addl %r12d, %r13d #47.5
  addl %r8d, %esi #51.5
  xorl %r13d, %r14d #47.5
  addl %edi, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %esi, %edi #51.5
  addl %r14d, %eax #47.5
  shldl $16, %edi, %edi #51.5
  xorl %eax, %r12d #47.5
  addl %ebx, %r15d #50.5
  addl %edi, %eax #51.5
  xorl %r15d, %r14d #50.5
  xorl %eax, %r8d #51.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r14d, %ecx #50.5
  addl %r8d, %esi #51.5
  xorl %ecx, %ebx #50.5
  xorl %esi, %edi #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #51.5
  addl %r12d, %r9d #52.5
  addl %ebx, %r15d #50.5
  xorl %r9d, %r10d #52.5
  addl %edi, %eax #51.5
  addl %edx, %r13d #53.5
  xorl %r15d, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %eax, 104(%rsp) #51.5[spill]
  xorl %eax, %r8d #51.5
  movl 96(%rsp), %eax #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %eax #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %eax, %eax #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %ecx #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %eax, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %ebx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %edx, %r13d #53.5
  addl %ebx, %esi #45.5
  xorl %r13d, %eax #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %eax, %eax #53.5
  xorl %esi, %r10d #45.5
  addl %eax, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %ebx #45.5
  xorl %r9d, %eax #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #45.5
  shldl $16, %eax, %eax #46.5
  shldl $7, %edx, %edx #53.5
  shldl $7, %r12d, %r12d #52.5
  addl %ebx, %esi #45.5
  addl %eax, %ecx #46.5
  xorl %esi, %r10d #45.5
  xorl %ecx, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %edx, %r15d #44.5
  addl %r10d, %r14d #45.5
  xorl %r15d, %edi #44.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %ebx #45.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %eax #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edi, %r11d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %edx #44.5
  shldl $8, %eax, %eax #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %edx, %edx #44.5
  movl %eax, 96(%rsp) #46.5[spill]
  addl %eax, %ecx #46.5
  movl 104(%rsp), %eax #47.5[spill]
  addl %edx, %r15d #44.5
  addl %r14d, %eax #47.5
  xorl %ecx, %r8d #46.5
  xorl %eax, %r12d #47.5
  xorl %r15d, %edi #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %edi, %edi #44.5
  addl %r12d, %r13d #47.5
  addl %r8d, %esi #51.5
  xorl %r13d, %r14d #47.5
  addl %edi, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %esi, %edi #51.5
  addl %r14d, %eax #47.5
  shldl $16, %edi, %edi #51.5
  xorl %eax, %r12d #47.5
  addl %ebx, %r15d #50.5
  addl %edi, %eax #51.5
  xorl %r15d, %r14d #50.5
  xorl %eax, %r8d #51.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r14d, %ecx #50.5
  addl %r8d, %esi #51.5
  xorl %ecx, %ebx #50.5
  xorl %esi, %edi #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #51.5
  addl %r12d, %r9d #52.5
  addl %ebx, %r15d #50.5
  xorl %r9d, %r10d #52.5
  addl %edi, %eax #51.5
  addl %edx, %r13d #53.5
  xorl %r15d, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %eax, 104(%rsp) #51.5[spill]
  xorl %eax, %r8d #51.5
  movl 96(%rsp), %eax #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %eax #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %eax, %eax #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %ecx #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %eax, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %ebx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %edx, %r13d #53.5
  addl %ebx, %esi #45.5
  xorl %r13d, %eax #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %eax, %eax #53.5
  xorl %esi, %r10d #45.5
  addl %eax, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %ebx #45.5
  xorl %r9d, %eax #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #45.5
  shldl $16, %eax, %eax #46.5
  shldl $7, %edx, %edx #53.5
  shldl $7, %r12d, %r12d #52.5
  addl %ebx, %esi #45.5
  addl %eax, %ecx #46.5
  xorl %esi, %r10d #45.5
  xorl %ecx, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %edx, %r15d #44.5
  addl %r10d, %r14d #45.5
  xorl %r15d, %edi #44.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %ebx #45.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %eax #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edi, %r11d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %edx #44.5
  shldl $8, %eax, %eax #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %edx, %edx #44.5
  movl %eax, 96(%rsp) #46.5[spill]
  addl %eax, %ecx #46.5
  movl 104(%rsp), %eax #47.5[spill]
  addl %edx, %r15d #44.5
  addl %r14d, %eax #47.5
  xorl %ecx, %r8d #46.5
  xorl %eax, %r12d #47.5
  xorl %r15d, %edi #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %edi, %edi #44.5
  addl %r12d, %r13d #47.5
  addl %r8d, %esi #51.5
  xorl %r13d, %r14d #47.5
  addl %edi, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %esi, %edi #51.5
  addl %r14d, %eax #47.5
  shldl $16, %edi, %edi #51.5
  xorl %eax, %r12d #47.5
  addl %ebx, %r15d #50.5
  addl %edi, %eax #51.5
  xorl %r15d, %r14d #50.5
  xorl %eax, %r8d #51.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r14d, %ecx #50.5
  addl %r8d, %esi #51.5
  xorl %ecx, %ebx #50.5
  xorl %esi, %edi #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #51.5
  addl %r12d, %r9d #52.5
  addl %ebx, %r15d #50.5
  xorl %r9d, %r10d #52.5
  addl %edi, %eax #51.5
  addl %edx, %r13d #53.5
  xorl %r15d, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %eax, 104(%rsp) #51.5[spill]
  xorl %eax, %r8d #51.5
  movl 96(%rsp), %eax #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %eax #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %eax, %eax #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %ecx #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %eax, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %ebx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %edx, %r13d #53.5
  addl %ebx, %esi #45.5
  xorl %r13d, %eax #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %eax, %eax #53.5
  xorl %esi, %r10d #45.5
  addl %eax, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %ebx #45.5
  xorl %r9d, %eax #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #45.5
  shldl $16, %eax, %eax #46.5
  shldl $7, %r12d, %r12d #52.5
  shldl $7, %edx, %edx #53.5
  addl %ebx, %esi #45.5
  addl %eax, %ecx #46.5
  xorl %esi, %r10d #45.5
  xorl %ecx, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %ebx #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %eax #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edx, %r15d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r15d, %edi #44.5
  shldl $8, %eax, %eax #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %eax, 96(%rsp) #46.5[spill]
  addl %eax, %ecx #46.5
  movl 104(%rsp), %eax #47.5[spill]
  addl %edi, %r11d #44.5
  addl %r14d, %eax #47.5
  xorl %r11d, %edx #44.5
  xorl %eax, %r12d #47.5
  xorl %ecx, %r8d #46.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %edx, %edx #44.5
  shldl $7, %r8d, %r8d #46.5
  addl %r12d, %r13d #47.5
  addl %edx, %r15d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r15d, %edi #44.5
  shldl $8, %r14d, %r14d #47.5
  shldl $8, %edi, %edi #44.5
  addl %r14d, %eax #47.5
  addl %ebx, %r15d #50.5
  xorl %eax, %r12d #47.5
  addl %r8d, %esi #51.5
  shldl $7, %r12d, %r12d #47.5
  addl %r12d, %r9d #52.5
  addl %edi, %r11d #44.5
  xorl %r15d, %r14d #50.5
  xorl %esi, %edi #51.5
  xorl %r9d, %r10d #52.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $16, %edi, %edi #51.5
  shldl $16, %r10d, %r10d #52.5
  shldl $7, %edx, %edx #44.5
  addl %r14d, %ecx #50.5
  addl %edi, %eax #51.5
  addl %r10d, %r11d #52.5
  xorl %ecx, %ebx #50.5
  xorl %eax, %r8d #51.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $12, %r12d, %r12d #52.5
  addl %ebx, %r15d #50.5
  addl %r8d, %esi #51.5
  addl %r12d, %r9d #52.5
  xorl %r15d, %r14d #50.5
  xorl %esi, %edi #51.5
  xorl %r9d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  shldl $8, %edi, %edi #51.5
  shldl $8, %r10d, %r10d #52.5
  addl %r14d, %ecx #50.5
  addl %edi, %eax #51.5
  addl %r10d, %r11d #52.5
  addl %edx, %r13d #53.5
  movl %r14d, 80(%rsp) #50.5[spill]
  xorl %ecx, %ebx #50.5
  movl 96(%rsp), %r14d #53.5[spill]
  xorl %eax, %r8d #51.5
  xorl %r11d, %r12d #52.5
  xorl %r13d, %r14d #53.5
  movl %edx, (%rsp) #44.5[spill]
  movl %r15d, 72(%rsp) #50.5[spill]
  shldl $7, %ebx, %ebx #50.5
  shldl $7, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #52.5
  shldl $16, %r14d, %r14d #53.5
..B1.15: # Preds ..B1.16
  movl 88(%rsp), %r15d #53.5[spill]
  addl %r8d, %r9d #46.5
  addl %r14d, %r15d #53.5
  addl %ebx, %esi #45.5
  xorl %esi, %r10d #45.5
  xorl %r15d, %edx #53.5
  shldl $12, %edx, %edx #53.5
  shldl $16, %r10d, %r10d #45.5
  addl %edx, %r13d #53.5
  xorl %r13d, %r14d #53.5
  addl %r12d, %r13d #47.5
  shldl $8, %r14d, %r14d #53.5
  addl %r14d, %r15d #53.5
  xorl %r9d, %r14d #46.5
  shldl $16, %r14d, %r14d #46.5
  addl %r14d, %ecx #46.5
  xorl %r15d, %edx #53.5
  xorl %ecx, %r8d #46.5
  addl %r10d, %r15d #45.5
  shldl $12, %r8d, %r8d #46.5
  shldl $7, %edx, %edx #53.5
  xorl %r15d, %ebx #45.5
  addl %r8d, %r9d #46.5
  shldl $12, %ebx, %ebx #45.5
  movl %eax, 104(%rsp) #[spill]
  xorl %r9d, %r14d #46.5
  movl 72(%rsp), %eax #44.5[spill]
  addl %ebx, %esi #45.5
  addl %edx, %eax #44.5
  xorl %esi, %r10d #45.5
  shldl $8, %r14d, %r14d #46.5
  shldl $8, %r10d, %r10d #45.5
  xorl %eax, %edi #44.5
  addl %r14d, %ecx #46.5
  shldl $16, %edi, %edi #44.5
  movl %r14d, 96(%rsp) #46.5[spill]
  addl %edi, %r11d #44.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %r10d, %r15d #45.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %edx, %edx #44.5
  movl %r15d, 88(%rsp) #45.5[spill]
  xorl %r15d, %ebx #45.5
  movl 104(%rsp), %r15d #47.5[spill]
  addl %edx, %eax #44.5
  addl %r14d, %r15d #47.5
  xorl %ecx, %r8d #46.5
  xorl %r15d, %r12d #47.5
  xorl %eax, %edi #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  addl %r12d, %r13d #47.5
  addl %r8d, %esi #51.5
  xorl %r13d, %r14d #47.5
  addl %edi, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %esi, %edi #51.5
  addl %r14d, %r15d #47.5
  shldl $16, %edi, %edi #51.5
  xorl %r15d, %r12d #47.5
  addl %ebx, %eax #50.5
  addl %edi, %r15d #51.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %r8d #51.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r14d, %ecx #50.5
  addl %r8d, %esi #51.5
  xorl %ecx, %ebx #50.5
  xorl %esi, %edi #51.5
  shldl $12, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #51.5
  addl %r12d, %r9d #52.5
  addl %ebx, %eax #50.5
  xorl %r9d, %r10d #52.5
  addl %edi, %r15d #51.5
  addl %edx, %r13d #53.5
  xorl %eax, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %r15d, 104(%rsp) #51.5[spill]
  xorl %r15d, %r8d #51.5
  movl 96(%rsp), %r15d #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %r15d #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r15d, %r15d #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %ecx #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %ecx, %ebx #50.5
  addl %r15d, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %ebx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %edx, %r13d #53.5
  addl %ebx, %esi #45.5
  xorl %r13d, %r15d #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %r15d, %r15d #53.5
  xorl %esi, %r10d #45.5
  addl %r15d, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %ebx #45.5
  xorl %r9d, %r15d #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %ebx, %ebx #45.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %r12d, %r12d #52.5
  shldl $7, %edx, %edx #53.5
  addl %ebx, %esi #45.5
  addl %r15d, %ecx #46.5
  xorl %esi, %r10d #45.5
  xorl %ecx, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %ebx #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %r15d #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edx, %eax #44.5
  xorl %r13d, %r14d #47.5
  xorl %eax, %edi #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $16, %edi, %edi #44.5
  shldl $7, %ebx, %ebx #45.5
  movl %r15d, 96(%rsp) #46.5[spill]
  addl %r15d, %ecx #46.5
  movl 104(%rsp), %r15d #47.5[spill]
  addl %edi, %r11d #44.5
  addl %r14d, %r15d #47.5
  xorl %r11d, %edx #44.5
  xorl %r15d, %r12d #47.5
  xorl %ecx, %r8d #46.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %edx, %edx #44.5
  shldl $7, %r8d, %r8d #46.5
  addl %r12d, %r13d #47.5
  addl %edx, %eax #44.5
  xorl %r13d, %r14d #47.5
  xorl %eax, %edi #44.5
  shldl $8, %r14d, %r14d #47.5
  shldl $8, %edi, %edi #44.5
  addl %r14d, %r15d #47.5
  addl %edi, %r11d #44.5
  xorl %r15d, %r12d #47.5
  xorl %r11d, %edx #44.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r12d, %r9d #52.5
  addl %ebx, %eax #50.5
  xorl %r9d, %r10d #52.5
  xorl %eax, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $16, %r14d, %r14d #50.5
  addl %r10d, %r11d #52.5
  addl %r14d, %ecx #50.5
  xorl %r11d, %r12d #52.5
  addl %r8d, %esi #51.5
  shldl $12, %r12d, %r12d #52.5
  addl %r12d, %r9d #52.5
  addl %edx, %r13d #53.5
  xorl %r9d, %r10d #52.5
  xorl %ecx, %ebx #50.5
  shldl $8, %r10d, %r10d #52.5
  shldl $12, %ebx, %ebx #50.5
  addl %r10d, %r11d #52.5
  xorl %esi, %edi #51.5
  movl %r11d, 112(%rsp) #52.5[spill]
  xorl %r11d, %r12d #52.5
  movl 96(%rsp), %r11d #53.5[spill]
  addl %ebx, %eax #50.5
  xorl %r13d, %r11d #53.5
  xorl %eax, %r14d #50.5
  shldl $16, %edi, %edi #51.5
  shldl $16, %r11d, %r11d #53.5
  shldl $8, %r14d, %r14d #50.5
  movl %r12d, 120(%rsp) #52.5[spill]
  addl %edi, %r15d #51.5
  movl 88(%rsp), %r12d #53.5[spill]
  xorl %r15d, %r8d #51.5
  addl %r11d, %r12d #53.5
  addl %r14d, %ecx #50.5
  xorl %r12d, %edx #53.5
  xorl %ecx, %ebx #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %ebx, %ebx #50.5
  addl %r8d, %esi #51.5
  addl %edx, %r13d #53.5
  xorl %esi, %edi #51.5
  xorl %r13d, %r11d #53.5
  shldl $8, %edi, %edi #51.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r15d #51.5
  addl %r11d, %r12d #53.5
  xorl %r15d, %r8d #51.5
  xorl %r12d, %edx #53.5
  movd %eax, %xmm11 #50.5
  movd %esi, %xmm14 #51.5
  movd %ebx, %xmm13 #50.5
  movl 120(%rsp), %ebx #52.5[spill]
  movd %r9d, %xmm15 #52.5
  shldl $7, %r8d, %r8d #51.5
  shldl $7, %ebx, %ebx #52.5
  shldl $7, %edx, %edx #53.5
  unpcklps %xmm14, %xmm11 #57.14
  movd %r13d, %xmm14 #53.5
  movd %ecx, %xmm6 #50.5
  movl 112(%rsp), %ecx #52.5[spill]
  movd %r14d, %xmm10 #50.5
  unpcklps %xmm14, %xmm15 #57.14
  movd %edi, %xmm8 #51.5
  movlhps %xmm15, %xmm11 #57.14
  movd %r15d, %xmm5 #51.5
  paddd %xmm4, %xmm11 #57.21
  movd %r8d, %xmm1 #51.5
  movd %r10d, %xmm9 #52.5
  movd %ecx, %xmm3 #52.5
  movd %ebx, %xmm0 #52.5
  movd %r11d, %xmm14 #53.5
  movd %r12d, %xmm15 #53.5
  movd %edx, %xmm4 #53.5
  movd %xmm11, %esi #72.7
  unpcklps %xmm13, %xmm4 #57.14
  unpcklps %xmm0, %xmm1 #57.14
  unpcklps %xmm15, %xmm3 #57.14
  unpcklps %xmm5, %xmm6 #57.14
  unpcklps %xmm9, %xmm8 #57.14
  unpcklps %xmm10, %xmm14 #57.14
  movlhps %xmm1, %xmm4 #57.14
  movlhps %xmm6, %xmm3 #57.14
  paddd %xmm2, %xmm4 #57.21
  movlhps %xmm14, %xmm8 #57.14
  paddd %xmm7, %xmm3 #57.21
  paddd %xmm12, %xmm8 #57.21
  movdqu %xmm11, (%rsp) #71.17
  movdqu %xmm4, 16(%rsp) #71.17
  movdqu %xmm3, 32(%rsp) #71.17
  movdqu %xmm8, 48(%rsp) #71.17
  cmpl $-454561520, %esi #72.22
  jne ..B1.12 # Prob 28% #72.22
..B1.2: # Preds ..B1.15
  cmpl $358169553, 4(%rsp) #72.51
  jne ..B1.12 # Prob 28% #72.51
..B1.3: # Preds ..B1.2
  cmpl $-394014517, 56(%rsp) #73.23
  jne ..B1.12 # Prob 28% #73.23
..B1.4: # Preds ..B1.3
  cmpl $1312575650, 60(%rsp) #73.53
  jne ..B1.12 # Prob 50% #73.53
..B1.5: # Preds ..B1.4
  movdqu .L_2il0floatpacket.1(%rip), %xmm5 #94.13
  xorl %ebx, %ebx #100.3
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #94.5
  movdqa %xmm5, %xmm6 #94.32
  movdqu .L_2il0floatpacket.4(%rip), %xmm2 #94.5
  movdqa %xmm1, %xmm0 #94.32
  movdqu .L_2il0floatpacket.2(%rip), %xmm4 #94.13
  psrlq $32, %xmm0 #94.32
  pmuludq %xmm1, %xmm6 #94.32
  paddd %xmm2, %xmm1 #94.5
  pmuludq %xmm1, %xmm5 #94.32
  psrlq $32, %xmm4 #94.32
  psrlq $32, %xmm1 #94.32
  pmuludq %xmm4, %xmm0 #94.32
  pmuludq %xmm1, %xmm4 #94.32
  movdqu .L_2il0floatpacket.3(%rip), %xmm3 #94.32
  psllq $32, %xmm0 #94.32
  pand %xmm3, %xmm6 #94.32
  pand %xmm3, %xmm5 #94.32
  psllq $32, %xmm4 #94.32
  por %xmm0, %xmm6 #94.32
  por %xmm4, %xmm5 #94.32
  movl $1634760805, 80(%rsp) #92.3
  movl $857760878, 84(%rsp) #92.23
  movl $2036477234, 88(%rsp) #92.43
  movl $1797285236, 92(%rsp) #92.63
  movdqu %xmm6, 96(%rsp) #94.5
  movdqu %xmm5, 112(%rsp) #94.5
  movl $-559038737, 132(%rsp) #96.3
  movl $19088743, 136(%rsp) #97.3
  movl $-1985229329, 140(%rsp) #98.3
  movd %xmm6, %r13d #105.7
  movq 64(%rsp), %r12 #100.3[spill]
..B1.6: # Preds ..B1.9 ..B1.5
  xorl %r14d, %r14d #101.5
..B1.7: # Preds ..B1.8 ..B1.6
  movl %ebx, %eax #102.20
  lea (%rsp), %rdi #103.7
  xorl %r14d, %eax #102.20
  lea 80(%rsp), %rsi #103.7
  movl %eax, 48(%rsi) #102.7
  call chacha20_core #103.7
..B1.8: # Preds ..B1.7
  movl (%rsp), %eax #104.25
  movl %eax, %esi #104.25
  incl %r14d #101.5
  shlq $32, %rsi #104.35
  movl 60(%rsp), %edx #104.41
  xorq %rdx, %rsi #104.41
  movl 28(%rsp), %ecx #104.57
  shlq $16, %rcx #104.67
  movzbl %al, %eax #105.25
  xorq %rcx, %rsi #104.67
  xorl %eax, %r13d #105.7
  addq %rsi, %r12 #104.7
  movl %r13d, 96(%rsp) #105.7
  cmpl $131072, %r14d #101.5
  jb ..B1.7 # Prob 99% #101.5
..B1.9: # Preds ..B1.8
  incl %ebx #100.3
  cmpl $16, %ebx #100.3
  jb ..B1.6 # Prob 93% #100.3
..B1.10: # Preds ..B1.9
  movq %r12, 64(%rsp) #[spill]
  movl $.L_2__STRING.0, %edi #109.3
  xorl %eax, %eax #109.3
  movq %r12, %rsi #109.3
  call printf #109.3
..B1.11: # Preds ..B1.10
  xorl %eax, %eax #110.10
  addq $216, %rsp #110.10
  popq %rbx #110.10
  popq %r15 #110.10
  popq %r14 #110.10
  popq %r13 #110.10
  popq %r12 #110.10
  movq %rbp, %rsp #110.10
  popq %rbp #110.10
  ret #110.10
..B1.12: # Preds ..B1.15 ..B1.3 ..B1.2 ..B1.4
  movl $2, %eax #89.12
  addq $216, %rsp #89.12
  popq %rbx #89.12
  popq %r15 #89.12
  popq %r14 #89.12
  popq %r13 #89.12
  popq %r12 #89.12
  movq %rbp, %rsp #89.12
  popq %rbp #89.12
  ret #89.12
test_in.146.0.2:
chacha20_core:
..B2.1: # Preds ..B2.0
  pushq %r12 #35.1
  pushq %r13 #35.1
  pushq %r14 #35.1
  pushq %r15 #35.1
  pushq %rbx #35.1
  pushq %rbp #35.1
  movdqu (%rsi), %xmm5 #40.12
  movdqu 16(%rsi), %xmm3 #40.12
  movdqu 48(%rsi), %xmm13 #40.12
  movdqu 32(%rsi), %xmm8 #40.12
  movd %xmm5, %ebp #44.5
  movdqa %xmm5, %xmm0 #40.5
  movdqa %xmm3, %xmm1 #40.5
  psrldq $4, %xmm0 #40.5
  movdqa %xmm13, %xmm2 #40.5
  psrldq $4, %xmm1 #40.5
  movdqa %xmm8, %xmm4 #40.5
  psrldq $4, %xmm2 #40.5
  movdqa %xmm5, %xmm6 #40.5
  movd %xmm0, %edx #45.5
  movdqa %xmm3, %xmm7 #40.5
  movd %xmm1, %r14d #45.5
  movdqa %xmm13, %xmm9 #40.5
  movd %xmm2, %r8d #45.5
  movdqa %xmm8, %xmm10 #40.5
  psrldq $4, %xmm4 #40.5
  movdqa %xmm5, %xmm11 #40.5
  psrldq $8, %xmm6 #40.5
  movdqa %xmm3, %xmm12 #40.5
  addl %r14d, %edx #45.5
  movdqa %xmm13, %xmm14 #40.5
  xorl %edx, %r8d #45.5
  movdqa %xmm8, %xmm15 #40.5
  movd %xmm4, %eax #45.5
  shldl $16, %r8d, %r8d #45.5
  movd %xmm3, %r13d #44.5
  psrldq $8, %xmm7 #40.5
  psrldq $8, %xmm9 #40.5
  addl %r8d, %eax #45.5
  addl %r13d, %ebp #44.5
  xorl %eax, %r14d #45.5
  shldl $12, %r14d, %r14d #45.5
  movd %xmm7, %esi #46.5
  movd %xmm9, %r15d #46.5
  psrldq $8, %xmm10 #40.5
  psrldq $12, %xmm11 #40.5
  addl %r14d, %edx #45.5
  movq %rdi, -40(%rsp) #35.1[spill]
  xorl %edx, %r8d #45.5
  movd %xmm6, %edi #46.5
  shldl $8, %r8d, %r8d #45.5
  movd %xmm13, %ecx #44.5
  psrldq $12, %xmm12 #40.5
  psrldq $12, %xmm14 #40.5
  addl %esi, %edi #46.5
  addl %r8d, %eax #45.5
  xorl %edi, %r15d #46.5
  xorl %eax, %r14d #45.5
  movl %eax, -24(%rsp) #45.5[spill]
  xorl %ebp, %ecx #44.5
  movd %xmm10, %eax #46.5
  shldl $16, %r15d, %r15d #46.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %r14d, %r14d #45.5
  movd %xmm11, %r11d #47.5
  movd %xmm12, %r10d #47.5
  movd %xmm14, %ebx #47.5
  movd %xmm8, %r9d #44.5
  psrldq $12, %xmm15 #40.5
  addl %r15d, %eax #46.5
  addl %r10d, %r11d #47.5
  xorl %eax, %esi #46.5
  xorl %r11d, %ebx #47.5
  shldl $12, %esi, %esi #46.5
  shldl $16, %ebx, %ebx #47.5
  movd %xmm15, %r12d #47.5
  addl %esi, %edi #46.5
  addl %ecx, %r9d #44.5
  xorl %edi, %r15d #46.5
  xorl %r9d, %r13d #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $12, %r13d, %r13d #44.5
  addl %ebx, %r12d #47.5
  addl %r15d, %eax #46.5
  xorl %r12d, %r10d #47.5
  addl %r13d, %ebp #44.5
  shldl $12, %r10d, %r10d #47.5
  xorl %eax, %esi #46.5
  xorl %ebp, %ecx #44.5
  shldl $7, %esi, %esi #46.5
  shldl $8, %ecx, %ecx #44.5
  addl %r10d, %r11d #47.5
  addl %esi, %edx #51.5
  xorl %r11d, %ebx #47.5
  addl %ecx, %r9d #44.5
  shldl $8, %ebx, %ebx #47.5
  xorl %edx, %ecx #51.5
  addl %r14d, %ebp #50.5
  shldl $16, %ecx, %ecx #51.5
  addl %ebx, %r12d #47.5
  xorl %ebp, %ebx #50.5
  xorl %r12d, %r10d #47.5
  addl %ecx, %r12d #51.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r10d, %r10d #47.5
  xorl %r12d, %esi #51.5
  xorl %r9d, %r13d #44.5
  shldl $12, %esi, %esi #51.5
  shldl $7, %r13d, %r13d #44.5
  addl %ebx, %eax #50.5
  addl %r10d, %edi #52.5
  xorl %eax, %r14d #50.5
  addl %esi, %edx #51.5
  shldl $12, %r14d, %r14d #50.5
  xorl %edi, %r8d #52.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  addl %r14d, %ebp #50.5
  shldl $16, %r8d, %r8d #52.5
  shldl $8, %ecx, %ecx #51.5
  xorl %r11d, %r15d #53.5
  xorl %ebp, %ebx #50.5
  shldl $16, %r15d, %r15d #53.5
  shldl $8, %ebx, %ebx #50.5
  addl %r8d, %r9d #52.5
  addl %ecx, %r12d #51.5
  xorl %r9d, %r10d #52.5
  xorl %r12d, %esi #51.5
  movl %r12d, -16(%rsp) #51.5[spill]
  addl %ebx, %eax #50.5
  movl -24(%rsp), %r12d #53.5[spill]
  xorl %eax, %r14d #50.5
  addl %r15d, %r12d #53.5
  shldl $12, %r10d, %r10d #52.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  xorl %r12d, %r13d #53.5
  addl %r10d, %edi #52.5
  shldl $12, %r13d, %r13d #53.5
  xorl %edi, %r8d #52.5
  addl %r13d, %r11d #53.5
  shldl $8, %r8d, %r8d #52.5
  xorl %r11d, %r15d #53.5
  addl %r14d, %edx #45.5
  shldl $8, %r15d, %r15d #53.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  shldl $16, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  addl %r15d, %r12d #53.5
  xorl %edi, %r15d #46.5
  xorl %r12d, %r13d #53.5
  addl %r8d, %r12d #45.5
  xorl %r9d, %r10d #52.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %r13d, %r13d #53.5
  shldl $7, %r10d, %r10d #52.5
  xorl %r12d, %r14d #45.5
  addl %r15d, %eax #46.5
  shldl $12, %r14d, %r14d #45.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  addl %r14d, %edx #45.5
  xorl %ebp, %ecx #44.5
  shldl $12, %esi, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %r8d #45.5
  addl %r10d, %r11d #47.5
  shldl $8, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  xorl %r11d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edi, %r15d #46.5
  addl %r8d, %r12d #45.5
  xorl %r9d, %r13d #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $12, %r13d, %r13d #44.5
  movl %r12d, -24(%rsp) #45.5[spill]
  xorl %r12d, %r14d #45.5
  movl -16(%rsp), %r12d #47.5[spill]
  addl %r15d, %eax #46.5
  addl %ebx, %r12d #47.5
  addl %r13d, %ebp #44.5
  xorl %r12d, %r10d #47.5
  xorl %eax, %esi #46.5
  shldl $12, %r10d, %r10d #47.5
  shldl $7, %esi, %esi #46.5
  shldl $7, %r14d, %r14d #45.5
  xorl %ebp, %ecx #44.5
  addl %r10d, %r11d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r11d, %ebx #47.5
  addl %esi, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  addl %ebx, %r12d #47.5
  xorl %ebp, %ebx #50.5
  xorl %r12d, %r10d #47.5
  addl %ecx, %r12d #51.5
  xorl %r9d, %r13d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r10d, %r10d #47.5
  shldl $7, %r13d, %r13d #44.5
  xorl %r12d, %esi #51.5
  addl %ebx, %eax #50.5
  shldl $12, %esi, %esi #51.5
  xorl %eax, %r14d #50.5
  addl %r10d, %edi #52.5
  addl %esi, %edx #51.5
  xorl %edi, %r8d #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $16, %r8d, %r8d #52.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  xorl %r11d, %r15d #53.5
  shldl $16, %r15d, %r15d #53.5
  xorl %ebp, %ebx #50.5
  addl %r8d, %r9d #52.5
  addl %ecx, %r12d #51.5
  xorl %r9d, %r10d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r10d, %r10d #52.5
  movl %r12d, -16(%rsp) #51.5[spill]
  xorl %r12d, %esi #51.5
  movl -24(%rsp), %r12d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r15d, %r12d #53.5
  xorl %eax, %r14d #50.5
  xorl %r12d, %r13d #53.5
  addl %r10d, %edi #52.5
  shldl $12, %r13d, %r13d #53.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  xorl %edi, %r8d #52.5
  addl %r13d, %r11d #53.5
  shldl $8, %r8d, %r8d #52.5
  xorl %r11d, %r15d #53.5
  addl %r14d, %edx #45.5
  shldl $8, %r15d, %r15d #53.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  shldl $16, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  addl %r15d, %r12d #53.5
  xorl %edi, %r15d #46.5
  xorl %r12d, %r13d #53.5
  addl %r8d, %r12d #45.5
  xorl %r9d, %r10d #52.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %r13d, %r13d #53.5
  shldl $7, %r10d, %r10d #52.5
  xorl %r12d, %r14d #45.5
  addl %r15d, %eax #46.5
  shldl $12, %r14d, %r14d #45.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  addl %r14d, %edx #45.5
  xorl %ebp, %ecx #44.5
  shldl $12, %esi, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %r8d #45.5
  addl %r10d, %r11d #47.5
  shldl $8, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  xorl %r11d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edi, %r15d #46.5
  addl %r8d, %r12d #45.5
  xorl %r9d, %r13d #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $12, %r13d, %r13d #44.5
  movl %r12d, -24(%rsp) #45.5[spill]
  xorl %r12d, %r14d #45.5
  movl -16(%rsp), %r12d #47.5[spill]
  addl %r15d, %eax #46.5
  addl %ebx, %r12d #47.5
  addl %r13d, %ebp #44.5
  xorl %r12d, %r10d #47.5
  xorl %eax, %esi #46.5
  shldl $12, %r10d, %r10d #47.5
  shldl $7, %esi, %esi #46.5
  shldl $7, %r14d, %r14d #45.5
  xorl %ebp, %ecx #44.5
  addl %r10d, %r11d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r11d, %ebx #47.5
  addl %esi, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %ebx, %r12d #47.5
  xorl %r9d, %r13d #44.5
  xorl %r12d, %r10d #47.5
  addl %ecx, %r12d #51.5
  xorl %r12d, %esi #51.5
  addl %r14d, %ebp #50.5
  shldl $12, %esi, %esi #51.5
  shldl $7, %r13d, %r13d #44.5
  shldl $7, %r10d, %r10d #47.5
  addl %esi, %edx #51.5
  xorl %ebp, %ebx #50.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  shldl $16, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #51.5
  xorl %r11d, %r15d #53.5
  addl %r10d, %edi #52.5
  shldl $16, %r15d, %r15d #53.5
  addl %ebx, %eax #50.5
  addl %ecx, %r12d #51.5
  xorl %edi, %r8d #52.5
  xorl %eax, %r14d #50.5
  movl %r12d, -16(%rsp) #51.5[spill]
  xorl %r12d, %esi #51.5
  movl -24(%rsp), %r12d #53.5[spill]
  shldl $16, %r8d, %r8d #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  addl %r15d, %r12d #53.5
  addl %r8d, %r9d #52.5
  xorl %r12d, %r13d #53.5
  addl %r14d, %ebp #50.5
  shldl $12, %r13d, %r13d #53.5
  xorl %r9d, %r10d #52.5
  xorl %ebp, %ebx #50.5
  shldl $12, %r10d, %r10d #52.5
  shldl $8, %ebx, %ebx #50.5
  addl %r13d, %r11d #53.5
  addl %r10d, %edi #52.5
  xorl %r11d, %r15d #53.5
  addl %ebx, %eax #50.5
  shldl $8, %r15d, %r15d #53.5
  xorl %edi, %r8d #52.5
  addl %esi, %edi #46.5
  xorl %eax, %r14d #50.5
  addl %r15d, %r12d #53.5
  xorl %edi, %r15d #46.5
  xorl %r12d, %r13d #53.5
  shldl $7, %r14d, %r14d #50.5
  shldl $16, %r15d, %r15d #46.5
  shldl $8, %r8d, %r8d #52.5
  shldl $7, %r13d, %r13d #53.5
  addl %r14d, %edx #45.5
  addl %r15d, %eax #46.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  xorl %eax, %esi #46.5
  xorl %r9d, %r10d #52.5
  shldl $16, %r8d, %r8d #45.5
  shldl $12, %esi, %esi #46.5
  shldl $7, %r10d, %r10d #52.5
  addl %r8d, %r12d #45.5
  addl %esi, %edi #46.5
  xorl %r12d, %r14d #45.5
  addl %r10d, %r11d #47.5
  xorl %edi, %r15d #46.5
  xorl %r11d, %ebx #47.5
  shldl $12, %r14d, %r14d #45.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %ebx, %ebx #47.5
  addl %r13d, %ebp #44.5
  addl %r14d, %edx #45.5
  xorl %ebp, %ecx #44.5
  addl %r15d, %eax #46.5
  movl %r15d, -8(%rsp) #46.5[spill]
  xorl %edx, %r8d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  xorl %eax, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  shldl $8, %r8d, %r8d #45.5
  shldl $7, %esi, %esi #46.5
  addl %ebx, %r15d #47.5
  addl %ecx, %r9d #44.5
  xorl %r15d, %r10d #47.5
  xorl %r9d, %r13d #44.5
  shldl $12, %r10d, %r10d #47.5
  shldl $12, %r13d, %r13d #44.5
  addl %r8d, %r12d #45.5
  addl %r10d, %r11d #47.5
  xorl %r12d, %r14d #45.5
  xorl %r11d, %ebx #47.5
  shldl $7, %r14d, %r14d #45.5
  shldl $8, %ebx, %ebx #47.5
  addl %r13d, %ebp #44.5
  addl %ebx, %r15d #47.5
  xorl %ebp, %ecx #44.5
  addl %r14d, %ebp #50.5
  xorl %ebp, %ebx #50.5
  addl %esi, %edx #51.5
  shldl $16, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #44.5
  addl %ebx, %eax #50.5
  addl %ecx, %r9d #44.5
  xorl %eax, %r14d #50.5
  xorl %edx, %ecx #51.5
  shldl $12, %r14d, %r14d #50.5
  shldl $16, %ecx, %ecx #51.5
  xorl %r15d, %r10d #47.5
  addl %r14d, %ebp #50.5
  addl %ecx, %r15d #51.5
  xorl %ebp, %ebx #50.5
  xorl %r15d, %esi #51.5
  xorl %r9d, %r13d #44.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %esi, %esi #51.5
  shldl $7, %r13d, %r13d #44.5
  shldl $7, %r10d, %r10d #47.5
  addl %ebx, %eax #50.5
  addl %esi, %edx #51.5
  xorl %eax, %r14d #50.5
  xorl %edx, %ecx #51.5
  movl %r12d, -24(%rsp) #45.5[spill]
  movl %ebx, -32(%rsp) #50.5[spill]
  shldl $7, %r14d, %r14d #50.5
  shldl $8, %ecx, %ecx #51.5
..B2.5: # Preds ..B2.1
  movl %r12d, %ebx #53.5
  addl %r10d, %edi #52.5
  addl %r13d, %r11d #53.5
  xorl %edi, %r8d #52.5
  addl %r14d, %edx #45.5
  shldl $16, %r8d, %r8d #52.5
  movl -8(%rsp), %r12d #53.5[spill]
  addl %r8d, %r9d #52.5
  xorl %r11d, %r12d #53.5
  xorl %r9d, %r10d #52.5
  shldl $16, %r12d, %r12d #53.5
  shldl $12, %r10d, %r10d #52.5
  addl %r10d, %edi #52.5
  addl %r12d, %ebx #53.5
  xorl %edi, %r8d #52.5
  xorl %ebx, %r13d #53.5
  addl %ecx, %r15d #51.5
  shldl $12, %r13d, %r13d #53.5
  shldl $8, %r8d, %r8d #52.5
  addl %r13d, %r11d #53.5
  addl %r8d, %r9d #52.5
  xorl %r11d, %r12d #53.5
  xorl %edx, %r8d #45.5
  shldl $8, %r12d, %r12d #53.5
  shldl $16, %r8d, %r8d #45.5
  xorl %r15d, %esi #51.5
  addl %r12d, %ebx #53.5
  shldl $7, %esi, %esi #51.5
  xorl %ebx, %r13d #53.5
  addl %r8d, %ebx #45.5
  xorl %ebx, %r14d #45.5
  addl %esi, %edi #46.5
  shldl $12, %r14d, %r14d #45.5
  shldl $7, %r13d, %r13d #53.5
  xorl %edi, %r12d #46.5
  addl %r14d, %edx #45.5
  shldl $16, %r12d, %r12d #46.5
  xorl %edx, %r8d #45.5
  xorl %r9d, %r10d #52.5
  addl %r12d, %eax #46.5
  xorl %eax, %esi #46.5
  shldl $8, %r8d, %r8d #45.5
  shldl $7, %r10d, %r10d #52.5
  shldl $12, %esi, %esi #46.5
  addl %r13d, %ebp #44.5
  addl %r8d, %ebx #45.5
  xorl %ebp, %ecx #44.5
  addl %r10d, %r11d #47.5
  shldl $16, %ecx, %ecx #44.5
  movl %ebx, -24(%rsp) #45.5[spill]
  xorl %ebx, %r14d #45.5
  movl -32(%rsp), %ebx #47.5[spill]
  addl %esi, %edi #46.5
  xorl %r11d, %ebx #47.5
  addl %ecx, %r9d #44.5
  shldl $16, %ebx, %ebx #47.5
  shldl $7, %r14d, %r14d #45.5
  xorl %edi, %r12d #46.5
  xorl %r9d, %r13d #44.5
  shldl $8, %r12d, %r12d #46.5
  shldl $12, %r13d, %r13d #44.5
  addl %ebx, %r15d #47.5
  addl %r12d, %eax #46.5
  xorl %r15d, %r10d #47.5
  addl %r13d, %ebp #44.5
  shldl $12, %r10d, %r10d #47.5
  xorl %eax, %esi #46.5
  xorl %ebp, %ecx #44.5
  shldl $7, %esi, %esi #46.5
  shldl $8, %ecx, %ecx #44.5
  addl %r10d, %r11d #47.5
  addl %esi, %edx #51.5
  xorl %r11d, %ebx #47.5
  addl %ecx, %r9d #44.5
  shldl $8, %ebx, %ebx #47.5
  xorl %edx, %ecx #51.5
  addl %r14d, %ebp #50.5
  shldl $16, %ecx, %ecx #51.5
  addl %ebx, %r15d #47.5
  xorl %ebp, %ebx #50.5
  xorl %r15d, %r10d #47.5
  addl %ecx, %r15d #51.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r10d, %r10d #47.5
  xorl %r15d, %esi #51.5
  xorl %r9d, %r13d #44.5
  shldl $12, %esi, %esi #51.5
  shldl $7, %r13d, %r13d #44.5
  addl %ebx, %eax #50.5
  addl %r10d, %edi #52.5
  xorl %eax, %r14d #50.5
  addl %esi, %edx #51.5
  shldl $12, %r14d, %r14d #50.5
  xorl %edi, %r8d #52.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  addl %r14d, %ebp #50.5
  shldl $16, %r8d, %r8d #52.5
  shldl $8, %ecx, %ecx #51.5
  xorl %r11d, %r12d #53.5
  xorl %ebp, %ebx #50.5
  shldl $16, %r12d, %r12d #53.5
  shldl $8, %ebx, %ebx #50.5
  addl %r8d, %r9d #52.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r10d #52.5
  xorl %r15d, %esi #51.5
  movl %r15d, -16(%rsp) #51.5[spill]
  addl %ebx, %eax #50.5
  movl -24(%rsp), %r15d #53.5[spill]
  xorl %eax, %r14d #50.5
  addl %r12d, %r15d #53.5
  shldl $12, %r10d, %r10d #52.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  xorl %r15d, %r13d #53.5
  addl %r10d, %edi #52.5
  shldl $12, %r13d, %r13d #53.5
  xorl %edi, %r8d #52.5
  addl %r13d, %r11d #53.5
  shldl $8, %r8d, %r8d #52.5
  xorl %r11d, %r12d #53.5
  addl %r14d, %edx #45.5
  shldl $8, %r12d, %r12d #53.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  shldl $16, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  addl %r12d, %r15d #53.5
  xorl %edi, %r12d #46.5
  xorl %r15d, %r13d #53.5
  addl %r8d, %r15d #45.5
  xorl %r9d, %r10d #52.5
  shldl $16, %r12d, %r12d #46.5
  shldl $7, %r13d, %r13d #53.5
  shldl $7, %r10d, %r10d #52.5
  xorl %r15d, %r14d #45.5
  addl %r12d, %eax #46.5
  shldl $12, %r14d, %r14d #45.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  addl %r14d, %edx #45.5
  xorl %ebp, %ecx #44.5
  shldl $12, %esi, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %r8d #45.5
  addl %r10d, %r11d #47.5
  shldl $8, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  xorl %r11d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edi, %r12d #46.5
  addl %r8d, %r15d #45.5
  xorl %r9d, %r13d #44.5
  shldl $8, %r12d, %r12d #46.5
  shldl $12, %r13d, %r13d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r14d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r12d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r13d, %ebp #44.5
  xorl %r15d, %r10d #47.5
  xorl %eax, %esi #46.5
  shldl $12, %r10d, %r10d #47.5
  shldl $7, %esi, %esi #46.5
  shldl $7, %r14d, %r14d #45.5
  xorl %ebp, %ecx #44.5
  addl %r10d, %r11d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r11d, %ebx #47.5
  addl %esi, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  addl %ebx, %r15d #47.5
  xorl %ebp, %ebx #50.5
  xorl %r15d, %r10d #47.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r13d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r10d, %r10d #47.5
  shldl $7, %r13d, %r13d #44.5
  xorl %r15d, %esi #51.5
  addl %ebx, %eax #50.5
  shldl $12, %esi, %esi #51.5
  xorl %eax, %r14d #50.5
  addl %r10d, %edi #52.5
  addl %esi, %edx #51.5
  xorl %edi, %r8d #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $16, %r8d, %r8d #52.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  xorl %r11d, %r12d #53.5
  shldl $16, %r12d, %r12d #53.5
  xorl %ebp, %ebx #50.5
  addl %r8d, %r9d #52.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r10d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r10d, %r10d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %esi #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r12d, %r15d #53.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %r13d #53.5
  addl %r10d, %edi #52.5
  shldl $12, %r13d, %r13d #53.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  xorl %edi, %r8d #52.5
  addl %r13d, %r11d #53.5
  shldl $8, %r8d, %r8d #52.5
  xorl %r11d, %r12d #53.5
  addl %r14d, %edx #45.5
  shldl $8, %r12d, %r12d #53.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  shldl $16, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  addl %r12d, %r15d #53.5
  xorl %edi, %r12d #46.5
  xorl %r15d, %r13d #53.5
  addl %r8d, %r15d #45.5
  xorl %r9d, %r10d #52.5
  shldl $16, %r12d, %r12d #46.5
  shldl $7, %r13d, %r13d #53.5
  shldl $7, %r10d, %r10d #52.5
  xorl %r15d, %r14d #45.5
  addl %r12d, %eax #46.5
  shldl $12, %r14d, %r14d #45.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  addl %r14d, %edx #45.5
  xorl %ebp, %ecx #44.5
  shldl $12, %esi, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %r8d #45.5
  addl %r10d, %r11d #47.5
  shldl $8, %r8d, %r8d #45.5
  addl %esi, %edi #46.5
  xorl %r11d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edi, %r12d #46.5
  addl %r8d, %r15d #45.5
  xorl %r9d, %r13d #44.5
  shldl $8, %r12d, %r12d #46.5
  shldl $12, %r13d, %r13d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r14d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r12d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r13d, %ebp #44.5
  xorl %r15d, %r10d #47.5
  xorl %eax, %esi #46.5
  shldl $12, %r10d, %r10d #47.5
  shldl $7, %esi, %esi #46.5
  shldl $7, %r14d, %r14d #45.5
  xorl %ebp, %ecx #44.5
  addl %r10d, %r11d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r11d, %ebx #47.5
  addl %esi, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  addl %ebx, %r15d #47.5
  xorl %ebp, %ebx #50.5
  xorl %r15d, %r10d #47.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r13d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r10d, %r10d #47.5
  shldl $7, %r13d, %r13d #44.5
  xorl %r15d, %esi #51.5
  addl %ebx, %eax #50.5
  shldl $12, %esi, %esi #51.5
  xorl %eax, %r14d #50.5
  addl %r10d, %edi #52.5
  addl %esi, %edx #51.5
  xorl %edi, %r8d #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $16, %r8d, %r8d #52.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  xorl %r11d, %r12d #53.5
  shldl $16, %r12d, %r12d #53.5
  xorl %ebp, %ebx #50.5
  addl %r8d, %r9d #52.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r10d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r10d, %r10d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %esi #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r12d, %r15d #53.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %r13d #53.5
  addl %r10d, %edi #52.5
  shldl $12, %r13d, %r13d #53.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  xorl %edi, %r8d #52.5
  addl %r13d, %r11d #53.5
  shldl $8, %r8d, %r8d #52.5
  xorl %r11d, %r12d #53.5
  addl %r14d, %edx #45.5
  shldl $8, %r12d, %r12d #53.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  shldl $16, %r8d, %r8d #45.5
  addl %r12d, %r15d #53.5
  xorl %r9d, %r10d #52.5
  xorl %r15d, %r13d #53.5
  addl %r8d, %r15d #45.5
  xorl %r15d, %r14d #45.5
  addl %esi, %edi #46.5
  shldl $12, %r14d, %r14d #45.5
  shldl $7, %r10d, %r10d #52.5
  shldl $7, %r13d, %r13d #53.5
  addl %r14d, %edx #45.5
  addl %r10d, %r11d #47.5
  xorl %edx, %r8d #45.5
  xorl %edi, %r12d #46.5
  shldl $8, %r8d, %r8d #45.5
  shldl $16, %r12d, %r12d #46.5
  xorl %r11d, %ebx #47.5
  addl %r8d, %r15d #45.5
  shldl $16, %ebx, %ebx #47.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r14d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r12d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  xorl %r15d, %r10d #47.5
  shldl $12, %esi, %esi #46.5
  shldl $12, %r10d, %r10d #47.5
  shldl $7, %r14d, %r14d #45.5
  xorl %ebp, %ecx #44.5
  addl %esi, %edi #46.5
  shldl $16, %ecx, %ecx #44.5
  addl %r10d, %r11d #47.5
  addl %ecx, %r9d #44.5
  xorl %edi, %r12d #46.5
  xorl %r11d, %ebx #47.5
  shldl $8, %r12d, %r12d #46.5
  shldl $8, %ebx, %ebx #47.5
  xorl %r9d, %r13d #44.5
  addl %r12d, %eax #46.5
  shldl $12, %r13d, %r13d #44.5
  addl %ebx, %r15d #47.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  xorl %r15d, %r10d #47.5
  shldl $7, %esi, %esi #46.5
  shldl $7, %r10d, %r10d #47.5
  xorl %ebp, %ecx #44.5
  addl %r14d, %ebp #50.5
  shldl $8, %ecx, %ecx #44.5
  addl %esi, %edx #51.5
  addl %r10d, %edi #52.5
  addl %ecx, %r9d #44.5
  xorl %ebp, %ebx #50.5
  xorl %edx, %ecx #51.5
  xorl %edi, %r8d #52.5
  shldl $16, %ebx, %ebx #50.5
  shldl $16, %ecx, %ecx #51.5
  shldl $16, %r8d, %r8d #52.5
  xorl %r9d, %r13d #44.5
  addl %ebx, %eax #50.5
  addl %ecx, %r15d #51.5
  addl %r8d, %r9d #52.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %esi #51.5
  xorl %r9d, %r10d #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $12, %esi, %esi #51.5
  shldl $12, %r10d, %r10d #52.5
  shldl $7, %r13d, %r13d #44.5
  addl %r14d, %ebp #50.5
  addl %esi, %edx #51.5
  addl %r10d, %edi #52.5
  xorl %ebp, %ebx #50.5
  xorl %edx, %ecx #51.5
  xorl %edi, %r8d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #51.5
  shldl $8, %r8d, %r8d #52.5
  addl %ebx, %eax #50.5
  addl %ecx, %r15d #51.5
  addl %r8d, %r9d #52.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %esi #51.5
  xorl %r9d, %r10d #52.5
  movl %r12d, -8(%rsp) #46.5[spill]
  addl %r13d, %r11d #53.5
  movl %ebx, -32(%rsp) #50.5[spill]
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  shldl $7, %r10d, %r10d #52.5
..B2.4: # Preds ..B2.5
  addl %r14d, %edx #45.5
  xorl %r11d, %r12d #53.5
  xorl %edx, %r8d #45.5
  shldl $16, %r12d, %r12d #53.5
  shldl $16, %r8d, %r8d #45.5
  movl -24(%rsp), %ebx #53.5[spill]
  addl %esi, %edi #46.5
  addl %r12d, %ebx #53.5
  xorl %ebx, %r13d #53.5
  shldl $12, %r13d, %r13d #53.5
  addl %r13d, %r11d #53.5
  xorl %r11d, %r12d #53.5
  addl %r10d, %r11d #47.5
  shldl $8, %r12d, %r12d #53.5
  addl %r12d, %ebx #53.5
  xorl %edi, %r12d #46.5
  xorl %ebx, %r13d #53.5
  addl %r8d, %ebx #45.5
  xorl %ebx, %r14d #45.5
  shldl $12, %r14d, %r14d #45.5
  shldl $16, %r12d, %r12d #46.5
  shldl $7, %r13d, %r13d #53.5
  addl %r14d, %edx #45.5
  addl %r12d, %eax #46.5
  xorl %edx, %r8d #45.5
  xorl %eax, %esi #46.5
  shldl $8, %r8d, %r8d #45.5
  shldl $12, %esi, %esi #46.5
  addl %r13d, %ebp #44.5
  addl %r8d, %ebx #45.5
  xorl %ebp, %ecx #44.5
  xorl %ebx, %r14d #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %r14d, %r14d #45.5
  movl %ebx, -24(%rsp) #45.5[spill]
  addl %esi, %edi #46.5
  movl -32(%rsp), %ebx #47.5[spill]
  addl %ecx, %r9d #44.5
  xorl %r11d, %ebx #47.5
  xorl %edi, %r12d #46.5
  shldl $16, %ebx, %ebx #47.5
  shldl $8, %r12d, %r12d #46.5
  xorl %r9d, %r13d #44.5
  addl %ebx, %r15d #47.5
  shldl $12, %r13d, %r13d #44.5
  addl %r12d, %eax #46.5
  xorl %r15d, %r10d #47.5
  shldl $12, %r10d, %r10d #47.5
  addl %r13d, %ebp #44.5
  xorl %eax, %esi #46.5
  shldl $7, %esi, %esi #46.5
  xorl %ebp, %ecx #44.5
  addl %r10d, %r11d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r11d, %ebx #47.5
  addl %esi, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r9d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  addl %ebx, %r15d #47.5
  xorl %ebp, %ebx #50.5
  xorl %r15d, %r10d #47.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r13d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r10d, %r10d #47.5
  shldl $7, %r13d, %r13d #44.5
  xorl %r15d, %esi #51.5
  addl %ebx, %eax #50.5
  shldl $12, %esi, %esi #51.5
  xorl %eax, %r14d #50.5
  addl %r10d, %edi #52.5
  addl %esi, %edx #51.5
  xorl %edi, %r8d #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $16, %r8d, %r8d #52.5
  xorl %edx, %ecx #51.5
  addl %r13d, %r11d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r14d, %ebp #50.5
  xorl %r11d, %r12d #53.5
  shldl $16, %r12d, %r12d #53.5
  xorl %ebp, %ebx #50.5
  addl %r8d, %r9d #52.5
  addl %ecx, %r15d #51.5
  xorl %r9d, %r10d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r10d, %r10d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %esi #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r12d, %r15d #53.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %r13d #53.5
  addl %r10d, %edi #52.5
  shldl $12, %r13d, %r13d #53.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  xorl %edi, %r8d #52.5
  addl %r13d, %r11d #53.5
  shldl $8, %r8d, %r8d #52.5
  xorl %r11d, %r12d #53.5
  addl %r14d, %edx #45.5
  shldl $8, %r12d, %r12d #53.5
  addl %r8d, %r9d #52.5
  xorl %edx, %r8d #45.5
  shldl $16, %r8d, %r8d #45.5
  addl %r12d, %r15d #53.5
  xorl %r9d, %r10d #52.5
  xorl %r15d, %r13d #53.5
  addl %r8d, %r15d #45.5
  xorl %r15d, %r14d #45.5
  addl %esi, %edi #46.5
  shldl $12, %r14d, %r14d #45.5
  shldl $7, %r10d, %r10d #52.5
  shldl $7, %r13d, %r13d #53.5
  addl %r14d, %edx #45.5
  addl %r10d, %r11d #47.5
  xorl %edx, %r8d #45.5
  xorl %r11d, %ebx #47.5
  shldl $8, %r8d, %r8d #45.5
  shldl $16, %ebx, %ebx #47.5
  addl %r8d, %r15d #45.5
  addl %r13d, %ebp #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r14d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  xorl %ebp, %ecx #44.5
  addl %ebx, %r15d #47.5
  xorl %edi, %r12d #46.5
  xorl %r15d, %r10d #47.5
  shldl $12, %r10d, %r10d #47.5
  shldl $16, %ecx, %ecx #44.5
  shldl $16, %r12d, %r12d #46.5
  shldl $7, %r14d, %r14d #45.5
  addl %r10d, %r11d #47.5
  addl %ecx, %r9d #44.5
  xorl %r11d, %ebx #47.5
  addl %r12d, %eax #46.5
  shldl $8, %ebx, %ebx #47.5
  xorl %r9d, %r13d #44.5
  xorl %eax, %esi #46.5
  addl %ebx, %r15d #47.5
  shldl $12, %r13d, %r13d #44.5
  shldl $12, %esi, %esi #46.5
  xorl %r15d, %r10d #47.5
  addl %r13d, %ebp #44.5
  shldl $7, %r10d, %r10d #47.5
  addl %esi, %edi #46.5
  xorl %ebp, %ecx #44.5
  xorl %edi, %r12d #46.5
  addl %r10d, %edi #52.5
  shldl $8, %ecx, %ecx #44.5
  shldl $8, %r12d, %r12d #46.5
  xorl %edi, %r8d #52.5
  addl %ecx, %r9d #44.5
  shldl $16, %r8d, %r8d #52.5
  xorl %r9d, %r13d #44.5
  addl %r8d, %r9d #52.5
  xorl %r9d, %r10d #52.5
  addl %r12d, %eax #46.5
  shldl $12, %r10d, %r10d #52.5
  shldl $7, %r13d, %r13d #44.5
  xorl %eax, %esi #46.5
  addl %r14d, %ebp #50.5
  shldl $7, %esi, %esi #46.5
  addl %r10d, %edi #52.5
  xorl %ebp, %ebx #50.5
  xorl %edi, %r8d #52.5
  addl %esi, %edx #51.5
  addl %r13d, %r11d #53.5
  xorl %edx, %ecx #51.5
  shldl $16, %ebx, %ebx #50.5
  shldl $8, %r8d, %r8d #52.5
  shldl $16, %ecx, %ecx #51.5
  xorl %r11d, %r12d #53.5
  addl %ebx, %eax #50.5
  shldl $16, %r12d, %r12d #53.5
  addl %r8d, %r9d #52.5
  xorl %eax, %r14d #50.5
  xorl %r9d, %r10d #52.5
  addl %ecx, %r15d #51.5
  movl %r10d, -8(%rsp) #52.5[spill]
  xorl %r15d, %esi #51.5
  movl -24(%rsp), %r10d #53.5[spill]
  movd %edi, %xmm15 #52.5
  addl %r12d, %r10d #53.5
  movd %r8d, %xmm10 #52.5
  shldl $12, %r14d, %r14d #50.5
  shldl $12, %esi, %esi #51.5
  xorl %r10d, %r13d #53.5
  addl %r14d, %ebp #50.5
  shldl $12, %r13d, %r13d #53.5
  xorl %ebp, %ebx #50.5
  addl %esi, %edx #51.5
  addl %r13d, %r11d #53.5
  xorl %edx, %ecx #51.5
  shldl $8, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #51.5
  xorl %r11d, %r12d #53.5
  addl %ebx, %eax #50.5
  shldl $8, %r12d, %r12d #53.5
  addl %ecx, %r15d #51.5
  addl %r12d, %r10d #53.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %esi #51.5
  xorl %r10d, %r13d #53.5
  movd %eax, %xmm7 #50.5
  movd %ebp, %xmm12 #50.5
  movl -8(%rsp), %eax #52.5[spill]
  movd %edx, %xmm14 #51.5
  shldl $7, %r14d, %r14d #50.5
  shldl $7, %esi, %esi #51.5
  shldl $7, %eax, %eax #52.5
  shldl $7, %r13d, %r13d #53.5
  unpcklps %xmm14, %xmm12 #57.14
  movd %r11d, %xmm14 #53.5
  unpcklps %xmm14, %xmm15 #57.14
  movd %ebx, %xmm11 #50.5
  movlhps %xmm15, %xmm12 #57.14
  movd %r14d, %xmm0 #50.5
  movd %ecx, %xmm9 #51.5
  movd %r15d, %xmm6 #51.5
  movd %esi, %xmm2 #51.5
  movd %r9d, %xmm4 #52.5
  movd %eax, %xmm1 #52.5
  movd %r12d, %xmm14 #53.5
  movd %r10d, %xmm15 #53.5
  paddd %xmm5, %xmm12 #57.21
  movd %r13d, %xmm5 #53.5
  unpcklps %xmm0, %xmm5 #57.14
  unpcklps %xmm1, %xmm2 #57.14
  unpcklps %xmm15, %xmm4 #57.14
  unpcklps %xmm6, %xmm7 #57.14
  unpcklps %xmm10, %xmm9 #57.14
  unpcklps %xmm11, %xmm14 #57.14
  movq -40(%rsp), %rdx #57.5[spill]
  movlhps %xmm2, %xmm5 #57.14
  movlhps %xmm7, %xmm4 #57.14
  paddd %xmm3, %xmm5 #57.21
  movlhps %xmm14, %xmm9 #57.14
  paddd %xmm8, %xmm4 #57.21
  paddd %xmm13, %xmm9 #57.21
  movdqu %xmm12, (%rdx) #57.5
  movdqu %xmm5, 16(%rdx) #57.5
  movdqu %xmm4, 32(%rdx) #57.5
  movdqu %xmm9, 48(%rdx) #57.5
  popq %rbp #58.1
  popq %rbx #58.1
  popq %r15 #58.1
  popq %r14 #58.1
  popq %r13 #58.1
  popq %r12 #58.1
  ret #58.1
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2__STRING.0:
