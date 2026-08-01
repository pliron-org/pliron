; ModuleID = 'const_struct_array'
source_filename = "const_struct_array.ll"

; A NUL-terminated byte string, which round-trips as a `builtin.string` attribute initializer.
@const_str = private constant [15 x i8] c"const sum: %d\0A\00"

declare i32 @printf(ptr, ...)

define i32 @const_array() {
entry:
  ; Create a local copy of the constant array
  %array = alloca [3 x i32]
  store [3 x i32] [i32 11, i32 21, i32 31], [3 x i32]* %array

  ; Extract the elements
  %array_val = load [3 x i32], [3 x i32]* %array
  %elem0 = extractvalue [3 x i32] %array_val, 0
  %elem1 = extractvalue [3 x i32] %array_val, 1
  %elem2 = extractvalue [3 x i32] %array_val, 2

  ; Sum the elements
  %sum1 = add i32 %elem0, %elem1
  %sum2 = add i32 %sum1, %elem2

  ; Return the sum
  ret i32 %sum2
}

define i32 @const_struct() {

entry:
  ; Create a local copy of the constant struct
  %struct = alloca { i32, i32, i32 }
  store { i32, i32, i32 } { i32 10, i32 20, i32 30 }, { i32, i32, i32 }* %struct

  ; Extract the fields
  %struct_val = load { i32, i32, i32 }, { i32, i32, i32 }* %struct
  %field0 = extractvalue { i32, i32, i32 } %struct_val, 0
  %field1 = extractvalue { i32, i32, i32 } %struct_val, 1
  %field2 = extractvalue { i32, i32, i32 } %struct_val, 2

  ; Sum the fields
  %sum1 = add i32 %field0, %field1
  %sum2 = add i32 %sum1, %field2

  ; Return the sum
  ret i32 %sum2
}

define i32 @const_array_of_structs() {
entry:
  ; Create a local copy of the constant array of structs
  %array = alloca [2 x { i32, i32 }]
  store [2 x { i32, i32 }] [ { i32, i32 } { i32 1, i32 2 }, { i32, i32 } { i32 3, i32 4 } ], [2 x { i32, i32 }]* %array

  ; Extract the elements
  %array_val = load [2 x { i32, i32 }], [2 x { i32, i32 }]* %array
  %elem0 = extractvalue [2 x { i32, i32 }] %array_val, 0
  %elem1 = extractvalue [2 x { i32, i32 }] %array_val, 1

  ; Extract fields from the first struct
  %field0_0 = extractvalue { i32, i32 } %elem0, 0
  %field0_1 = extractvalue { i32, i32 } %elem0, 1

  ; Extract fields from the second struct
  %field1_0 = extractvalue { i32, i32 } %elem1, 0
  %field1_1 = extractvalue { i32, i32 } %elem1, 1

  ; Sum all the fields
  %sum1 = add i32 %field0_0, %field0_1
  %sum2 = add i32 %sum1, %field1_0
  %sum3 = add i32 %sum2, %field1_1

  ; Return the sum
  ret i32 %sum3
}

define i32 @const_struct_with_arrays() {
entry:
  ; Create a local copy of the constant struct with two array fields
  %struct = alloca { [2 x i32], [3 x i32] }
  store { [2 x i32], [3 x i32] } { [2 x i32] [i32 1, i32 2], [3 x i32] [i32 3, i32 4, i32 5] }, { [2 x i32], [3 x i32] }* %struct

  ; Extract the struct value
  %struct_val = load { [2 x i32], [3 x i32] }, { [2 x i32], [3 x i32] }* %struct

  ; Extract the first array field
  %array0 = extractvalue { [2 x i32], [3 x i32] } %struct_val, 0
  %array0_elem0 = extractvalue [2 x i32] %array0, 0
  %array0_elem1 = extractvalue [2 x i32] %array0, 1

  ; Extract the second array field
  %array1 = extractvalue { [2 x i32], [3 x i32] } %struct_val, 1
  %array1_elem0 = extractvalue [3 x i32] %array1, 0
  %array1_elem1 = extractvalue [3 x i32] %array1, 1
  %array1_elem2 = extractvalue [3 x i32] %array1, 2

  ; Sum all the elements
  %sum1 = add i32 %array0_elem0, %array0_elem1
  %sum2 = add i32 %sum1, %array1_elem0
  %sum3 = add i32 %sum2, %array1_elem1
  %sum4 = add i32 %sum3, %array1_elem2

  ; Return the sum
  ret i32 %sum4
}

