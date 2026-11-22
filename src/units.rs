//! Compiler-verified units of measure v1: the dimension/scale algebra that
//! the type checker uses for `Quantity<int, "unit">` style annotations.
//!
//! A unit is an exact rational scale times a product of integer powers of
//! the seven SI base dimensions plus two non-SI base dimensions that
//! programs routinely confuse with plain integers: information (`B`) and
//! a dimensionless `count`. Rules (all decided at compile time):
//!
//! * `+`, `-`, comparisons and assignment require the *same unit* (same
//!   dimension AND same scale). `s + ms` is rejected rather than silently
//!   rescaled; an explicit `convert` with an exact factor is required.
//! * `*` and `/` compose dimensions and multiply scales exactly.
//! * Converting between different dimensions (seconds to bytes, meters to
//!   seconds) is never possible.
//! * Scales are exact rationals over u64/i128, never floats, so
//!   `KiB -> B` is exactly 1024 and `ms -> s` exactly 1/1000.
//!
//! Numeric representation, overflow and rounding of the *values* are
//! owned by the numeric contract (atto-spec #209); this module only
//! decides whether an operation is dimensionally legal and the exact
//! factor a conversion multiplies by.
use crate::diag::{Diagnostic, Pos};
use std::collections::BTreeMap;
use std::fmt;

type R<T> = Result<T, Diagnostic>;

pub const BASE_NAMES: [&str; 9] = ["m", "kg", "s", "A", "K", "mol", "cd", "B", "count"];
const MAX_EXPONENT: i8 = 12;
const MAX_UNIT_EXPR: usize = 128;

fn err(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Pos { line: 1, col: 1 }, code, message)
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Exact positive rational scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ratio {
    pub num: u128,
    pub den: u128,
}

impl Ratio {
    pub const ONE: Ratio = Ratio { num: 1, den: 1 };

    pub fn new(num: u128, den: u128) -> R<Ratio> {
        if num == 0 || den == 0 {
            return Err(err("E_UNIT_SCALE", "unit scale must be a positive rational"));
        }
        let g = gcd(num, den);
        Ok(Ratio {
            num: num / g,
            den: den / g,
        })
    }

    fn mul(self, o: Ratio) -> R<Ratio> {
        // Cross-reduce first so exact products stay small.
        let g1 = gcd(self.num, o.den);
        let g2 = gcd(o.num, self.den);
        let num = (self.num / g1).checked_mul(o.num / g2);
        let den = (self.den / g2).checked_mul(o.den / g1);
        match (num, den) {
            (Some(n), Some(d)) => Ratio::new(n, d),
            _ => Err(err("E_UNIT_SCALE", "unit scale overflows exact 128-bit rational")),
        }
    }

    fn inv(self) -> Ratio {
        Ratio {
            num: self.den,
            den: self.num,
        }
    }

