; For Unix x86_64 platforms, va_list is the following struct:
%struct.va_list = type { i32, i32, ptr, ptr }

define i32 @test(i32 %X, ...) {
  ; Initialize variable argument processing
  %ap = alloca %struct.va_list
  call void @llvm.va_start.p0(ptr %ap)

  ; Read a single integer argument
  %tmp = va_arg ptr %ap, i32

  ; Stop processing of arguments.
  call void @llvm.va_end.p0(ptr %ap)
  ret i32 %tmp
}

declare void @llvm.va_start.p0(ptr)
declare void @llvm.va_end.p0(ptr)

; Test function to call test with variable arguments
define i32 @main() {
  ; Call test with first fixed argument as 42 and variable argument as 75
  %result = call i32 (i32, ...) @test(i32 42, i32 75)
  ret i32 %result
}