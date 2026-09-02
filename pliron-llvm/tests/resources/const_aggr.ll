; ModuleID = 'const_aggr'
source_filename = "const_aggr.ll"

; Constant aggregates, vector splats, byte strings and symbol addresses, both as
; global initializers and as constants in a function body.

@g = global i32 7
@other = global i32 3

@array = global [4 x i32] [i32 1, i32 2, i32 3, i32 4]
@struct = global { i32, i8, ptr } { i32 10, i8 20, ptr @g }
@nested = global [2 x { i32, i32 }] [{ i32, i32 } { i32 1, i32 2 },
                                     { i32, i32 } { i32 3, i32 4 }]
@vector = global <4 x i32> <i32 1, i32 2, i32 3, i32 4>
@splat = global <4 x i32> splat (i32 5)
@addr = global ptr @g
@vtable = global [2 x ptr] [ptr @g, ptr @other]
@string = global [6 x i8] c"hello\00"

; 1 + 2 + 3 + 4 = 10
define i32 @global_array() {
entry:
  %v = load [4 x i32], ptr @array
  %e0 = extractvalue [4 x i32] %v, 0
  %e1 = extractvalue [4 x i32] %v, 1
  %e2 = extractvalue [4 x i32] %v, 2
  %e3 = extractvalue [4 x i32] %v, 3
  %s0 = add i32 %e0, %e1
  %s1 = add i32 %s0, %e2
  %s2 = add i32 %s1, %e3
  ret i32 %s2
}

; 10 + 20 + *@g = 37
define i32 @global_struct() {
entry:
  %v = load { i32, i8, ptr }, ptr @struct
  %f0 = extractvalue { i32, i8, ptr } %v, 0
  %f1 = extractvalue { i32, i8, ptr } %v, 1
  %f1z = zext i8 %f1 to i32
  %f2 = extractvalue { i32, i8, ptr } %v, 2
  %f2v = load i32, ptr %f2
  %s0 = add i32 %f0, %f1z
  %s1 = add i32 %s0, %f2v
  ret i32 %s1
}

; 1 + 2 + 3 + 4 = 10
define i32 @global_nested() {
entry:
  %v = load [2 x { i32, i32 }], ptr @nested
  %e0 = extractvalue [2 x { i32, i32 }] %v, 0
  %e1 = extractvalue [2 x { i32, i32 }] %v, 1
  %a = extractvalue { i32, i32 } %e0, 0
  %b = extractvalue { i32, i32 } %e0, 1
  %c = extractvalue { i32, i32 } %e1, 0
  %d = extractvalue { i32, i32 } %e1, 1
  %s0 = add i32 %a, %b
  %s1 = add i32 %s0, %c
  %s2 = add i32 %s1, %d
  ret i32 %s2
}

; 1 + 2 + 3 + 4 = 10
define i32 @global_vector() {
entry:
  %v = load <4 x i32>, ptr @vector
  %e0 = extractelement <4 x i32> %v, i32 0
  %e1 = extractelement <4 x i32> %v, i32 1
  %e2 = extractelement <4 x i32> %v, i32 2
  %e3 = extractelement <4 x i32> %v, i32 3
  %s0 = add i32 %e0, %e1
  %s1 = add i32 %s0, %e2
  %s2 = add i32 %s1, %e3
  ret i32 %s2
}

; 5 + 5 = 10
define i32 @global_splat() {
entry:
  %v = load <4 x i32>, ptr @splat
  %e0 = extractelement <4 x i32> %v, i32 0
  %e3 = extractelement <4 x i32> %v, i32 3
  %s = add i32 %e0, %e3
  ret i32 %s
}

; *@g + *@other = 10
define i32 @global_symbol_addrs() {
entry:
  %p = load ptr, ptr @addr
  %a = load i32, ptr %p
  %q = getelementptr [2 x ptr], ptr @vtable, i32 0, i32 1
  %qp = load ptr, ptr %q
  %b = load i32, ptr %qp
  %s = add i32 %a, %b
  ret i32 %s
}

; 'o' - 'h' = 7
define i32 @global_string() {
entry:
  %p0 = getelementptr [6 x i8], ptr @string, i32 0, i32 0
  %c0 = load i8, ptr %p0
  %p4 = getelementptr [6 x i8], ptr @string, i32 0, i32 4
  %c4 = load i8, ptr %p4
  %d = sub i8 %c4, %c0
  %r = zext i8 %d to i32
  ret i32 %r
}

; 2 + 3 + 5 = 10
define i32 @local_aggregate() {
entry:
  %arr = alloca [3 x i32]
  store [3 x i32] [i32 2, i32 3, i32 5], ptr %arr
  %v = load [3 x i32], ptr %arr
  %e0 = extractvalue [3 x i32] %v, 0
  %e1 = extractvalue [3 x i32] %v, 1
  %e2 = extractvalue [3 x i32] %v, 2
  %s0 = add i32 %e0, %e1
  %s1 = add i32 %s0, %e2
  ret i32 %s1
}

; 4 + *@other = 7
define i32 @local_struct_with_addr() {
entry:
  %st = alloca { i32, ptr }
  store { i32, ptr } { i32 4, ptr @other }, ptr %st
  %v = load { i32, ptr }, ptr %st
  %f0 = extractvalue { i32, ptr } %v, 0
  %f1 = extractvalue { i32, ptr } %v, 1
  %f1v = load i32, ptr %f1
  %s = add i32 %f0, %f1v
  ret i32 %s
}

; 4 + 4 = 8
define i32 @local_splat() {
entry:
  %v = alloca <4 x i32>
  store <4 x i32> splat (i32 4), ptr %v
  %vv = load <4 x i32>, ptr %v
  %e0 = extractelement <4 x i32> %vv, i32 0
  %e1 = extractelement <4 x i32> %vv, i32 1
  %s = add i32 %e0, %e1
  ret i32 %s
}

; 'o' - 'h' = 7
define i32 @local_bytes() {
entry:
  %buf = alloca [6 x i8]
  store [6 x i8] c"hello\00", ptr %buf
  %p0 = getelementptr [6 x i8], ptr %buf, i32 0, i32 0
  %c0 = load i8, ptr %p0
  %p4 = getelementptr [6 x i8], ptr %buf, i32 0, i32 4
  %c4 = load i8, ptr %p4
  %d = sub i8 %c4, %c0
  %r = zext i8 %d to i32
  ret i32 %r
}

define i32 @main() {
entry:
  %r0 = call i32 @global_array()
  %r1 = call i32 @global_struct()
  %r2 = call i32 @global_nested()
  %r3 = call i32 @global_vector()
  %r4 = call i32 @global_splat()
  %r5 = call i32 @global_symbol_addrs()
  %r6 = call i32 @global_string()
  %r7 = call i32 @local_aggregate()
  %r8 = call i32 @local_struct_with_addr()
  %r9 = call i32 @local_splat()
  %r10 = call i32 @local_bytes()

  %s0 = add i32 %r0, %r1
  %s1 = add i32 %s0, %r2
  %s2 = add i32 %s1, %r3
  %s3 = add i32 %s2, %r4
  %s4 = add i32 %s3, %r5
  %s5 = add i32 %s4, %r6
  %s6 = add i32 %s5, %r7
  %s7 = add i32 %s6, %r8
  %s8 = add i32 %s7, %r9
  %s9 = add i32 %s8, %r10

  ret i32 %s9
}
