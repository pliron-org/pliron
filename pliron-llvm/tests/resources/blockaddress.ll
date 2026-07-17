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

  ret i32 %sum3
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