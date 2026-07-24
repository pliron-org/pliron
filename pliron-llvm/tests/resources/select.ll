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

; Selects between NaN and %a without fast-math flags, which is fully defined.
; Returns 1 if the select result is NaN, 0 otherwise.
define i32 @test_select_nan(i1 %cond, float %a) {
entry:
  %sel = select i1 %cond, float 0x7FF8000000000000, float %a
  %isnan = fcmp uno float %sel, %sel
  %r = select i1 %isnan, i32 1, i32 0
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
  %call5 = call i32 @test_select_nan(i1 true, float 1.0)
  %call6 = call i32 @test_select_nan(i1 false, float 1.0)

  %call7 = call i32 @test_select_fmf(i1 true, float 1.0, float 0x7FF8000000000000)

  ; Sum the results
  %sum1 = add i32 %call1, %call2
  %sum2 = add i32 %sum1, %call3
  %sum3 = add i32 %sum2, %call4
  %sum4 = add i32 %sum3, %call5
  %sum5 = add i32 %sum4, %call6
  %sum6 = add i32 %sum5, %call7

  ; Return the sum
  ret i32 %sum6
}