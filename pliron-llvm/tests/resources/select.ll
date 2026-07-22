; Function to test the select instruction
define i32 @test_select(i1 %cond, i32 %a, i32 %b) {
entry:
  ; Use the select instruction to choose between %a and %b based on %cond
  %result = select i1 %cond, i32 %a, i32 %b
  ret i32 %result
}

; Function to test the select instruction with fast-math flags
define i32 @test_select_fmf(i1 %cond, float %a, float %b) {
entry:
  ; Select between float values; the fast-math flags must survive the round trip
  %result = select nnan nsz i1 %cond, float %a, float %b
  %int = fptosi float %result to i32
  ret i32 %int
}

; Returns 1 if %x is NaN, 0 otherwise
define i32 @is_nan(float %x) {
entry:
  %cmp = fcmp uno float %x, %x
  %r = zext i1 %cmp to i32
  ret i32 %r
}

; Without fast-math flags a NaN operand is fully defined: the select simply
; returns the chosen value, NaN or not.
define i32 @test_select_nan(i1 %cond) {
entry:
  %sel1 = select i1 %cond, float 0x7FF8000000000000, float 1.0
  %sel2 = select i1 %cond, float 2.0, float 0x7FF8000000000000
  %nan1 = call i32 @is_nan(float %sel1)
  %nan2 = call i32 @is_nan(float %sel2)
  %nan1x10 = mul i32 %nan1, 10
  %r = add i32 %nan1x10, %nan2
  ret i32 %r
}

; Main function
define i32 @main() {
entry:
  ; Call test_select with different inputs
  %call1 = call i32 @test_select(i1 true, i32 10, i32 20)
  %call2 = call i32 @test_select(i1 false, i32 30, i32 40)
  %call3 = call i32 @test_select(i1 true, i32 50, i32 60)
  %call4 = call i32 @test_select_fmf(i1 false, float 5.0, float 0.0)
  %call5 = call i32 @test_select_nan(i1 true)
  %call6 = call i32 @test_select_nan(i1 false)

  ; With nnan, a NaN operand makes the result poison, so results are kept out of the exit code.
  %poison1 = call i32 @test_select_fmf(i1 true, float 0x7FF8000000000000, float 1.0)
  %poison2 = call i32 @test_select_fmf(i1 true, float 1.0, float 0x7FF8000000000000)

  ; Sum the results
  %sum1 = add i32 %call1, %call2
  %sum2 = add i32 %sum1, %call3
  %sum3 = add i32 %sum2, %call4
  %sum4 = add i32 %sum3, %call5
  %sum5 = add i32 %sum4, %call6

  ; Return the sum
  ret i32 %sum5
}