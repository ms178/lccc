main:
..B1.1: # Preds ..B1.0
  pushq %rbp #48.16
  movq %rsp, %rbp #48.16
  andq $-128, %rsp #48.16
  pushq %r12 #48.16
  pushq %r13 #48.16
  pushq %r14 #48.16
  pushq %r15 #48.16
  pushq %rbx #48.16
  subq $88, %rsp #48.16
  movl $3, %edi #48.16
  xorl %esi, %esi #48.16
  call __intel_new_feature_proc_init #48.16
..B1.47: # Preds ..B1.1
  stmxcsr (%rsp) #48.16
  xorl %ecx, %ecx #7.5
  movl $11, %edx #12.13
  movq %rdx, %r13 #13.15
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #10.33
  movdqu .L_2il0floatpacket.1(%rip), %xmm0 #10.33
  movl %ecx, %r12d #13.15
  orl $32832, (%rsp) #48.16
  movl %ecx, %r14d #13.15
  movdqu %xmm1, perm1(%rip) #10.33
  paddd %xmm0, %xmm1 #10.33
  ldmxcsr (%rsp) #48.16
  movdqu %xmm1, 16+perm1(%rip) #10.33
  movl %ecx, %ebx #13.15
  movl $8, 32+perm1(%rip) #10.33
  movl $9, 36+perm1(%rip) #10.33
  movl $10, 40+perm1(%rip) #10.33
..B1.2: # Preds ..B1.30 ..B1.47
  movl %r13d, %r10d #16.9
  cmpl $1, %r10d #16.20
  jle ..B1.9 # Prob 50% #16.20
..B1.3: # Preds ..B1.2
  xorl %edx, %edx #16.9
  lea -1(%r10), %r9d #16.9
  movl 32+perm1(%rip), %ecx #18.47
  movl $1, %eax #16.9
  movl 36+perm1(%rip), %esi #18.47
  xorl %edi, %edi #16.25
  movl 40+perm1(%rip), %r8d #18.47
  shrl $1, %r9d #16.9
  je ..B1.7 # Prob 10% #16.9
..B1.4: # Preds ..B1.3
  movslq %r13d, %rax #16.25
  lea (,%rax,4), %r11 #16.25
..B1.5: # Preds ..B1.5 ..B1.4
  incl %edx #16.9
  lea (%r10,%rdi), %r13d #43.13
  movl %r13d, -4+count(%r11) #16.25
  lea -1(%rdi,%r10), %r15d #16.25
  movl %r15d, -8+count(%r11) #16.25
  addq $-8, %r11 #16.9
  addl $-2, %edi #16.9
  cmpl %r9d, %edx #16.9
  jb ..B1.5 # Prob 87% #16.9
..B1.6: # Preds ..B1.5
  movslq %edx, %r13 #16.43
  addq %r13, %r13 #43.13
  negq %r13 #43.13
  addq %rax, %r13 #43.13
  lea 1(%rdx,%rdx), %eax #16.25
..B1.7: # Preds ..B1.3 ..B1.6
  lea -1(%rax), %edi #16.9
  lea -1(%r10), %r9d #16.9
  cmpl %r9d, %edi #16.9
  jae ..B1.10 # Prob 10% #16.9
..B1.8: # Preds ..B1.7
  movl %eax, %edi #16.9
  movslq %r10d, %r13 #16.25
  negl %edi #16.9
  movslq %eax, %rax #16.25
  subq %rax, %r13 #16.25
  lea 1(%r10,%rdi), %r9d #43.13
  movl %r9d, count(,%r13,4) #16.25
  jmp ..B1.10 # Prob 100% #16.25
..B1.9: # Preds ..B1.2
  movl 32+perm1(%rip), %ecx #18.47
  movl 36+perm1(%rip), %esi #18.47
  movl 40+perm1(%rip), %r8d #18.47
