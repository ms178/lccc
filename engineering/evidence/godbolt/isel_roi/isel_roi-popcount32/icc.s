popcount32:
..B1.1: # Preds ..B1.0
  movl %edi, %eax #12.55
  shrl $1, %eax #12.55
  andl $1431655765, %eax #12.55
  subl %eax, %edi #12.55
  movl %edi, %ecx #12.55
  andl $858993459, %edi #12.55
  shrl $2, %ecx #12.55
  andl $858993459, %ecx #12.55
  addl %edi, %ecx #12.55
  movl %ecx, %edx #12.55
  shrl $4, %edx #12.55
  addl %edx, %ecx #12.55
  andl $252645135, %ecx #12.55
  imull $16843009, %ecx, %eax #12.55
  shrl $24, %eax #12.55
  ret #12.55
