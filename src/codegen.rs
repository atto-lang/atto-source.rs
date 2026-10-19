//! Cranelift object backend: no LLVM/GraalVM, runtime or JIT dependencies.
use crate::{frontend::Expr, mir::Module};
use cranelift::{
    codegen::{ir::{types, AbiParam, Value}, settings},
    frontend::{FunctionBuilder, FunctionBuilderContext},
    module::{Linkage, Module as ClifModule},
    native,
    object::{ObjectBuilder, ObjectModule},
    prelude::InstBuilder,
};
pub fn emit_object(program: &Module) -> Result<Vec<u8>, String> {
    let isa = native::builder().map_err(|e| e.to_string())?
        .finish(settings::Flags::new(settings::builder()))
        .map_err(|e| e.to_string())?;
    let builder = ObjectBuilder::new(isa, "oreslang-module",
        cranelift::module::default_libcall_names()).map_err(|e| e.to_string())?;
    let mut module = ObjectModule::new(builder);
    for function in &program.functions {
        let mut ctx = module.make_context();
        ctx.func.signature.returns.push(AbiParam::new(types::I64));
        let symbol = module.declare_function(&function.symbol,
            Linkage::Export, &ctx.func.signature).map_err(|e| e.to_string())?;
        let mut fbctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
            let block = builder.create_block();
            builder.switch_to_block(block);
            builder.seal_block(block);
            let result = compile_expr(&mut builder, &function.return_value);
            builder.ins().return_(&[result]);
            builder.finalize();
        }
        module.define_function(symbol, &mut ctx).map_err(|e| e.to_string())?;
        module.clear_context(&mut ctx);
    }
    module.finish().emit().map_err(|e| e.to_string())
}
fn compile_expr(builder: &mut FunctionBuilder<'_>, expr: &Expr) -> Value {
    match expr {
        Expr::Integer(value) => builder.ins().iconst(types::I64, *value),
        Expr::Add(a,b) => {
            let a = compile_expr(builder, a);
            let b = compile_expr(builder, b);
            builder.ins().iadd(a,b)
        }
        Expr::Subtract(a,b) => {
            let a = compile_expr(builder, a);
            let b = compile_expr(builder, b);
            builder.ins().isub(a,b)
        }
        Expr::Multiply(a,b) => {
            let a = compile_expr(builder, a);
            let b = compile_expr(builder, b);
            builder.ins().imul(a,b)
        }
    }
}
