@global_int = global i32 42, align 8
@global_array = global [4 x i32] [i32 1, i32 2, i32 3, i32 4]
@global$struct = global { i32, i32 } { i32 10, i32 14 }
@global_external_int = extern_weak global i32
@global_as1 = addrspace(1) global i32 5, align 4

define i32 @main() {
entry:
  ; Round-trip through a non-default address space and then load global_int.
  %global_int_as1 = addrspacecast i32* @global_int to i32 addrspace(1)*
  %global_int_as0 = addrspacecast i32 addrspace(1)* %global_int_as1 to i32*
  %int_val = load i32, i32* %global_int_as0

  ; Round-trip through default address space for a non-zero-AS global and load it.
  %global_as1_as0 = addrspacecast i32 addrspace(1)* @global_as1 to i32*
  %global_as1_back = addrspacecast i32* %global_as1_as0 to i32 addrspace(1)*
  %as1_val = load i32, i32 addrspace(1)* %global_as1_back, align 4

  ; Load global_array[2]
  %array_ptr = getelementptr [4 x i32], [4 x i32]* @global_array, i32 0, i32 2
  %array_val = load i32, i32* %array_ptr, align 8

  ; Load global$struct.1
  %struct_ptr = getelementptr { i32, i32 }, { i32, i32 }* @global$struct, i32 0, i32 1
  %struct_int_val = load i32, i32* %struct_ptr

  ; Compute result
  %tmp_result = add i32 %int_val, %array_val
  %tmp_result2 = add i32 %tmp_result, %struct_int_val
  %result = add i32 %tmp_result2, %as1_val

  ret i32 %result
}