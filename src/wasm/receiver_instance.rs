//! Experimental, instance-bound scalar receiver Wasm.
//!
//! A trusted host instantiates one module per actor and invokes exactly one
//! atto.bind.<Owner>(opaque_nonzero_i64) before any atto.receiver.* method.
//! Immutable actor owner, private capability, and terminal close phase are
//! Wasm globals, isolated per instance. The capability is not a security
//! boundary by itself: the host must retain the token and enforce tenant
//! authority. There is no actor state, mailbox, lease or scheduler here.
use super::*;
use crate::diag::{Diagnostic, Pos};
use crate::target_scalar::ScalarMethod;
use crate::target_scalar_cfg::ScalarOp;

pub(super) const PANIC_INVALID_RECEIVER: i32 = 6;

fn export_function(out: &mut Vec<u8>, name_str: &str, index: usize) {
    name(out, name_str);
    out.push(0); // function export
    uleb(out, index as u64);
}

fn emit_checked_scalar(method: &ScalarMethod, owner_index: usize) -> Result<Vec<u8>, Diagnostic> {
    let cfg=crate::target_scalar_cfg::lower_scalar_cfg(&method.expr,method.arity,method.pos)?;
    let mut code=vec![0]; // 0 local declarations; local 0 = opaque actor token
    // Admission checks MUST run before any arithmetic or user instruction.
    code.extend_from_slice(&[0x23,3,0x41,1,0x47,0x04,0x40]); // phase != ACTIVE
    panic(&mut code,PANIC_INVALID_RECEIVER);
    code.push(0x0b);
    code.extend_from_slice(&[0x23,2,0x41]); // owner != compiled method owner
    sleb(&mut code,owner_index as i64);
    code.extend_from_slice(&[0x47,0x04,0x40]);
    panic(&mut code,PANIC_INVALID_RECEIVER);
    code.push(0x0b);
    code.extend_from_slice(&[0x20,0,0x23,1,0x52,0x04,0x40]); // caller token != bound token
    panic(&mut code,PANIC_INVALID_RECEIVER);
    code.push(0x0b);
    for node in cfg.nodes() {
        match node.op {
            ScalarOp::Param(index) => {code.push(0x20);uleb(&mut code,u64::from(index)+1);}
            ScalarOp::Const(value) => {code.push(0x42);sleb(&mut code,value);}
            ScalarOp::CheckedNeg => {code.push(0x10);uleb(&mut code,H_NEG as u64);}
            ScalarOp::CheckedAdd => {code.push(0x10);uleb(&mut code,H_ADD as u64);}
            ScalarOp::CheckedSub => {code.push(0x10);uleb(&mut code,H_SUB as u64);}
            ScalarOp::CheckedMul => {code.push(0x10);uleb(&mut code,H_MUL as u64);}
            ScalarOp::Return => code.push(0x0b),
            ScalarOp::OverflowPanic => {} // checked helpers set panic_code then unreachable
        }
    }
    Ok(code)
}

fn emit_bind(owner_index: usize) -> Vec<u8> {
    // (i64 token) -> i32. Exactly one successful binding in module lifetime.
    // Phase 0 -> 1, owner locked, token nonzero. Terminal phase 2 cannot rebind.
    let mut code=vec![
        0,0x23,3,0x45, // unbound phase == 0
        0x20,0,0x50,0x45,0x71, // token != 0 && phase == 0
        0x04,I32, // if (result i32)
        0x20,0,0x24,1, // capability = token
        0x41
    ];
    sleb(&mut code,owner_index as i64);
    code.extend_from_slice(&[
        0x24,2, // owner = owner_index
        0x41,1,0x24,3, // phase = ACTIVE
        0x41,1, // accepted
        0x05,0x41,0, // else rejected
        0x0b,0x0b // endif, function end
    ]);
    code
}

fn emit_close() -> Vec<u8> {
    // (i64 token) -> i32. Terminal close drops capability; no reuse of instance.
    vec![
        0,0x23,3,0x41,1,0x46, // phase == ACTIVE
        0x20,0,0x23,1,0x51,0x71, // token == bound token
        0x04,I32,
        0x42,0,0x24,1, // erase capability
        0x41,2,0x24,3, // phase = CLOSED
        0x41,1,
        0x05,0x41,0,
        0x0b,0x0b,
    ]
}

