; fpops.ll - Test various floating point operations (no math intrinsics, more casts)

define i32 @main() {
entry:
  ; Declare some float constants
  %a = fadd float 1.5, 2.5           ; a = 1.5 + 2.5 = 4.0
  %b = fsub float %a, 1.0            ; b = 4.0 - 1.0 = 3.0
  %c = fmul float %b, 2.0            ; c = 3.0 * 2.0 = 6.0
  %d = fdiv float %c, 3.0            ; d = 6.0 / 3.0 = 2.0
  %e = frem float %d, 1.5            ; e = 2.0 % 1.5 = 0.5

  ; Instead of math intrinsics, just use more arithmetic
  %f = fmul float %c, %d             ; f = 6.0 * 2.0 = 12.0
  %g = fdiv float %f, %a             ; g = 12.0 / 4.0 = 3.0

  ; Cast results to double and back to float
  %a_d = fpext float %a to double
  %b_d = fpext float %b to double
  %c_d = fpext float %c to double
  %d_d = fpext float %d to double
  %e_d = fpext float %e to double
  %f_d = fpext float %f to double
  %g_d = fpext float %g to double

  %a_f = fptrunc double %a_d to float
  %b_f = fptrunc double %b_d to float
  %c_f = fptrunc double %c_d to float
  %d_f = fptrunc double %d_d to float
  %e_f = fptrunc double %e_d to float
  %f_f = fptrunc double %f_d to float
  %g_f = fptrunc double %g_d to float

  ; Combine all results into a single float value
  %sum1 = fadd float %a_f, %b_f
  %sum2 = fadd float %c_f, %d_f
  %sum3 = fadd float %e_f, %f_f
  %sum4 = fadd float %sum1, %sum2
  %sum5 = fadd float %sum3, %g_f
  %final_sum = fadd float %sum4, %sum5

  ; Cast final result to int and return
  %result2 = fptosi float %final_sum to i32
  %result1 = fptoui float %sum5 to i32
  %result = add i32 %result2, %result1
  ret i32 %result
}