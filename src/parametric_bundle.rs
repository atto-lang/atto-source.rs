//! Staged compile-time parameter bundle binder over already resolved arguments.
//! No source parser, inference engine, allocation or Wasm guest admission.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Application,
    Refinement,
}

pub struct Formal<'a, V> {
    pub name: &'a str,
    pub accepts: fn(&V) -> bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BindError {
    DuplicateFormal(String),
    UnknownFormal(String),
    DuplicateArgument(String),
    InvalidArgument(String),
}

pub fn bind<V: Clone>(
    schema: &[Formal<'_, V>],
    seed: &[(&str, V)],
    updates: &[(&str, V)],
    mode: Mode,
) -> Result<Vec<(String, V)>, BindError> {
    use std::collections::BTreeMap;
    let mut names = BTreeMap::new();
    for (i, formal) in schema.iter().enumerate() {
        if names.insert(formal.name, i).is_some() {
            return Err(BindError::DuplicateFormal(formal.name.into()));
        }
    }
    let checked = |args: &[(&str, V)]| -> Result<Vec<Option<V>>, BindError> {
        let mut values = vec![None; schema.len()];
        for (name, value) in args {
            let i = *names
                .get(name)
                .ok_or_else(|| BindError::UnknownFormal((*name).into()))?;
            if values[i].is_some() {
                return Err(BindError::DuplicateArgument((*name).into()));
            }
            if !(schema[i].accepts)(value) {
                return Err(BindError::InvalidArgument((*name).into()));
            }
            values[i] = Some(value.clone());
        }
        Ok(values)
    };
    let base = checked(seed)?;
    let changes = checked(updates)?;
    let mut result = Vec::new();
    for ((formal, old), new) in schema.iter().zip(base).zip(changes) {
        if mode == Mode::Application && old.is_some() && new.is_some() {
            return Err(BindError::DuplicateArgument(formal.name.into()));
        }
        if let Some(value) = new.or(old) {
            result.push((formal.name.into(), value));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Value {
        Type(&'static str),
        Size(i64),
    }
    use Value::*;

    fn schema() -> [Formal<'static, Value>; 2] {
        [
            Formal { name: "type", accepts: |v| matches!(v, Type(_)) },
            Formal { name: "size", accepts: |v| matches!(v, Size(n) if *n >= 0) },
        ]
    }

    #[test]
    fn seed_refinement_is_canonical_and_non_mutating() {
        let schema = schema();
        let seed = [("size", Size(5)), ("type", Type("int"))];
        let expected = vec![("type".into(), Type("int")), ("size".into(), Size(6))];
        let mut result = bind(&schema, &seed, &[("size", Size(6))], Mode::Refinement).unwrap();
        assert_eq!(result, expected);
        assert_eq!(seed[0].1, Size(5));
        assert_eq!(bind(&schema, &[seed[1].clone(), seed[0].clone()], &[("size", Size(6))], Mode::Refinement).unwrap(), expected);
        assert_eq!(bind(&schema, &seed[1..], &[("size", Size(6))], Mode::Refinement).unwrap(), expected);
        assert_eq!(bind(&schema, &seed, &[], Mode::Refinement).unwrap()[1].1, Size(5));
        result[1].1 = Size(9);
        assert_eq!(seed[0].1, Size(5));
        assert_eq!(bind(&schema, &seed, &[("size", Size(6))], Mode::Refinement).unwrap(), expected);
    }

    #[test]
    fn ordinary_application_never_overrides() {
        let schema = schema();
        let seed = [("size", Size(5)), ("type", Type("int"))];
        assert_eq!(bind(&schema, &seed[1..], &[("size", Size(6))], Mode::Application).unwrap(),
            vec![("type".into(), Type("int")), ("size".into(), Size(6))]);
        assert_eq!(bind(&schema, &seed, &[], Mode::Application).unwrap()[1].1, Size(5));
        for n in [5, 6] {
            assert_eq!(bind(&schema, &seed, &[("size", Size(n))], Mode::Application),
                Err(BindError::DuplicateArgument("size".into())));
        }
        assert_eq!(bind(&schema, &seed, &[("unknown", Size(1))], Mode::Application),
            Err(BindError::UnknownFormal("unknown".into())));
    }

    #[test]
    fn validation_rejects_duplicates_unknown_kind_and_bounds() {
        let schema = schema();
        let seed = [("size", Size(5)), ("type", Type("int"))];
        assert_eq!(bind(&schema, &seed, &[("size", Size(6)), ("size", Size(6))], Mode::Refinement),
            Err(BindError::DuplicateArgument("size".into())));
        assert_eq!(bind(&schema, &[seed[0].clone(), seed[0].clone()], &[], Mode::Refinement),
            Err(BindError::DuplicateArgument("size".into())));
        assert_eq!(bind(&schema, &seed, &[("unknown", Size(1))], Mode::Refinement),
            Err(BindError::UnknownFormal("unknown".into())));
        for bad in [Type("6"), Size(-1)] {
            assert_eq!(bind(&schema, &seed, &[("size", bad)], Mode::Refinement),
                Err(BindError::InvalidArgument("size".into())));
        }
        let dup = [
            Formal { name: "type", accepts: schema[0].accepts },
            Formal { name: "type", accepts: schema[0].accepts },
        ];
        assert_eq!(bind(&dup, &seed, &[], Mode::Refinement), Err(BindError::DuplicateFormal("type".into())));
    }
}
