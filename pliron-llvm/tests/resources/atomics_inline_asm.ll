define i32 @atomics_asm(ptr %p, i32 %v) {
entry:
  %old = atomicrmw add ptr %p, i32 %v monotonic
  %cx = cmpxchg ptr %p, i32 20, i32 33 seq_cst monotonic
  fence seq_cst
  %la = load atomic i32, ptr %p monotonic, align 4
  store atomic i32 %la, ptr %p release, align 4
  call void asm sideeffect "", ""()
  ret i32 %la
}

define i32 @main() {
entry:
  %p = alloca i32, align 4
  store i32 7, ptr %p, align 4
  %ret = call i32 @atomics_asm(ptr %p, i32 13)
  ret i32 %ret
}