/// Emit an independently instantiable actor-instance module. Each instance
/// holds private type/token/lifecycle globals, with byte-identical code from
/// native and browser Rust compiler builds. No dynamic imports or memory.
pub(super) fn emit(methods: &[ScalarMethod]) -> Result<Vec<u8>, Diagnostic> {
    if methods.is_empty() || methods.len()>32 {
        return Err(Diagnostic::new(Pos::default(),"E_UNSUPPORTED",
            "receiver instance preview requires one to 32 scalar methods"));
    }
    // Owner IDs are deterministic within this module. They are not globally
    // stable actor type identities or external capability registry keys.
    let mut owners:Vec<&str>=Vec::new();
    for m in methods {
        if !owners.contains(&m.owner.as_str()) {
            owners.push(&m.owner);
        }
    }

    let mut types=Types(Vec::new());
    let mut function_types=Vec::new();
    let mut bodies=Vec::new();
    for (sig,locals,code) in helper_bodies() {
        function_types.push(types.idx(sig));
        let mut body=locals;
        body.extend(code);
        bodies.push(body);
    }
    let bind_start=bodies.len();
    for i in 0..owners.len() {
        function_types.push(types.idx(Sig(vec![I64],vec![I32])));
        bodies.push({
            let code=emit_bind(i+1);
            let mut size=Vec::new();
            size.extend(code);
            size
        });
    }
    let close_index=bodies.len();
    function_types.push(types.idx(Sig(vec![I64],vec![I32])));
    bodies.push(emit_close());
    let method_start=bodies.len();
    for method in methods {
        let owner=owners.iter().position(|o|*o==method.owner)
            .ok_or_else(|| Diagnostic::new(method.pos,"E_UNSUPPORTED",
                "scalar receiver has no nominal actor owner"))?;
        let mut params=vec![I64]; // trusted host capability, not a source parameter
        params.extend(std::iter::repeat_n(I64,method.arity));
        function_types.push(types.idx(Sig(params,vec![I64])));
        bodies.push(emit_checked_scalar(method,owner+1)?);
    }

    let mut result=b"\0asm\x01\0\0\0".to_vec();
    let mut type_section=Vec::new();
    uleb(&mut type_section,types.0.len() as u64);
    for Sig(params,returns) in &types.0 {
        type_section.push(0x60);
        uleb(&mut type_section,params.len() as u64);
        type_section.extend(params);
        uleb(&mut type_section,returns.len() as u64);
        type_section.extend(returns);
    }
    section(&mut result,1,type_section);
    let mut func_section=Vec::new();
    uleb(&mut func_section,function_types.len() as u64);
    for idx in &function_types {uleb(&mut func_section,*idx as u64);}
    section(&mut result,3,func_section);

    // Private mutable globals in each WebAssembly.Instance.
    // 0 panic_code:i32; 1 capability:i64; 2 nominal owner:i32; 3 phase:i32
    let mut globals=vec![4];
    globals.extend_from_slice(&[I32,1,0x41,0,0x0b]);
    globals.extend_from_slice(&[I64,1,0x42,0,0x0b]);
    globals.extend_from_slice(&[I32,1,0x41,0,0x0b]);
    globals.extend_from_slice(&[I32,1,0x41,0,0x0b]);
    section(&mut result,6,globals);

    let mut exports=Vec::new();
    uleb(&mut exports,(owners.len()+methods.len()+2) as u64);
    name(&mut exports,"panic_code");
    exports.extend_from_slice(&[3,0]); // global 0
    for (i,owner) in owners.iter().enumerate() {
        export_function(&mut exports,&format!("atto.bind.{owner}"),bind_start+i);
    }
    export_function(&mut exports,"atto.actor_close",close_index);
    for (i,method) in methods.iter().enumerate() {
        export_function(&mut exports,&format!("atto.receiver.{}.{}.{}",
            method.owner,method.name,method.arity),method_start+i);
    }
    section(&mut result,7,exports);

    let mut code_sec=Vec::new();
    uleb(&mut code_sec,bodies.len() as u64);
    for b in bodies {
        uleb(&mut code_sec,b.len() as u64);
        code_sec.extend(b);
    }
    section(&mut result,10,code_sec);
    Ok(result)
}
