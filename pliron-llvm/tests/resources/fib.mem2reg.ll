; ModuleID = 'fib.ll'
source_filename = "fib.c"

; Function Attrs: noinline nounwind uwtable
define dso_local i32 @id(i32 noundef %0) #0 {
  ret i32 %0
}

; Function Attrs: noinline nounwind uwtable
define dso_local i32 @fib(i32 noundef %0) #0 {
  %2 = icmp sle i32 %0, 1
  br i1 %2, label %3, label %4

3:                                                ; preds = %1
  br label %16

4:                                                ; preds = %1
  %5 = icmp eq i32 %0, 2
  br i1 %5, label %6, label %7

6:                                                ; preds = %4
  br label %16

7:                                                ; preds = %4
  br label %8

8:                                                ; preds = %13, %7
  %.04 = phi i32 [ undef, %7 ], [ %11, %13 ]
  %.03 = phi i32 [ 1, %7 ], [ %12, %13 ]
  %.02 = phi i32 [ 0, %7 ], [ %.03, %13 ]
  %.01 = phi i32 [ 3, %7 ], [ %14, %13 ]
  %9 = icmp sle i32 %.01, %0
  br i1 %9, label %10, label %15

10:                                               ; preds = %8
  %11 = add nsw i32 %.03, %.02
  %12 = call i32 @id(i32 noundef %11)
  br label %13

13:                                               ; preds = %10
  %14 = add nsw i32 %.01, 1
  br label %8, !llvm.loop !6

15:                                               ; preds = %8
  br label %16

16:                                               ; preds = %15, %6, %3
  %.0 = phi i32 [ 0, %3 ], [ 1, %6 ], [ %.04, %15 ]
  ret i32 %.0
}

define dso_local i32 @main() #0 {
  %1 = alloca i32, align 4
  %2 = call i32 @fib(i32 6)
  store i32 %2, ptr %1, align 4
  %3 = load i32, ptr %1, align 4
  ret i32 %3
}

attributes #0 = { noinline nounwind uwtable "frame-pointer"="all" "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="x86-64" "target-features"="+cmov,+cx8,+fxsr,+mmx,+sse,+sse2,+x87" "tune-cpu"="generic" }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 1, !"wchar_size", i32 4}
!1 = !{i32 8, !"PIC Level", i32 2}
!2 = !{i32 7, !"PIE Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 2}
!4 = !{i32 7, !"frame-pointer", i32 2}
!5 = !{!"Ubuntu clang version 17.0.6 (++20231209124227+6009708b4367-1~exp1~20231209124336.77)"}
!6 = distinct !{!6, !7}
!7 = !{!"llvm.loop.mustprogress"}
