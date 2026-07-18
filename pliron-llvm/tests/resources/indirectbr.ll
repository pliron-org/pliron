; indirectbr.ll - Test indirectbr (computed goto) with block arguments.

define i32 @dispatch(i32 %sel, i32 %base) {
entry:
  %c0 = icmp eq i32 %sel, 0
  %tA = select i1 %c0, ptr blockaddress(@dispatch, %case0), ptr blockaddress(@dispatch, %case2)
  %c1 = icmp eq i32 %sel, 1
  %target = select i1 %c1, ptr blockaddress(@dispatch, %case1), ptr %tA
  indirectbr ptr %target, [label %case0, label %case1, label %case2]

case0:
  %v0 = phi i32 [ %base, %entry ]
  %r0 = add i32 %v0, 10
  ret i32 %r0

case1:
  %v1 = phi i32 [ %base, %entry ]
  %r1 = add i32 %v1, 20
  ret i32 %r1

case2:
  %v2 = phi i32 [ %base, %entry ]
  %r2 = add i32 %v2, 30
  ret i32 %r2
}

define i32 @main() {
entry:
  %r0 = call i32 @dispatch(i32 0, i32 1)
  %r1 = call i32 @dispatch(i32 1, i32 2)
  %r2 = call i32 @dispatch(i32 5, i32 3)
  %s0 = add i32 %r0, %r1
  %s1 = add i32 %s0, %r2
  ret i32 %s1
}
