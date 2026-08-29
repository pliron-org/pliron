; ModuleID = 'simple-loop.ll'
source_filename = "simple-loop.c"

; Function Attrs: noinline nounwind optnone uwtable
define dso_local i32 @main() #0 {
bb:
  %alloca = alloca i32, align 4
  %alloca1 = alloca [5 x i32], align 16
  %alloca2 = alloca i32, align 4
  %alloca3 = alloca i32, align 4
  %alloca4 = alloca i32, align 4
  store i32 0, ptr %alloca, align 4
  store i32 0, ptr %alloca2, align 4
  store i32 0, ptr %alloca3, align 4
  br label %bb5

bb5:                                              ; preds = %bb9, %bb
  %load = load i32, ptr %alloca3, align 4
  %icmp = icmp slt i32 %load, 5
  br i1 %icmp, label %bb6, label %bb12

bb6:                                              ; preds = %bb5
  %load7 = load i32, ptr %alloca3, align 4
  %add = add nsw i32 %load7, 1
  %load8 = load i32, ptr %alloca3, align 4
  %sext = sext i32 %load8 to i64
  %getelementptr = getelementptr inbounds [5 x i32], ptr %alloca1, i64 0, i64 %sext
  store i32 %add, ptr %getelementptr, align 4
  br label %bb9

bb9:                                              ; preds = %bb6
  %load10 = load i32, ptr %alloca3, align 4
  %add11 = add nsw i32 %load10, 1
  store i32 %add11, ptr %alloca3, align 4
  br label %bb5, !llvm.loop !6

bb12:                                             ; preds = %bb5
  store i32 0, ptr %alloca4, align 4
  br label %bb13

bb13:                                             ; preds = %bb23, %bb12
  %load14 = load i32, ptr %alloca4, align 4
  %icmp15 = icmp slt i32 %load14, 5
  br i1 %icmp15, label %bb16, label %bb26

bb16:                                             ; preds = %bb13
  %load17 = load i32, ptr %alloca4, align 4
  %sext18 = sext i32 %load17 to i64
  %getelementptr19 = getelementptr inbounds [5 x i32], ptr %alloca1, i64 0, i64 %sext18
  %load20 = load i32, ptr %getelementptr19, align 4
  %load21 = load i32, ptr %alloca2, align 4
  %add22 = add nsw i32 %load21, %load20
  store i32 %add22, ptr %alloca2, align 4
  br label %bb23

bb23:                                             ; preds = %bb16
  %load24 = load i32, ptr %alloca4, align 4
  %add25 = add nsw i32 %load24, 1
  store i32 %add25, ptr %alloca4, align 4
  br label %bb13, !llvm.loop !8

bb26:                                             ; preds = %bb13
  %load27 = load i32, ptr %alloca2, align 4
  ret i32 %load27
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
!8 = distinct !{!8, !7}