define i32 @const_aggregate_zero() {
entry:
  ; Zero-initialized array [3 x i32]
  %arr = alloca [3 x i32]
  store [3 x i32] zeroinitializer, [3 x i32]* %arr
  %arr_val = load [3 x i32], [3 x i32]* %arr
  %arr_e0 = extractvalue [3 x i32] %arr_val, 0
  %arr_e1 = extractvalue [3 x i32] %arr_val, 1
  %arr_e2 = extractvalue [3 x i32] %arr_val, 2

  ; Zero-initialized struct { i32, [2 x i32] }
  %st = alloca { i32, [2 x i32] }
  store { i32, [2 x i32] } zeroinitializer, { i32, [2 x i32] }* %st
  %st_val = load { i32, [2 x i32] }, { i32, [2 x i32] }* %st
  %st_f0 = extractvalue { i32, [2 x i32] } %st_val, 0
  %st_arr = extractvalue { i32, [2 x i32] } %st_val, 1
  %st_a0 = extractvalue [2 x i32] %st_arr, 0
  %st_a1 = extractvalue [2 x i32] %st_arr, 1

  ; Sum all extracted values (should be 0)
  %s1 = add i32 %arr_e0, %arr_e1
  %s2 = add i32 %s1, %arr_e2
  %s3 = add i32 %s2, %st_f0
  %s4 = add i32 %s3, %st_a0
  %s5 = add i32 %s4, %st_a1

  ret i32 %s5
}

define i32 @const_expr() {
entry:
  ; Return the result of the constant expression
  ret i32
    sub(
      i32 add(
        i32 ptrtoint (i32* getelementptr (i32, i32* inttoptr (i32 11 to i32*), i32 1) to i32),
        i32 ptrtoint (i32* getelementptr (i32, i32* null, i32 13) to i32)
      ),
      i32 add(
        i32 ptrtoint (i32* getelementptr (i32, i32* null, i32 1) to i32),
        i32 ptrtoint (i32* getelementptr (i32, i32* null, i32 2) to i32)
      )
    )
}

; Print through a variadic call and return what printf reports it wrote.
define i32 @const_printf() {
entry:
  ; "const sum: 42\n" is 14 characters, so this returns 14.
  %n = call i32 (ptr, ...) @printf(ptr @const_str, i32 42)
  ret i32 %n
}

define i32 @main() {
entry:
  ; Call the const_array function
  %result1 = call i32 @const_array()

  ; Call the const_struct function
  %result2 = call i32 @const_struct()

  ; Call the const_array_of_structs function
  %result3 = call i32 @const_array_of_structs()

  ; Call the const_struct_with_arrays function
  %result4 = call i32 @const_struct_with_arrays()

  ; Call the const_expr function
  %result5 = call i32 @const_expr()

  ; Call the const_aggregate_zero function
  %result6 = call i32 @const_aggregate_zero()

  ; Call the const_printf function
  %result7 = call i32 @const_printf()

  ; Add the results to the final sum
  %temp_sum0 = add i32 %result1, %result2
  %temp_sum1 = add i32 %temp_sum0, %result3
  %temp_sum2 = add i32 %temp_sum1, %result4
  %temp_sum3 = add i32 %temp_sum2, %result5
  %temp_sum4 = sub i32 %temp_sum3, %result6
  %temp_sum5 = add i32 %temp_sum4, %result7

  ; Return the final sum
  ret i32 %temp_sum5
}
