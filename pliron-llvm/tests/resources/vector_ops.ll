; LLVM IR test: vector_ops.ll
; Tests vector operations including constant vectors, shufflevector,
; insertelement/extractelement, integer and floating-point vector ops,
; and floating-point vector comparisons (fcmp yielding <N x i1>).
; main returns 0 on success, 1 on failure.

; ModuleID and source filename (optional)
; ModuleID = 'vector_ops_test'
source_filename = "vector_ops.ll"

@const_i32x4 = constant <4 x i32> <i32 1, i32 2, i32 3, i32 4>
@const_i32x4_b = constant <4 x i32> <i32 10, i32 20, i32 30, i32 40>
@const_f64x2 = constant <2 x double> <double 1.0, double 2.0>

define i32 @main() {
entry:
  ; load constant integer vectors and add
  %a = load <4 x i32>, <4 x i32>* @const_i32x4
  %b = load <4 x i32>, <4 x i32>* @const_i32x4_b
  %sum = add <4 x i32> %a, %b                    ; expected: <11,22,33,44>

  ; extract individual elements
  %s0 = extractelement <4 x i32> %sum, i32 0     ; 11
  %s2 = extractelement <4 x i32> %sum, i32 2     ; 33

  ; shuffle test: reorder elements (2,0,1,3) -> <33,11,22,44>
  %sh = shufflevector <4 x i32> %sum, <4 x i32> %sum, <4 x i32> <i32 2, i32 0, i32 1, i32 3>
  %sh0 = extractelement <4 x i32> %sh, i32 0     ; 33

  ; compare integer results
  %c1 = icmp eq i32 %s2, 33
  %c1b = icmp eq i32 %sh0, 33

  ; floating-point vector add and convert
  %fvec = load <2 x double>, <2 x double>* @const_f64x2
  %fadd = fadd <2 x double> %fvec, <double 3.0, double 4.0> ; <4.0,6.0>
  %fe0 = extractelement <2 x double> %fadd, i32 0
  %fe0_i = fptosi double %fe0 to i32                  ; 4
  %c2 = icmp eq i32 %fe0_i, 4

  ; float vector comparison
  %fcmp = fcmp olt <2 x double> %fadd, <double 5.0, double 5.0> ; <true,false>
  %fc0 = extractelement <2 x i1> %fcmp, i32 0           ; true
  %fc1 = extractelement <2 x i1> %fcmp, i32 1           ; false
  %fc1n = xor i1 %fc1, true
  %c4 = and i1 %fc0, %fc1n

  ; insertelement / extractelement test
  %vundef = insertelement <4 x i32> undef, i32 7, i32 0
  %v2 = insertelement <4 x i32> %vundef, i32 8, i32 1
  %ex1 = extractelement <4 x i32> %v2, i32 1            ; 8
  %c3 = icmp eq i32 %ex1, 8

  ; combine all checks -> return 0 on success, 1 on failure
  %a1 = and i1 %c1, %c1b
  %a2 = and i1 %a1, %c2
  %a3 = and i1 %a2, %c3
  %all = and i1 %a3, %c4
  %ret = select i1 %all, i32 0, i32 1
  ret i32 %ret
}
