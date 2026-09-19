branch_index_store:
..B6.1: # Preds ..B6.0
  xorl %ecx, %ecx #75.11
  xorl %edx, %edx #75.11
  testq %rdi, %rdi #76.12
  je ..B6.8 # Prob 10% #76.12
..B6.2: # Preds ..B6.1
  movl $64, %eax #77.17
..B6.3: # Preds ..B6.6 ..B6.2
  bsf %rdi, %rsi #77.17
  cmove %rax, %rsi #77.17
  cmpl %ecx, %esi #78.18
  jne ..B6.5 # Prob 50% #78.18
..B6.4: # Preds ..B6.3
  movl %ecx, branch_slots(,%rdx,4) #79.13
  jmp ..B6.6 # Prob 100% #79.13
..B6.5: # Preds ..B6.3
  movslq %esi, %rsi #81.31
  movl branch_slots(,%rsi,4), %r8d #81.31
  movl %r8d, branch_slots(,%rdx,4) #81.13
..B6.6: # Preds ..B6.4 ..B6.5
  incq %rdx #82.9
  lea -1(%rdi), %rsi #83.24
  incl %ecx #82.9
  andq %rsi, %rdi #83.9
  jne ..B6.3 # Prob 82% #76.12
..B6.8: # Preds ..B6.6 ..B6.1
  ret #85.1
