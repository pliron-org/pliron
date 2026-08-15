; Define a struct type
%MyStruct = type { i32, i64 }

; A packed named struct.
%Packed = type <{ i8, i32 }>

; A struct nested inside another struct, mixing packed and unpacked layouts.
%Nested = type { %MyStruct, %Packed, i16 }

; A recursive struct type, self-referential through a pointer field.
%List = type { i32, %List* }

; A struct that is never given a body, used only behind a pointer.
%Opaque = type opaque

; Only touches the opaque struct via a pointer, to exercise the
; never-defined named-struct type through argument passing.
define i32 @use_opaque(%Opaque* %p) {
  ret i32 7
}

; Define a function to demonstrate insertvalue and extractvalue
define i32 @main() {
entry:
  ; Create an instance of the struct and insert values
  %struct = alloca %MyStruct
  %struct_init = insertvalue %MyStruct undef, i32 42, 0
  %struct_final = insertvalue %MyStruct %struct_init, i64 13, 1
  store %MyStruct %struct_final, %MyStruct* %struct

  ; Extract values from the struct
  %loaded_struct = load %MyStruct, %MyStruct* %struct
  %extracted_i32 = extractvalue %MyStruct %loaded_struct, 0
  %extracted_i64 = extractvalue %MyStruct %loaded_struct, 1

  ; Perform a simple computation on the struct elements
  %struct_computation = add i32 %extracted_i32, 1

  ; Create an array and insert values
  %array = alloca [3 x i32]
  %array_init = insertvalue [3 x i32] undef, i32 10, 0
  %array_mid = insertvalue [3 x i32] %array_init, i32 20, 1
  %array_final = insertvalue [3 x i32] %array_mid, i32 30, 2
  store [3 x i32] %array_final, [3 x i32]* %array

  ; Extract values from the array
  %loaded_array = load [3 x i32], [3 x i32]* %array
  %extracted_elem0 = extractvalue [3 x i32] %loaded_array, 0
  %extracted_elem1 = extractvalue [3 x i32] %loaded_array, 1
  %extracted_elem2 = extractvalue [3 x i32] %loaded_array, 2

  ; Perform a simple computation on the array elements
  %array_computation = add i32 %extracted_elem0, %extracted_elem1
  %final_computation = add i32 %array_computation, %extracted_elem2

  ; Combine the results of the struct and array computations
  %result = add i32 %struct_computation, %final_computation

  ; Packed named struct.
  %packed = alloca %Packed
  %packed0 = insertvalue %Packed undef, i8 1, 0
  %packed1 = insertvalue %Packed %packed0, i32 2, 1
  store %Packed %packed1, %Packed* %packed
  %packed_loaded = load %Packed, %Packed* %packed
  %packed_field = extractvalue %Packed %packed_loaded, 1
  %result1 = add i32 %result, %packed_field

  ; Anonymous unpacked struct.
  %anonu = alloca { i8, i32 }
  %anonu0 = insertvalue { i8, i32 } undef, i8 1, 0
  %anonu1 = insertvalue { i8, i32 } %anonu0, i32 3, 1
  store { i8, i32 } %anonu1, { i8, i32 }* %anonu
  %anonu_loaded = load { i8, i32 }, { i8, i32 }* %anonu
  %anonu_field = extractvalue { i8, i32 } %anonu_loaded, 1
  %result2 = add i32 %result1, %anonu_field

  ; Anonymous packed struct.
  %anonp = alloca <{ i8, i32 }>
  %anonp0 = insertvalue <{ i8, i32 }> undef, i8 1, 0
  %anonp1 = insertvalue <{ i8, i32 }> %anonp0, i32 4, 1
  store <{ i8, i32 }> %anonp1, <{ i8, i32 }>* %anonp
  %anonp_loaded = load <{ i8, i32 }>, <{ i8, i32 }>* %anonp
  %anonp_field = extractvalue <{ i8, i32 }> %anonp_loaded, 1
  %result3 = add i32 %result2, %anonp_field

  ; Struct nested inside another struct.
  %nested = alloca %Nested
  %nested0 = insertvalue %Nested undef, %MyStruct %struct_final, 0
  %nested1 = insertvalue %Nested %nested0, %Packed %packed1, 1
  %nested2 = insertvalue %Nested %nested1, i16 5, 2
  store %Nested %nested2, %Nested* %nested
  %nested_loaded = load %Nested, %Nested* %nested
  %nested_field = extractvalue %Nested %nested_loaded, 2
  %nested_field_ext = sext i16 %nested_field to i32
  %result4 = add i32 %result3, %nested_field_ext

  ; Recursive struct type (self-referential through a pointer field).
  %list = alloca %List
  %list0 = insertvalue %List undef, i32 6, 0
  %list1 = insertvalue %List %list0, %List* null, 1
  store %List %list1, %List* %list
  %list_loaded = load %List, %List* %list
  %list_field = extractvalue %List %list_loaded, 0
  %result5 = add i32 %result4, %list_field

  ; Opaque struct, used only via a pointer, passed to a function.
  %opaque_call = call i32 @use_opaque(%Opaque* null)
  %result6 = add i32 %result5, %opaque_call

  ret i32 %result6
}
