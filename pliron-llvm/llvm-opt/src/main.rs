// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

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
    pass::{AnalysisManager, NestedOpsPass, OpPass, PMConfig, Pass, Passes},
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

    /// Print IR before every pass
    #[arg(long, default_value_t = false)]
    pm_print_before_all: bool,

    /// Print IR after every pass
    #[arg(long, default_value_t = false)]
    pm_print_after_all: bool,

    /// Print IR before these passes (comma-separated)
    ///
    /// Example: --pm-print-before mem2reg,dce
    #[arg(long, value_name = "PASS1,PASS2", value_delimiter = ',')]
    pm_print_before: Vec<String>,

    /// Print IR after these passes (comma-separated)
    ///
    /// Example: --pm-print-after mem2reg,dce
    #[arg(long, value_name = "PASS1,PASS2", value_delimiter = ',')]
    pm_print_after: Vec<String>,

    /// Verify IR before every pass
    #[arg(long, default_value_t = false)]
    pm_verify_before_all: bool,

    /// Verify IR after every pass
    #[arg(long, default_value_t = false)]
    pm_verify_after_all: bool,

    /// Verify IR before these passes (comma-separated)
    ///
    /// Example: --pm-verify-before mem2reg,dce
    #[arg(long, value_name = "PASS1,PASS2", value_delimiter = ',')]
    pm_verify_before: Vec<String>,

    /// Verify IR after these passes (comma-separated)
    ///
    /// Example: --pm-verify-after mem2reg,dce
    #[arg(long, value_name = "PASS1,PASS2", value_delimiter = ',')]
    pm_verify_after: Vec<String>,

    /// Time every pass
    #[arg(long, default_value_t = false)]
    pm_time_all_passes: bool,

    /// Time these passes (comma-separated)
    ///
    /// Example: --pm-time-passes mem2reg,dce
    #[arg(long, value_name = "PASS1,PASS2", value_delimiter = ',')]
    pm_time_passes: Vec<String>,

    /// Skip these passes (comma-separated)
    ///
    /// Example: --pm-skip-passes dce
    #[arg(long, value_name = "PASS1,PASS2", value_delimiter = ',')]
    pm_skip_passes: Vec<String>,
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

fn pm_config_from_cli(cli: &Cli) -> PMConfig {
    let mut pm_config = PMConfig {
        print_before_all: cli.pm_print_before_all,
        print_after_all: cli.pm_print_after_all,
        verify_before_all: cli.pm_verify_before_all,
        verify_after_all: cli.pm_verify_after_all,
        time_all_passes: cli.pm_time_all_passes,
        ..PMConfig::default()
    };

    for pass_name in &cli.pm_print_before {
        pm_config.print_before.insert(pass_name.clone());
    }

    for pass_name in &cli.pm_print_after {
        pm_config.print_after.insert(pass_name.clone());
    }

    for pass_name in &cli.pm_verify_before {
        pm_config.verify_before.insert(pass_name.clone());
    }

    for pass_name in &cli.pm_verify_after {
        pm_config.verify_after.insert(pass_name.clone());
    }

    for pass_name in &cli.pm_time_passes {
        pm_config.time_passes.insert(pass_name.clone());
    }

    for pass_name in &cli.pm_skip_passes {
        pm_config.skip_passes.insert(pass_name.clone());
    }

    pm_config
}

fn run_opt_passes(
    module: Ptr<Operation>,
    opts: &[OptPass],
    pm_config: PMConfig,
    ctx: &mut Context,
) -> Result<()> {
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

    let mut analyses = AnalysisManager::default();
    analyses.set_config(pm_config);

    passes.run(module, ctx, &mut analyses)?;

    Ok(())
}

fn run(cli: Cli, ctx: &mut Context) -> Result<()> {
    env_logger::init();

    let llvm_context = LLVMContext::default();
    let module = LLVMModule::from_ir_in_file(&llvm_context, cli.input.to_str().unwrap())
        .map_err(|err| arg_error_noloc!("{}", err))?;

    let pliron_module = from_llvm_ir::convert_module(ctx, &module)?;
    verify_op(&pliron_module, ctx).inspect_err(|_| {
        log::debug!(
            "Parsed pliron IR (verification failed):\n{}",
            pliron_module.disp(ctx)
        );
    })?;

    if let Some(opts) = cli.opts.as_ref() {
        run_opt_passes(
            pliron_module.get_operation(),
            opts,
            pm_config_from_cli(&cli),
            ctx,
        )?;
    }

    verify_op(&pliron_module, ctx).inspect_err(|_| {
        log::debug!(
            "pliron IR after optimizations (verification failed):\n{}",
            pliron_module.disp(ctx)
        );
    })?;

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
