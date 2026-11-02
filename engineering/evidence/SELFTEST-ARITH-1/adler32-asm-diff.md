```
$ diff -u <(lccc-base -O2 -S zlib_ng_adler32.c) <(lccc-new -O2 -S zlib_ng_adler32.c)
--- /tmp/zl.base.s
+++ /tmp/zl.new.s
@@ -185,7 +185,6 @@
     subl $1, %ebx
     movq %r8, %rdi
     movq %r10, %r9
-    testl %ebx, %ebx
 jne .LBB13
 .LBB14:
```