..B1.10: # Preds ..B1.8 ..B1.7 ..B1.9
  movdqu perm1(%rip), %xmm1 #18.47
  movl %ecx, 32+perm(%rip) #18.37
  movd %xmm1, %ecx #22.21
  movdqu 16+perm1(%rip), %xmm0 #18.47
  movdqu %xmm1, perm(%rip) #18.37
  movdqu %xmm0, 16+perm(%rip) #18.37
  movl %esi, 36+perm(%rip) #18.37
  xorl %esi, %esi #20.19
  movl %r8d, 40+perm(%rip) #18.37
  testl %ecx, %ecx #22.33
  je ..B1.24 # Prob 10% #22.33
..B1.11: # Preds ..B1.10
  movl $1, %esi #22.9
  lea 1(%rcx), %edi #23.27
  sarl $1, %edi #23.33
  testl %edi, %edi #24.33
  jle ..B1.44 # Prob 10% #24.33
..B1.12: # Preds ..B1.11
  lea 1(%rcx), %eax #23.27
  sarl $1, %eax #23.33
..B1.13: # Preds ..B1.21 ..B1.12
  movl %eax, %r8d #24.13
  movl $1, %r9d #24.13
  xorl %edx, %edx #24.13
  shrl $1, %r8d #24.13
  je ..B1.17 # Prob 0% #24.13
..B1.14: # Preds ..B1.13
  movslq %ecx, %rdi #26.27
  shlq $2, %rdi #26.27
..B1.15: # Preds ..B1.15 ..B1.14
  movl perm(%rdi), %r9d #26.27
  movl perm(,%rdx,8), %r10d #25.25
  movl %r9d, perm(,%rdx,8) #26.17
  movl %r10d, perm(%rdi) #27.17
  movl -4+perm(%rdi), %r11d #26.27
  movl 4+perm(,%rdx,8), %r15d #25.25
  movl %r11d, 4+perm(,%rdx,8) #26.17
  incq %rdx #24.13
  movl %r15d, -4+perm(%rdi) #27.17
  addq $-8, %rdi #24.13
  cmpq %r8, %rdx #24.13
  jb ..B1.15 # Prob 87% #24.13
..B1.16: # Preds ..B1.15
  lea 1(%rdx,%rdx), %r9d #25.25
..B1.17: # Preds ..B1.16 ..B1.13
  lea -1(%r9), %edi #24.13
  cmpl %eax, %edi #24.13
  jae ..B1.19 # Prob 0% #24.13
..B1.18: # Preds ..B1.17
  movslq %r9d, %r9 #25.25
  movslq %ecx, %rcx #26.27
  subq %r9, %rcx #26.27
  movl -4+perm(,%r9,4), %r8d #25.25
  movl 4+perm(,%rcx,4), %edi #26.27
  movl %edi, -4+perm(,%r9,4) #26.17
  movl %r8d, 4+perm(,%rcx,4) #27.17
..B1.19: # Preds ..B1.17 ..B1.18
  movl perm(%rip), %ecx #22.21
..B1.20: # Preds ..B1.19 ..B1.21
  testl %ecx, %ecx #22.33
  je ..B1.24 # Prob 18% #22.33
..B1.21: # Preds ..B1.44 ..B1.20
  incl %esi #22.9
  lea 1(%rcx), %eax #23.27
  sarl $1, %eax #23.33
  testl %eax, %eax #24.33
  jle ..B1.20 # Prob 10% #24.33
  jmp ..B1.13 # Prob 100% #24.33
..B1.24: # Preds ..B1.20 ..B1.10
  cmpl %r14d, %esi #32.9
  movl %esi, %edi #33.36
  cmovg %esi, %r14d #32.9
  negl %edi #33.36
  testl $1, %r12d #33.30
  movl %r14d, maxflips(%rip) #32.31
  cmovne %edi, %esi #33.9
  incl %r12d #34.9
  addl %esi, %ebx #33.9
  movl %ebx, checksum(%rip) #33.9
  jmp ..B1.25 # Prob 100% #33.9