    fn pow(self, e: i8) -> R<Ratio> {
        let mut out = Ratio::ONE;
        let base = if e < 0 { self.inv() } else { self };
        for _ in 0..e.unsigned_abs() {
            out = out.mul(base)?;
        }
        Ok(out)
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Integer exponents of the nine base dimensions, in `BASE_NAMES` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Dimension(pub [i8; 9]);

impl Dimension {
    pub fn is_dimensionless(&self) -> bool {
        self.0.iter().all(|&e| e == 0)
    }

    fn combine(self, o: Dimension, sign: i8) -> R<Dimension> {
        let mut out = [0i8; 9];
        for (i, slot) in out.iter_mut().enumerate() {
            let v = self.0[i] as i16 + sign as i16 * o.0[i] as i16;
            if v.unsigned_abs() > MAX_EXPONENT as u16 {
                return Err(err("E_UNIT_EXPONENT", "unit exponent exceeds +/-12"));
            }
            *slot = v as i8;
        }
        Ok(Dimension(out))
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = BASE_NAMES
            .iter()
            .zip(self.0)
            .filter(|(_, e)| *e != 0)
            .map(|(n, e)| if e == 1 { n.to_string() } else { format!("{n}^{e}") })
            .collect();
        if parts.is_empty() {
            write!(f, "1")
        } else {
            write!(f, "{}", parts.join("*"))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unit {
    pub scale: Ratio,
    pub dim: Dimension,
}

impl Unit {
    pub const DIMENSIONLESS: Unit = Unit {
        scale: Ratio::ONE,
        dim: Dimension([0; 9]),
    };

    pub fn mul(self, o: Unit) -> R<Unit> {
        Ok(Unit {
            scale: self.scale.mul(o.scale)?,
            dim: self.dim.combine(o.dim, 1)?,
        })
    }

    pub fn div(self, o: Unit) -> R<Unit> {
        Ok(Unit {
            scale: self.scale.mul(o.scale.inv())?,
            dim: self.dim.combine(o.dim, -1)?,
        })
    }

    pub fn pow(self, e: i8) -> R<Unit> {
        let mut dim = Dimension::default();
        for _ in 0..e.unsigned_abs() {
            dim = dim.combine(self.dim, if e < 0 { -1 } else { 1 })?;
        }
        Ok(Unit {
            scale: self.scale.pow(e)?,
            dim,
        })
    }
}

fn base(i: usize) -> Unit {
    let mut d = [0i8; 9];
    d[i] = 1;
    Unit {
        scale: Ratio::ONE,
        dim: Dimension(d),
    }
}

/// Closed, compiler-owned unit registry. User-declared units are a later
/// slice (`define unit furlong = 201168/1000 m;`).
pub struct Registry {
    units: BTreeMap<&'static str, Unit>,
}

impl Default for Registry {
    fn default() -> Self {
        let mut units = BTreeMap::new();
        for (i, name) in BASE_NAMES.iter().enumerate() {
            units.insert(*name, base(i));
        }
        let s = base(2);
        let m = base(0);
        let byte = base(7);
        let scaled = |u: Unit, n: u128, d: u128| Unit {
            scale: Ratio::new(n, d).expect("static scale"),
            dim: u.dim,
        };
        units.insert("g", scaled(base(1), 1, 1000));
        units.insert("ms", scaled(s, 1, 1_000));
        units.insert("us", scaled(s, 1, 1_000_000));
        units.insert("ns", scaled(s, 1, 1_000_000_000));
        units.insert("min", scaled(s, 60, 1));
        units.insert("h", scaled(s, 3600, 1));
        units.insert("km", scaled(m, 1000, 1));
        units.insert("cm", scaled(m, 1, 100));
        units.insert("mm", scaled(m, 1, 1000));
        units.insert("bit", scaled(byte, 1, 8));
        units.insert("KiB", scaled(byte, 1024, 1));
        units.insert("MiB", scaled(byte, 1 << 20, 1));
        units.insert("GiB", scaled(byte, 1 << 30, 1));
        units.insert("KB", scaled(byte, 1000, 1));
        units.insert("MB", scaled(byte, 1_000_000, 1));
        let newton = base(1).mul(m).and_then(|u| u.div(s.pow(2)?)).expect("N");
        units.insert("N", newton);
        units.insert("J", newton.mul(m).expect("J"));
        units.insert("W", newton.mul(m).and_then(|u| u.div(s)).expect("W"));
        units.insert("Hz", Unit::DIMENSIONLESS.div(s).expect("Hz"));
        Registry { units }
    }
}

impl Registry {
    pub fn lookup(&self, name: &str) -> R<Unit> {
        self.units
            .get(name)
            .copied()
            .ok_or_else(|| err("E_UNIT_UNKNOWN", format!("unknown unit '{name}'")))
    }

    /// Parse `m/s^2`, `kg*m/s^2`, `B/s`, `1/s`, `KiB*count^-1`.
    /// Grammar: term (('*' | '/') term)* ; term = name ('^' '-'? digit+)? | '1'.
    /// Left-associative; no parentheses in v1 (fail closed rather than guess).
    pub fn parse(&self, expr: &str) -> R<Unit> {
        if expr.is_empty() || expr.len() > MAX_UNIT_EXPR || !expr.is_ascii() {
            return Err(err("E_UNIT_SYNTAX", "unit expression must be 1..=128 ASCII bytes"));
        }
        let mut out = Unit::DIMENSIONLESS;
        let mut op = '*';
        let mut rest = expr.trim();
        loop {
            let end = rest.find(['*', '/']).unwrap_or(rest.len());
            let term = rest[..end].trim();
            let unit = self.term(term)?;
            out = if op == '*' { out.mul(unit)? } else { out.div(unit)? };
            if end == rest.len() {
                return Ok(out);
            }
            op = rest.as_bytes()[end] as char;
            rest = &rest[end + 1..];
        }
    }

    fn term(&self, term: &str) -> R<Unit> {
        if term == "1" {
            return Ok(Unit::DIMENSIONLESS);
        }
        let (name, exp) = match term.split_once('^') {
            Some((n, e)) => {
                let e: i8 = e
                    .trim()
                    .parse()
                    .map_err(|_| err("E_UNIT_SYNTAX", format!("invalid exponent in '{term}'")))?;
                if e == 0 || e.unsigned_abs() > MAX_EXPONENT as u8 {
                    return Err(err("E_UNIT_EXPONENT", "unit exponent must be nonzero and within +/-12"));
                }
                (n.trim(), e)
            }
            None => (term, 1),
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(err("E_UNIT_SYNTAX", format!("invalid unit term '{term}'")));
        }
        self.lookup(name)?.pow(exp)
    }
}

/// `a + b`, `a - b`, comparisons, assignment, returns.
pub fn check_same(op: &str, a: Unit, b: Unit) -> R<Unit> {
    if a == b {
        return Ok(a);
    }
    if a.dim == b.dim {
        return Err(err(
            "E_UNIT_SCALE_MISMATCH",
            format!(
                "'{op}' mixes the same dimension at different scales ({} vs {}); convert explicitly",
                a.scale, b.scale
            ),
        ));
    }
    Err(err(
        "E_UNIT_MISMATCH",
        format!("'{op}' mixes incompatible dimensions {} and {}", a.dim, b.dim),
    ))
}

/// Exact factor `f` such that `value_in_to = value_in_from * f`.
pub fn conversion_factor(from: Unit, to: Unit) -> R<Ratio> {
    if from.dim != to.dim {
        return Err(err(
            "E_UNIT_MISMATCH",
            format!("cannot convert {} to {}: dimensions differ", from.dim, to.dim),
        ));
    }
    from.scale.mul(to.scale.inv())
}

/// Exact integer conversion: allowed only when the factor is integral
/// (`s -> ms`, `KiB -> B`). Downscaling (`ms -> s`) needs an explicit
/// rounding mode from the numeric contract and is rejected here.
pub fn convert_int_exact(value: i64, from: Unit, to: Unit) -> R<i64> {
    let f = conversion_factor(from, to)?;
    if f.den != 1 {
        return Err(err(
            "E_UNIT_LOSSY",
            format!("conversion factor {f} is not integral; choose an explicit rounding mode"),
        ));
    }
    let factor = i64::try_from(f.num).map_err(|_| err("E_UNIT_OVERFLOW", "conversion factor exceeds i64"))?;
    value
        .checked_mul(factor)
        .ok_or_else(|| err("E_UNIT_OVERFLOW", "converted value overflows i64"))
}
