bc_scan:
  movq %rdi, %rax
  cmpq %rsi, %rdi
  jae .LBB0_4
.LBB0_1:
  movzbl (%rax), %ecx
  cmpb (%rdx), %cl
  jne .LBB0_4
  incq %rax
  incq %rdx
  cmpq %rsi, %rax
  jne .LBB0_1
  movq %rsi, %rax
.LBB0_4:
  retq

bc_scan_signed:
  movq %rdi, %rax
  cmpq %rsi, %rdi
  jae .LBB1_4
.LBB1_1:
  movzbl (%rax), %ecx
  cmpb (%rdx), %cl
  jne .LBB1_4
  incq %rax
  incq %rdx
  cmpq %rsi, %rax
  jne .LBB1_1
  movq %rsi, %rax
.LBB1_4:
  retq

match_extend:
  leaq 8(%rdi), %rax
  cmpq %rdx, %rax
  jbe .LBB2_2
  movq %rdi, %rax
  cmpq %rdx, %rax
  jb .LBB2_7
  jmp .LBB2_10
.LBB2_2:
  movq %rdi, %rcx
.LBB2_3:
  movq (%rcx), %rax
  cmpq (%rsi), %rax
  jne .LBB2_4
  addq $8, %rsi
  leaq 8(%rcx), %rax
  addq $16, %rcx
  cmpq %rdx, %rcx
  movq %rax, %rcx
  jbe .LBB2_3
  cmpq %rdx, %rax
  jb .LBB2_7
.LBB2_10:
  subq %rdi, %rax
  retq
.LBB2_4:
  movq %rcx, %rax
  cmpq %rdx, %rax
  jae .LBB2_10
.LBB2_7:
  movzbl (%rax), %ecx
  cmpb (%rsi), %cl
  jne .LBB2_10
  incq %rax
  incq %rsi
  cmpq %rdx, %rax
  jne .LBB2_7
  movq %rdx, %rax
  subq %rdi, %rax
  retq