..B1.31: # Preds ..B1.30
  incq %r13 #43.13
..B1.25: # Preds ..B1.31 ..B1.24
  cmpq $11, %r13 #37.22
  je ..B1.42 # Prob 6% #37.22
..B1.26: # Preds ..B1.25
  movl perm1(%rip), %r15d #38.22
  testq %r13, %r13 #39.33
  jle ..B1.30 # Prob 50% #39.33
..B1.27: # Preds ..B1.26
  cmpq $24, %r13 #39.13
  jle ..B1.33 # Prob 10% #39.13
..B1.28: # Preds ..B1.27
  xorl %r8d, %r8d #39.13
  lea (,%r13,4), %rdx #39.13
  cmpq $-4, %rdx #39.13
  setl %r8b #39.13
  xorl %edi, %edi #39.13
  cmpq $4, %rdx #39.13
  setl %dil #39.13
  orl %edi, %r8d #39.13
  je ..B1.33 # Prob 10% #39.13
..B1.29: # Preds ..B1.28
  movl $perm1, %edi #39.13
  movl $perm1+4, %esi #39.13
  call _intel_fast_memcpy #39.13
..B1.30: # Preds ..B1.39 ..B1.37 ..B1.26 ..B1.29
  movl count(,%r13,4), %edi #41.13
  decl %edi #41.13
  movl %r15d, perm1(,%r13,4) #40.13
  movl %edi, count(,%r13,4) #41.13
  testl %edi, %edi #42.28
  jg ..B1.2 # Prob 20% #42.28
  jmp ..B1.31 # Prob 100% #42.28
..B1.33: # Preds ..B1.27 ..B1.28
  movslq %r13d, %r9 #39.13
  cmpq $4, %r9 #39.13
  jl ..B1.41 # Prob 10% #39.13
..B1.34: # Preds ..B1.33
  movl %r9d, %r8d #39.13
  xorl %edi, %edi #39.13
  andl $-4, %r8d #39.13
  movslq %r8d, %r8 #39.13
..B1.35: # Preds ..B1.35 ..B1.34
  movdqu 4+perm1(,%rdi,4), %xmm0 #39.52
  movdqu %xmm0, perm1(,%rdi,4) #39.41
  addq $4, %rdi #39.13
  cmpq %r8, %rdi #39.13
  jb ..B1.35 # Prob 93% #39.13
..B1.37: # Preds ..B1.35 ..B1.41
  cmpq %r9, %r8 #39.13
  jae ..B1.30 # Prob 10% #39.13
..B1.39: # Preds ..B1.37 ..B1.39
  movl 4+perm1(,%r8,4), %edi #39.52
  movl %edi, perm1(,%r8,4) #39.41
  incq %r8 #39.13
  cmpq %r9, %r8 #39.13
  jb ..B1.39 # Prob 93% #39.13
  jmp ..B1.30 # Prob 100% #39.13
..B1.41: # Preds ..B1.33
  xorl %r8d, %r8d #39.13
  jmp ..B1.37 # Prob 100% #39.13
..B1.42: # Preds ..B1.25
  movl %ebx, %esi #
  movl %r14d, %ecx #
  movl $.L_2__STRING.0, %edi #51.5
  movl $11, %edx #51.5
  xorl %eax, %eax #51.5
  call printf #51.5
..B1.43: # Preds ..B1.42
  xorl %eax, %eax #52.12
  addq $88, %rsp #52.12
  popq %rbx #52.12
  popq %r15 #52.12
  popq %r14 #52.12
  popq %r13 #52.12
  popq %r12 #52.12
  movq %rbp, %rsp #52.12
  popq %rbp #52.12
  ret #52.12
..B1.44: # Preds ..B1.11
  movl $4, %esi #22.9
  jmp ..B1.21 # Prob 100% #22.9
perm1:
count:
perm:
maxflips:
checksum:
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2__STRING.0:
