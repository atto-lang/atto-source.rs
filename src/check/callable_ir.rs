//! Typed callable signatures and dispatch intent (pre-first-class ABI).
//!
//! Only immutable, noncapturing, directly invoked scalar callbacks have
//! executable Wasm lowering. Indirect tables and captured environments must
//! remain gated until fully implemented and tested.

use crate::ast::{Expr, Type};
use crate::diag::{Diagnostic, Pos};
use super::lambda_types::check_noncapturing;
type R<T> = Result<T, Diagnostic>;

/// A bounded scalar callback contract: no void parameters, but void results
/// are permitted. Object, actor and generator transports are not yet admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallableSignature {
    pub params: Vec<Type>,
    pub result: Type,
}
impl CallableSignature {
    pub fn scalar(params: Vec<Type>, result: Type, pos: Pos) -> R<Self> {
        if params.len() > crate::parser::MAX_PARAMS {
            return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT",
                "callable signature exceeds parameter limit"));
        }
        if params.iter().any(|t| !matches!(t,
            Type::Int | Type::Bool | Type::Float | Type::Str))
            || !matches!(result,
                Type::Int | Type::Bool | Type::Float | Type::Str | Type::Void) {
            return Err(Diagnostic::new(pos, "E_UNSUPPORTED_TYPE",
                "callable signature is outside the scalar noncapturing profile"));
        }
        Ok(Self { params, result })
    }

    /// Contextually infer the callback signature and verify the lambda AST.
    /// This does not lower callable values, table handles or guest Wasm.
    pub fn from_noncapturing(
        lambda: &Expr, expected_params: Option<&[Type]>,
        expected_result: Option<Type>,
    ) -> R<Self> {
        let checked = check_noncapturing(lambda, expected_params, expected_result)?;
        Self::scalar(checked.params.into_iter().map(|(_, ty)| ty).collect(),
            checked.result, lambda.pos)
    }

    pub fn check_arguments(&self, actual: &[Type], pos: Pos) -> R<Type> {
        if actual.len() != self.params.len() {
            return Err(Diagnostic::new(pos, "E_ARITY", format!(
                "callable expects {} arguments, found {}",
                self.params.len(), actual.len())));
        }
        for (index, (want, got)) in self.params.iter().zip(actual).enumerate() {
            if want != got {
                return Err(Diagnostic::new(pos, "E_TYPE", format!(
                    "callable argument {} expects {} but got {}",
                    index + 1, want.name(), got.name())));
            }
        }
        Ok(self.result)
    }
}

/// Only Direct is currently lowered. Indirect is a reserved future intent
/// requiring checked per-instance Wasm table ownership and generation fences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallableTarget {
    Direct { function_index: u32 },
    Indirect { table_index: u32, generation: u32, type_index: u32 },
}

/// Fields are private to prevent unverified indirect target construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedCallable {
    signature: CallableSignature,
    target: CallableTarget,
}
impl TypedCallable {
    pub fn checked(signature: CallableSignature, target: CallableTarget, pos: Pos) -> R<Self> {
        match target {
            CallableTarget::Direct { function_index }
                if function_index < crate::parser::MAX_FUNCS as u32 =>
                Ok(Self { signature, target }),
            CallableTarget::Direct { .. } =>
                Err(Diagnostic::new(pos, "E_SOURCE_LIMIT",
                    "direct callable function index exceeds module limit")),
            CallableTarget::Indirect { .. } =>
                Err(Diagnostic::new(pos, "E_UNSUPPORTED",
                    "indirect callable table and generation validation are not implemented")),
        }
    }
    pub fn signature(&self) -> &CallableSignature { &self.signature }
    pub fn target(&self) -> CallableTarget { self.target }
    pub fn check_arguments(&self, args: &[Type], pos: Pos) -> R<Type> {
        self.signature.check_arguments(args, pos)
    }
}
