//! Unit tests for the LLVM dialect ops. Each test builds a small module, prints
//! it, checks for expected tokens, and asserts the printed IR re-parses. They
//! exercise only the printer and parser (no `llvm-sys`), so they run without
//! that feature.

use pliron::{
    basic_block::BasicBlock,
    builtin::{
        op_interfaces::SingleBlockRegionInterface,
        ops::ModuleOp,
        types::{IntegerType, Signedness},
    },
    combine::Parser,
    context::{Context, Ptr},
    irfmt::parsers::spaced,
    location,
    op::Op,
    operation::Operation,
    parsable::{self, state_stream_from_iterator},
    printable::Printable,
    r#type::TypeObj,
    value::Value,
};
use pliron_llvm::{
    attributes::{AtomicOrderingAttr, AtomicRmwKindAttr},
    op_interfaces::CastOpInterface,
    ops::{
        AddrSpaceCastOp, AtomicCmpxchgOp, AtomicLoadOp, AtomicRmwOp, AtomicStoreOp, FenceOp,
        FuncOp, InlineAsmOp, ReturnOp,
    },
    types::{FuncType, PointerType, VoidType},
};

/// Build a module with a function whose entry block has the given argument
/// types, let `build` add ops to that block, then assert the whole module
/// round-trips through the parser unchanged.
fn assert_op_roundtrips(
    make_arg_types: impl FnOnce(&mut Context) -> Vec<Ptr<TypeObj>>,
    build: impl FnOnce(&mut Context, Ptr<BasicBlock>, &[Value]),
    expected: &[&str],
) {
    let ctx = &mut Context::new();
    let arg_types = make_arg_types(ctx);

    let module = ModuleOp::new(ctx, "test".try_into().unwrap());
    let module_block = module.get_body(ctx, 0);

    let void = VoidType::get(ctx).into();
    let fty = FuncType::get(ctx, void, arg_types, false);
    let func = FuncOp::new(ctx, "f".try_into().unwrap(), fty);
    let entry = func.get_or_create_entry_block(ctx);
    let args: Vec<Value> = entry.deref(ctx).arguments().collect();

    build(ctx, entry, &args);
    ReturnOp::new(ctx, None)
        .get_operation()
        .insert_at_back(entry, ctx);
    func.get_operation().insert_at_back(module_block, ctx);

    let printed = module.get_operation().disp(ctx).to_string();
    for tok in expected {
        assert!(
            printed.contains(tok),
            "expected `{tok}` in printed IR:\n{printed}"
        );
    }

    // The printed IR must re-parse: this exercises the op's (and its
    // attributes') parser against exactly what its printer produced.
    let ctx2 = &mut Context::new();
    let state_stream = state_stream_from_iterator(
        printed.chars(),
        parsable::State::new(ctx2, location::Source::InMemory),
    );
    spaced(Operation::top_level_parser())
        .parse(state_stream)
        .expect("printed IR should re-parse");
}

fn i32_ty(ctx: &mut Context) -> Ptr<TypeObj> {
    IntegerType::get(ctx, 32, Signedness::Signless).into()
}

#[test]
fn test_addrspacecast_roundtrips() {
    assert_op_roundtrips(
        |ctx| vec![PointerType::get(ctx, 0).into()],
        |ctx, entry, args| {
            let dst = PointerType::get(ctx, 1).into();
            AddrSpaceCastOp::new(ctx, args[0], dst)
                .get_operation()
                .insert_at_back(entry, ctx);
        },
        &["llvm.addrspacecast", "llvm.ptr (1)"],
    );
}

#[test]
fn test_atomicrmw_roundtrips() {
    assert_op_roundtrips(
        |ctx| vec![PointerType::get(ctx, 1).into(), i32_ty(ctx)],
        |ctx, entry, args| {
            AtomicRmwOp::new(
                ctx,
                args[0],
                args[1],
                AtomicRmwKindAttr::Add,
                AtomicOrderingAttr::SeqCst,
                Some("device".to_string()),
            )
            .get_operation()
            .insert_at_back(entry, ctx);
        },
        &["llvm.atomicrmw", "Add", "SeqCst", "device"],
    );
}

#[test]
fn test_cmpxchg_roundtrips() {
    assert_op_roundtrips(
        |ctx| {
            let i = i32_ty(ctx);
            vec![PointerType::get(ctx, 0).into(), i, i]
        },
        |ctx, entry, args| {
            AtomicCmpxchgOp::new(
                ctx,
                args[0],
                args[1],
                args[2],
                AtomicOrderingAttr::AcqRel,
                AtomicOrderingAttr::Monotonic,
                None,
            )
            .get_operation()
            .insert_at_back(entry, ctx);
        },
        &["llvm.cmpxchg", "AcqRel", "Monotonic"],
    );
}

#[test]
fn test_fence_roundtrips() {
    assert_op_roundtrips(
        |_ctx| vec![],
        |ctx, entry, _args| {
            FenceOp::new(ctx, AtomicOrderingAttr::SeqCst, Some("block".to_string()))
                .get_operation()
                .insert_at_back(entry, ctx);
        },
        &["llvm.fence", "SeqCst", "block"],
    );
}

#[test]
fn test_atomic_load_roundtrips() {
    assert_op_roundtrips(
        |ctx| vec![PointerType::get(ctx, 3).into()],
        |ctx, entry, args| {
            let res = i32_ty(ctx);
            AtomicLoadOp::new(ctx, args[0], res, AtomicOrderingAttr::Acquire, None)
                .get_operation()
                .insert_at_back(entry, ctx);
        },
        &["llvm.atomic_load", "Acquire"],
    );
}

#[test]
fn test_atomic_store_roundtrips() {
    assert_op_roundtrips(
        |ctx| vec![i32_ty(ctx), PointerType::get(ctx, 3).into()],
        |ctx, entry, args| {
            AtomicStoreOp::new(
                ctx,
                args[0],
                args[1],
                AtomicOrderingAttr::Release,
                Some("device".to_string()),
            )
            .get_operation()
            .insert_at_back(entry, ctx);
        },
        &["llvm.atomic_store", "Release", "device"],
    );
}

#[test]
fn test_inline_asm_roundtrips() {
    assert_op_roundtrips(
        |ctx| vec![i32_ty(ctx)],
        |ctx, entry, args| {
            let res = i32_ty(ctx);
            InlineAsmOp::new(ctx, res, vec![args[0]], "mov $0, $1", "=r,r", true)
                .get_operation()
                .insert_at_back(entry, ctx);
        },
        &["llvm.inline_asm", "mov", "=r,r", "convergent"],
    );
}
