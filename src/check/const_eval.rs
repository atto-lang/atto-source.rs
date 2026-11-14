//! Deterministic typed evaluation for the admitted local const subset.
//!
//! Runs *after* type checking, so numeric promotion and operator validity use
//! precisely the same typed IR as guest execution. Fail closed on calls,
//! allocations, actor operations, mutable bindings, and unsupported effects.
//! Values are materialized back into typed literals: guest runtime does not
//! re-evaluate a constant expression.
use super::*;

#[derive(Clone, Debug)]
pub(super) enum ConstValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Enum(u32),
}

fn not_const(pos: Pos) -> Diagnostic {
    Diagnostic::new(pos, "E_CONST", "initializer is not a supported compile-time value; use 'val' for a runtime immutable binding")
}
fn overflow(pos: Pos) -> Diagnostic {
    Diagnostic::new(pos, "E_CONST_OVERFLOW", "checked integer arithmetic overflows during compile-time evaluation")
}

impl<'a> FnCx<'a> {
    pub(super) fn eval_const(&self, v: &TE, pos: Pos) -> R<ConstValue> {
        use ConstValue::*;
        match &v.e {
            TExpr::I64(x) => Ok(Int(*x)),
            TExpr::F64(x) => Ok(Float(*x)),
            TExpr::Bool(x) => Ok(Bool(*x)),
            TExpr::EnumVal(x) => Ok(Enum(*x)),
            TExpr::StrConst(off, len) => {
                let start = *off as usize;
                let end = start.checked_add(*len as usize).ok_or_else(|| not_const(pos))?;
                let bytes = self.data.get(start..end).ok_or_else(|| not_const(pos))?;
                let value = std::str::from_utf8(bytes).map_err(|_| not_const(pos))?;
                Ok(Str(value.to_string()))
            }
            TExpr::Local(slot) => self.const_values.get(slot).cloned().ok_or_else(|| not_const(pos)),
            TExpr::Neg(x) => match self.eval_const(x, pos)? {
                Int(a) => a.checked_neg().map(Int).ok_or_else(|| overflow(pos)),
                _ => Err(not_const(pos)),
            },
            TExpr::FNeg(x) => match self.eval_const(x, pos)? {
                Float(a) => Ok(Float(-a)),
                _ => Err(not_const(pos)),
            },
            TExpr::Not(x) => match self.eval_const(x, pos)? {
                Bool(a) => Ok(Bool(!a)),
                _ => Err(not_const(pos)),
            },
            TExpr::IntToFloat(x) => match self.eval_const(x, pos)? {
                Int(a) => Ok(Float(a as f64)),
                _ => Err(not_const(pos)),
            },
            TExpr::Ternary(cond, yes, no) => match self.eval_const(cond, pos)? {
                Bool(true) => self.eval_const(yes, pos),
                Bool(false) => self.eval_const(no, pos),
                _ => Err(not_const(pos)),
            },
            TExpr::Str(op, lhs, rhs) => {
                let a = match self.eval_const(lhs, pos)? { Str(v) => v, _ => return Err(not_const(pos)) };
                let b = match rhs {
                    Some(v) => match self.eval_const(v, pos)? { Str(v) => v, _ => return Err(not_const(pos)) },
                    None => return Err(not_const(pos)),
                };
                match op {
                    StrOp::Eq => Ok(Bool(a == b)),
                    StrOp::Cmp => Ok(Int(match a.encode_utf16().cmp(b.encode_utf16()) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    })),
                    StrOp::Concat => {
                        if a.len().checked_add(b.len()).is_none_or(|n| n > MAX_STRING_DATA) {
                            return Err(Diagnostic::new(pos, "E_SOURCE_LIMIT", "compile-time string exceeds data limit"));
                        }
                        Ok(Str(a + &b))
                    }
                    _ => Err(not_const(pos)),
                }
            }
            TExpr::Bin(op, lhs, rhs) => {
                // Binary operators in this guest subset evaluate both sides.
                let a = self.eval_const(lhs, pos)?;
                let b = self.eval_const(rhs, pos)?;
                match (a, b) {
                    (Int(a), Int(b)) => match op {
                        BinOp::Add => a.checked_add(b).map(Int).ok_or_else(|| overflow(pos)),
                        BinOp::Sub => a.checked_sub(b).map(Int).ok_or_else(|| overflow(pos)),
                        BinOp::Mul => a.checked_mul(b).map(Int).ok_or_else(|| overflow(pos)),
                        BinOp::Eq => Ok(Bool(a == b)),
                        BinOp::Ne => Ok(Bool(a != b)),
                        BinOp::Lt => Ok(Bool(a < b)),
                        BinOp::Le => Ok(Bool(a <= b)),
                        BinOp::Gt => Ok(Bool(a > b)),
                        BinOp::Ge => Ok(Bool(a >= b)),
                        _ => Err(not_const(pos)),
                    },
                    (Float(a), Float(b)) => match op {
                        BinOp::Add => Ok(Float(a + b)),
                        BinOp::Sub => Ok(Float(a - b)),
                        BinOp::Mul => Ok(Float(a * b)),
                        BinOp::Div => Ok(Float(a / b)),
                        BinOp::Eq => Ok(Bool(a == b)),
                        BinOp::Ne => Ok(Bool(a != b)),
                        BinOp::Lt => Ok(Bool(a < b)),
                        BinOp::Le => Ok(Bool(a <= b)),
                        BinOp::Gt => Ok(Bool(a > b)),
                        BinOp::Ge => Ok(Bool(a >= b)),
                        _ => Err(not_const(pos)),
                    },
                    (Bool(a), Bool(b)) => match op {
                        BinOp::And => Ok(Bool(a && b)),
                        BinOp::Or => Ok(Bool(a || b)),
                        BinOp::Eq => Ok(Bool(a == b)),
                        BinOp::Ne => Ok(Bool(a != b)),
                        _ => Err(not_const(pos)),
                    },
                    (Enum(a), Enum(b)) => match op {
                        BinOp::Eq => Ok(Bool(a == b)),
                        BinOp::Ne => Ok(Bool(a != b)),
                        _ => Err(not_const(pos)),
                    },
                    _ => Err(not_const(pos)),
                }
            }
            _ => Err(not_const(pos)),
        }
    }

    pub(super) fn materialize_const(&mut self, constant: &ConstValue, ty: Type, pos: Pos) -> R<TE> {
        let e = match constant {
            ConstValue::Int(v) if ty == Type::Int => TExpr::I64(*v),
            ConstValue::Float(v) if ty == Type::Float => TExpr::F64(*v),
            ConstValue::Bool(v) if ty == Type::Bool => TExpr::Bool(*v),
            ConstValue::Enum(v) if matches!(ty, Type::Enum(_)) => TExpr::EnumVal(*v),
            ConstValue::Str(v) if ty == Type::Str => {
                let (offset, len) = self.string(v, pos)?;
                TExpr::StrConst(offset, len)
            }
            _ => return Err(not_const(pos)),
        };
        Ok(TE { e, ty })
    }
}
