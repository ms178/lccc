bc_scan:
  movq %rdi, %rax
  cmpq %rsi, %rdi
  jae .LBB0_9
  movq %rsi, %rcx
  subq %rax, %rcx
  cmpq $2, %rcx
  jb .LBB0_6
  movq %rcx, %r8
  shrq %r8
  leaq 1(%rax), %rdi
  xorl %r9d, %r9d
.LBB0_3:
  movzbl -1(%rdi), %r10d
  cmpb (%rdx,%r9,2), %r10b
  jne .LBB0_13
  movzbl (%rdi), %r10d
  cmpb 1(%rdx,%r9,2), %r10b
  jne .LBB0_14
  incq %r9
  addq $2, %rdi
  cmpq %r9, %r8
  jne .LBB0_3
.LBB0_6:
  movq %rcx, %rdi
  andq $-2, %rdi
  cmpq %rcx, %rdi
  jb .LBB0_11
  cmpq %rax, %rsi
  je .LBB0_11
.LBB0_8:
  movq %rsi, %rax
.LBB0_9:
  retq
.LBB0_10:
  incq %rdi
  cmpq %rdi, %rcx
  je .LBB0_8
.LBB0_11:
  movzbl (%rax,%rdi), %r8d
  cmpb (%rdx,%rdi), %r8b
  je .LBB0_10
  addq %rdi, %rax
  retq
.LBB0_13:
  decq %rdi
.LBB0_14:
  movq %rdi, %rax
  retq

bc_scan_signed:
  movq %rdi, %rax
  cmpq %rsi, %rdi
  jae .LBB1_9
  movq %rsi, %rcx
  subq %rax, %rcx
  cmpq $2, %rcx
  jb .LBB1_6
  movq %rcx, %r8
  shrq %r8
  leaq 1(%rax), %rdi
  xorl %r9d, %r9d
.LBB1_3:
  movzbl -1(%rdi), %r10d
  cmpb (%rdx,%r9,2), %r10b
  jne .LBB1_13
  movzbl (%rdi), %r10d
  cmpb 1(%rdx,%r9,2), %r10b
  jne .LBB1_14
  incq %r9
  addq $2, %rdi
  cmpq %r9, %r8
  jne .LBB1_3
.LBB1_6:
  movq %rcx, %rdi
  andq $-2, %rdi
  cmpq %rcx, %rdi
  jb .LBB1_11
  cmpq %rax, %rsi
  je .LBB1_11
.LBB1_8:
  movq %rsi, %rax
.LBB1_9:
  retq
.LBB1_10:
  incq %rdi
  cmpq %rdi, %rcx
  je .LBB1_8
.LBB1_11:
  movzbl (%rax,%rdi), %r8d
  cmpb (%rdx,%rdi), %r8b
  je .LBB1_10
  addq %rdi, %rax
  retq
.LBB1_13:
  decq %rdi
.LBB1_14:
  movq %rdi, %rax
  retq

match_extend:
  leaq 8(%rdi), %rax
  cmpq %rdx, %rax
  jbe .LBB2_2
  movq %rdi, %rcx
  movq %rcx, %rax
  cmpq %rdx, %rcx
  jae .LBB2_6
  jmp .LBB2_8
.LBB2_2:
  movq %rdi, %rax
.LBB2_3:
  movq (%rax), %rcx
  cmpq (%rsi), %rcx
  jne .LBB2_7
  addq $8, %rsi
  leaq 8(%rax), %rcx
  addq $16, %rax
  cmpq %rdx, %rax
  movq %rcx, %rax
  jbe .LBB2_3
  movq %rcx, %rax
  cmpq %rdx, %rcx
  jb .LBB2_8
  jmp .LBB2_6
.LBB2_7:
  movq %rax, %rcx
  movq %rcx, %rax
  cmpq %rdx, %rcx
  jae .LBB2_6
.LBB2_8:
  movq %rdx, %r8
  subq %rcx, %r8
  cmpq $2, %r8
  jb .LBB2_13
  movq %r8, %r9
  shrq %r9
  leaq 1(%rcx), %rax
  xorl %r10d, %r10d
.LBB2_10:
  movzbl -1(%rax), %r11d
  cmpb (%rsi,%r10,2), %r11b
  jne .LBB2_19
  movzbl (%rax), %r11d
  cmpb 1(%rsi,%r10,2), %r11b
  jne .LBB2_6
  incq %r10
  addq $2, %rax
  cmpq %r10, %r9
  jne .LBB2_10
.LBB2_13:
  movq %r8, %rax
  andq $-2, %rax
  cmpq %r8, %rax
  jb .LBB2_17
  cmpq %rdx, %rcx
  je .LBB2_17
.LBB2_15:
  movq %rdx, %rax
  subq %rdi, %rax
  retq
.LBB2_16:
  incq %rax
  cmpq %rax, %r8
  je .LBB2_15
.LBB2_17:
  movzbl (%rcx,%rax), %r9d
  cmpb (%rsi,%rax), %r9b
  je .LBB2_16
  addq %rax, %rcx
  movq %rcx, %rax
  subq %rdi, %rax
  retq
.LBB2_6:
  subq %rdi, %rax
  retq
.LBB2_19:
  decq %rax
  subq %rdi, %rax
  retq
