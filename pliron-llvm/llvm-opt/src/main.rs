use std::{path::PathBuf, process::ExitCode, str::FromStr};

use clap::Parser;
use pliron::{
    arg_error_noloc,
    builtin::ops::ModuleOp,
    context::{Context, Ptr},
    op::{Op, verify_op},
    operation::Operation,
    opts::{
        constants::sccp::SCCPPass, dce::DCEPass, mem2reg::Mem2RegPass,
        simplify_cfg::SimplifyCFGPass,
    },
    pass::{AnalysisManager, NestedOpsPass, OpPass, Pass, Passes},
    printable::Printable,
    result::Result,
    verify_error_noloc,
};
use pliron_llvm::{
    from_llvm_ir,
    llvm_sys::core::{LLVMContext, LLVMModule},
    ops::FuncOp,
    to_llvm_ir,
};

#[derive(Parser)]
#[command(version, about="LLVM Optimizer", long_about = None)]
struct Cli {
    /// Input LLVM-IR (Assembly / Bitcode) file
    #[arg(short, value_name = "FILE")]
    input: PathBuf,

    /// Output LLVM file
    #[arg(short, value_name = "FILE")]
    output: PathBuf,

    /// Emit text assembly LLVM-IR
    #[arg(short = 'S', default_value_t = false)]
    text_output: bool,

    /// Optimization passes to run in order (comma-separated)
    ///
    /// Example: --opts mem2reg,dce,o1
    #[arg(long = "opts", value_name = "PASS1,PASS2", value_delimiter = ',')]
    opts: Option<Vec<OptPass>>,
}

#[derive(Clone, Copy, Debug)]
enum OptPass {
    Mem2Reg,
    Dce,
    Sccp,
    SimplifyCfg,
    O1,
}

impl FromStr for OptPass {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mem2reg" => Ok(OptPass::Mem2Reg),
            "dce" => Ok(OptPass::Dce),
            "sccp" => Ok(OptPass::Sccp),
            "simplify-cfg" => Ok(OptPass::SimplifyCfg),
            "o1" => Ok(OptPass::O1),
            other => Err(format!(
                "unknown optimization pass '{other}'. Available passes: mem2reg, dce, sccp, simplify-cfg, o1"
            )),
        }
    }
}

fn run_opt_passes(module: Ptr<Operation>, opts: &[OptPass], ctx: &mut Context) -> Result<()> {
    let mut passes = OpPass::<ModuleOp, Passes>::default();

    for opt in opts {
        match opt {
            OptPass::Mem2Reg => {
                let mem2reg_pass = OpPass::<FuncOp, Mem2RegPass>::default();
                passes.add_pass(NestedOpsPass::new(mem2reg_pass));
            }
            OptPass::Dce => {
                let dce_pass = OpPass::<FuncOp, DCEPass>::default();
                passes.add_pass(NestedOpsPass::new(dce_pass));
            }
            OptPass::Sccp => {
                let sccp_pass = OpPass::<FuncOp, SCCPPass>::default();
                passes.add_pass(NestedOpsPass::new(sccp_pass));
            }
            OptPass::SimplifyCfg => {
                let simplify_cfg_pass = OpPass::<FuncOp, SimplifyCFGPass>::default();
                passes.add_pass(NestedOpsPass::new(simplify_cfg_pass));
            }
            OptPass::O1 => {
                pliron_llvm::append_o1_passes(&mut passes);
            }
        }
    }

    passes.run(module, ctx, &mut AnalysisManager::default())?;

    Ok(())
}

fn run(cli: Cli, ctx: &mut Context) -> Result<()> {
    env_logger::init();

    let llvm_context = LLVMContext::default();
    let module = LLVMModule::from_ir_in_file(&llvm_context, cli.input.to_str().unwrap())
        .map_err(|err| arg_error_noloc!("{}", err))?;

    let pliron_module = from_llvm_ir::convert_module(ctx, &module)?;
    log::debug!(
        "pliron IR parsed from LLVM-IR:\n{}",
        pliron_module.disp(ctx)
    );
    verify_op(&pliron_module, ctx)?;

    if let Some(opts) = cli.opts.as_ref() {
        run_opt_passes(pliron_module.get_operation(), opts, ctx)?;
    }

    log::debug!(
        "pliron IR after optimizations:\n{}",
        pliron_module.disp(ctx)
    );
    verify_op(&pliron_module, ctx)?;

    let module = to_llvm_ir::convert_module(ctx, &llvm_context, pliron_module)?;
    module
        .verify()
        .map_err(|err| verify_error_noloc!("{}", err.to_string()))?;

    if cli.text_output {
        module
            .asm_to_file(cli.output.to_str().unwrap())
            .map_err(|err| arg_error_noloc!("{}", err.to_string()))?;
    } else {
        module
            .bitcode_to_file(cli.output.to_str().unwrap())
            .map_err(|_err| arg_error_noloc!("{}", "Error writing bitcode to file"))?;
    }
    Ok(())
}

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let ctx = &mut Context::default();

    match run(cli, ctx) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e.disp(ctx));
            ExitCode::FAILURE
        }
    }
}
