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
  movq %rdi, %rax #22.1
  movq %rsi, %r10 #22.1
  movq %rax, %r8 #22.1
  movq %r10, %rdi #22.1
  movq %rax, %r9 #23.34
  lea 8(%rax), %rcx #24.12
  cmpq %rdx, %rcx #24.21
  ja ..B3.8 # Prob 0% #24.21
..B3.2: # Preds ..B3.1
  xorl %ecx, %ecx #24.5
..B3.3: # Preds ..B3.6 ..B3.2
  addq $16, %rcx #24.5
  lea (%r8,%rcx), %rsi #24.54
  movq -16(%rsi), %r11 #24.54
  cmpq -16(%rcx,%rdi), %r11 #24.83
  jne ..B3.8 # Prob 20% #24.83
..B3.4: # Preds ..B3.3
  lea -8(%r8,%rcx), %rax #27.37
  lea -8(%rdi,%rcx), %r10 #27.37
  cmpq %rdx, %rsi #24.21
  ja ..B3.8 # Prob 18% #24.21
..B3.5: # Preds ..B3.4
  movq -8(%rsi), %r11 #24.54
  cmpq -8(%rcx,%rdi), %r11 #24.83
  jne ..B3.8 # Prob 20% #24.83
..B3.6: # Preds ..B3.5
  movq %rsi, %rax #25.9
  lea 8(%r8,%rcx), %rsi #24.12
  lea (%rdi,%rcx), %r10 #27.37
  cmpq %rdx, %rsi #24.21
  jbe ..B3.3 # Prob 82% #24.21
..B3.8: # Preds ..B3.4 ..B3.5 ..B3.3 ..B3.6 ..B3.1
  cmpq %rdx, %rax #27.16
  jae ..B3.13 # Prob 10% #27.16
..B3.10: # Preds ..B3.8 ..B3.11
  movb (%rax), %cl #27.26
  cmpb (%r10), %cl #27.32
  jne ..B3.13 # Prob 20% #27.32
..B3.11: # Preds ..B3.10
  incq %rax #27.37
  incq %r10 #27.42
  cmpq %rdx, %rax #27.16
  jb ..B3.10 # Prob 82% #27.16
..B3.13: # Preds ..B3.10 ..B3.11 ..B3.8
  subq %r9, %rax #28.21
  ret #28.25
