.text
.global _start
_start:
    bl test
    mov x8, #93
    svc #0
