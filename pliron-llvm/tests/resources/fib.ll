; ModuleID = 'fib.c'
source_filename = "fib.c"

declare i64 @declared_function(i32)

; Function Attrs: noinline nounwind optnone uwtable
define dso_local i32 @fib$1(i32 noundef %0) #0 {
  %2 = alloca i32, align 4
  %3 = alloca i32, align 8
  %4 = alloca i32, align 4
  %5 = alloca i32, align 4
  %6 = alloca i32, align 4
  %7 = alloca i32, align 4
  store i32 %0, ptr %3, align 8
  %8 = load i32, ptr %3, align 8
  %9 = icmp sle i32 %8, 1
  br i1 %9, label %10, label %11

10:                                               ; preds = %1
  store i32 0, ptr %2, align 4
  br label %31

11:                                               ; preds = %1
  %12 = load i32, ptr %3, align 4
  %13 = icmp eq i32 %12, 2
  br i1 %13, label %14, label %15

14:                                               ; preds = %11
  store i32 1, ptr %2, align 4
  br label %31

15:                                               ; preds = %11
  store i32 0, ptr %4, align 4
  store i32 1, ptr %5, align 4
  store i32 3, ptr %7, align 4
  br label %16

16:                                               ; preds = %26, %15
  %17 = load i32, ptr %7, align 4
  %18 = load i32, ptr %3, align 4
  %19 = icmp sle i32 %17, %18
  br i1 %19, label %20, label %29

20:                                               ; preds = %16
  %21 = load i32, ptr %5, align 4
  %22 = load i32, ptr %4, align 4
  %23 = add nsw i32 %21, %22
  store i32 %23, ptr %6, align 4
  %24 = load i32, ptr %5, align 4
  store i32 %24, ptr %4, align 4
  %25 = load i32, ptr %6, align 4
  store i32 %25, ptr %5, align 4
  br label %26

26:                                               ; preds = %20
  %27 = load i32, ptr %7, align 4
  %28 = add nsw i32 %27, 1
  store i32 %28, ptr %7, align 4
  br label %16, !llvm.loop !6

29:                                               ; preds = %16
  %30 = load i32, ptr %6, align 4
  store i32 %30, ptr %2, align 4
  br label %31

31:                                               ; preds = %29, %14, %10
  %32 = load i32, ptr %2, align 4
  ret i32 %32
}

define dso_local i32 @main() #0 {
  %1 = alloca i32, align 4
  %2 = call i32 @fib$1(i32 5)
  store i32 %2, ptr %1, align 4
  %3 = load i32, ptr %1, align 4
  ret i32 %3
}

attributes #0 = { noinline nounwind optnone uwtable "frame-pointer"="all" "min-legal-vector-width"="0" "no-trapping-math"="true" "stack-protector-buffer-size"="8" "target-cpu"="x86-64" "target-features"="+cmov,+cx8,+fxsr,+mmx,+sse,+sse2,+x87" "tune-cpu"="generic" }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 1, !"wchar_size", i32 4}
!1 = !{i32 8, !"PIC Level", i32 2}
!2 = !{i32 7, !"PIE Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 2}
!4 = !{i32 7, !"frame-pointer", i32 2}
!5 = !{!"Ubuntu clang version 18.1.3 (1ubuntu1)"}
!6 = distinct !{!6, !7}
!7 = !{!"llvm.loop.mustprogress"}
