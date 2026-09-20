branch_index_store:
..B5.1: # Preds ..B5.0
  xorl %ecx, %ecx #75.11
  xorl %edx, %edx #75.11
  testq %rdi, %rdi #76.12
  je ..B5.8 # Prob 10% #76.12
..B5.2: # Preds ..B5.1
  movl $64, %eax #77.17
..B5.3: # Preds ..B5.6 ..B5.2
  bsf %rdi, %rsi #77.17
  cmove %rax, %rsi #77.17
  cmpl %ecx, %esi #78.18
  jne ..B5.5 # Prob 50% #78.18
..B5.4: # Preds ..B5.3
  movl %ecx, branch_slots(,%rdx,4) #79.13
  jmp ..B5.6 # Prob 100% #79.13
..B5.5: # Preds ..B5.3
  movslq %esi, %rsi #81.31
  movl branch_slots(,%rsi,4), %r8d #81.31
  movl %r8d, branch_slots(,%rdx,4) #81.13
..B5.6: # Preds ..B5.4 ..B5.5
  incq %rdx #82.9
  lea -1(%rdi), %rsi #83.24
  incl %ecx #82.9
  andq %rsi, %rdi #83.9
  jne ..B5.3 # Prob 82% #76.12
..B5.8: # Preds ..B5.6 ..B5.1
  ret #85.1
