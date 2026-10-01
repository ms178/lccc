bc_scan:
..B1.1: # Preds ..B1.0
  cmpq %rsi, %rdi #8.16
  jae ..B1.6 # Prob 10% #8.16
..B1.3: # Preds ..B1.1 ..B1.4
  movb (%rdi), %cl #8.24
  cmpb (%rdx), %cl #8.30
  jne ..B1.6 # Prob 20% #8.30
..B1.4: # Preds ..B1.3
  incq %rdi #8.35
  incq %rdx #8.40
  cmpq %rsi, %rdi #8.16
  jb ..B1.3 # Prob 82% #8.16
..B1.6: # Preds ..B1.3 ..B1.4 ..B1.1
  movq %rdi, %rax #9.12
  ret #9.12
bc_scan_signed:
..B2.1: # Preds ..B2.0
  cmpq %rsi, %rdi #15.16
  jae ..B2.6 # Prob 10% #15.16
..B2.3: # Preds ..B2.1 ..B2.4
  movb (%rdi), %cl #15.24
  cmpb (%rdx), %cl #15.30
  jne ..B2.6 # Prob 20% #15.30
..B2.4: # Preds ..B2.3
  incq %rdi #15.35
  incq %rdx #15.40
  cmpq %rsi, %rdi #15.16
  jb ..B2.3 # Prob 82% #15.16
..B2.6: # Preds ..B2.3 ..B2.4 ..B2.1
  movq %rdi, %rax #16.12
  ret #16.12
match_extend:
..B3.1: # Preds ..B3.0
  movq %rdx, %rcx #22.1
  movq %rdi, %rdx #23.34
  lea 8(%rdi), %r8 #24.12
  cmpq %rcx, %r8 #24.21
  ja ..B3.6 # Prob 0% #24.21
..B3.3: # Preds ..B3.1 ..B3.4
  movq (%rdi), %r9 #24.54
  cmpq (%rsi), %r9 #24.83
  jne ..B3.6 # Prob 20% #24.83
..B3.4: # Preds ..B3.3
  movq %r8, %rdi #25.9
  addq $8, %r8 #24.12
  addq $8, %rsi #25.17
  cmpq %rcx, %r8 #24.21
  jbe ..B3.3 # Prob 82% #24.21
..B3.6: # Preds ..B3.3 ..B3.4 ..B3.1
  cmpq %rcx, %rdi #27.16
  jae ..B3.11 # Prob 10% #27.16
..B3.8: # Preds ..B3.6 ..B3.9
  movb (%rdi), %r8b #27.26
  cmpb (%rsi), %r8b #27.32
  jne ..B3.11 # Prob 20% #27.32
..B3.9: # Preds ..B3.8
  incq %rdi #27.37
  incq %rsi #27.42
  cmpq %rcx, %rdi #27.16
  jb ..B3.8 # Prob 82% #27.16
..B3.11: # Preds ..B3.8 ..B3.9 ..B3.6
  subq %rdx, %rdi #28.21
  movq %rdi, %rax #28.25
  ret #28.25
