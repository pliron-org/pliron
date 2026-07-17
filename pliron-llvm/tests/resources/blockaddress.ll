@block_table = constant [2 x ptr] [ptr blockaddress(@block_targets_1, %taken), ptr blockaddress(@block_targets_1, %other)]

define void @block_targets_1(i1 %cond) {
entry:
  br i1 %cond, label %taken, label %other

taken:
  ret void

other:
  ret void
}

define i32 @main() {
entry:
  %use1 = icmp ne ptr blockaddress(@block_targets_1, %taken), null
  %use2 = icmp ne ptr blockaddress(@block_targets_1, %other), null
  %use3 = icmp ne ptr blockaddress(@block_targets_2, %other), null

  %use1_i32 = zext i1 %use1 to i32
  %use2_i32 = zext i1 %use2 to i32
  %use3_i32 = zext i1 %use3 to i32
  %sum = add i32 %use1_i32, %use2_i32
  %sum2 = add i32 %sum, %use3_i32

  %foo_val = call i32 @foo()
  %sum3 = add i32 %sum2, %foo_val

  %table_ptr0 = getelementptr [2 x ptr], ptr @block_table, i32 0, i32 0
  %table_val0 = load ptr, ptr %table_ptr0
  %use4 = icmp ne ptr %table_val0, null
  %use4_i32 = zext i1 %use4 to i32
  %sum4 = add i32 %sum3, %use4_i32

  ret i32 %sum4
}

define void @block_targets_2(i1 %cond) {
entry:
  br i1 %cond, label %taken, label %other

taken:
  ret void

other:
  ret void
}

define i32 @foo() {
entry:
  %use1 = icmp ne ptr blockaddress(@block_targets_2, %taken), null
  %use2 = icmp ne ptr blockaddress(@block_targets_2, %other), null
  %use3 = icmp ne ptr blockaddress(@block_targets_1, %taken), null

  %use1_i32 = zext i1 %use1 to i32
  %use2_i32 = zext i1 %use2 to i32
  %use3_i32 = zext i1 %use3 to i32

  %sum = add i32 %use1_i32, %use2_i32
  %sum2 = add i32 %sum, %use3_i32

  ret i32 %sum2
}