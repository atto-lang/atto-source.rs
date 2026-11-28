//! Static argument slot planning. Inputs must come from authenticated typed
//! IR: this module neither resolves source identifiers nor executes operands.
//! Plans are in source order; the eventual lowering evaluates each operand
//! once, then projects values and fills ABI slots. No implicit copy or move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item<'a> {
    Positional,
    Named(&'a str),
    Record(&'a [&'a str]),
    /// Every fixed Tuple spreads by index; preserve each complete element.
    Tuple(usize),
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownName, Duplicate, Missing, PositionalAfterNamed,
    Unsupported, Limit, InvalidSignature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub operand: usize,
    /// None for ordinary arguments; otherwise index in proven aggregate.
    pub projection: Option<usize>,
    pub parameter: usize,
}

const MAX_SLOTS: usize = 1024;

pub fn plan(items: &[Item<'_>], formals: &[&str], defaults: &[bool])
    -> Result<Vec<Slot>, (usize, Error)>
{
    if formals.len() > MAX_SLOTS || items.len() > MAX_SLOTS {
        return Err((0, Error::Limit));
    }
    if defaults.len() != formals.len() {
        return Err((0, Error::InvalidSignature));
    }
    let mut occupied = vec![false; formals.len()];
    let mut out = Vec::new();
    let mut next = 0;
    let mut named = false;
    let mut names = std::collections::BTreeMap::new();
    for (i, &name) in formals.iter().enumerate() {
        // Ignored private ABI slots are positional-only. The caller supplies
        // authenticated source parameter names, never a synthetic name.
        if name.is_empty() || name.starts_with('$') || name == "_" { continue; }
        if names.insert(name, i).is_some() {
            return Err((0, Error::InvalidSignature));
        }
    }
    for (operand, item) in items.iter().enumerate() {
        let count = match item {
            Item::Positional | Item::Named(_) => 1,
            Item::Record(fields) => fields.len(),
            Item::Tuple(n) => *n,
            Item::Unsupported => return Err((operand, Error::Unsupported)),
        };
        if count > MAX_SLOTS || out.len().saturating_add(count) > MAX_SLOTS {
            return Err((operand, Error::Limit));
        }
        for i in 0..count {
            let label = match item {
                Item::Named(s) => Some(*s),
                Item::Record(fields) => Some(fields[i]),
                _ => None,
            };
            let parameter = if let Some(name) = label {
                named = true;
                *names.get(name).ok_or((operand, Error::UnknownName))?
            } else {
                if named { return Err((operand, Error::PositionalAfterNamed)); }
                if next >= formals.len() { return Err((operand, Error::Limit)); }
                let slot = next;
                next += 1;
                slot
            };
            if occupied[parameter] { return Err((operand, Error::Duplicate)); }
            occupied[parameter] = true;
            let projection = match item {
                Item::Positional | Item::Named(_) => None,
                _ => Some(i),
            };
            out.push(Slot { operand, projection, parameter });
        }
    }
    if occupied.iter().zip(defaults).any(|(&used, &default)| !used && !default) {
        return Err((items.len(), Error::Missing));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_remaps_by_name_without_reordering_operands() {
        let p = plan(&[Item::Record(&["active", "name", "age"])], &["name","age","active"], &[false;3]).unwrap();
        assert_eq!(p, vec![
            Slot { operand:0,projection:Some(0),parameter:2 },
            Slot { operand:0,projection:Some(1),parameter:0 },
            Slot { operand:0,projection:Some(2),parameter:1 },
        ]);
    }
    #[test]
    fn records_and_explicit_and_empty_spread_compose() {
        let p = plan(&[Item::Tuple(0),Item::Positional,Item::Record(&["b"]),Item::Named("c")],
            &["a","b","c","d"], &[false,false,false,true]).unwrap();
        assert_eq!(p.iter().map(|x|(x.operand,x.parameter)).collect::<Vec<_>>(),vec![(1,0),(2,1),(3,2)]);
        assert_eq!(p[1].projection,Some(0));
    }
    #[test]
    fn every_tuple_is_positional_even_with_reversed_formal_names() {
        let p = plan(&[Item::Tuple(2)], &["a","b"], &[false;2]).unwrap();
        assert_eq!(p, vec![Slot { operand:0,projection:Some(0),parameter:0 }, Slot { operand:0,projection:Some(1),parameter:1 }]);
    }
    #[test]
    fn reject_duplicate_unknown_missing_order_and_unsupported() {
        let f = &["a","b"];
        let d = &[false;2];
        for (items, error) in [
            (vec![Item::Positional,Item::Record(&["a"])],Error::Duplicate),
            (vec![Item::Record(&["z"])],Error::UnknownName),
            (vec![Item::Named("a")],Error::Missing),
            (vec![Item::Named("a"),Item::Tuple(0),Item::Positional],Error::PositionalAfterNamed),
            (vec![Item::Unsupported],Error::Unsupported),
            (vec![Item::Tuple(usize::MAX)],Error::Limit),
        ] { assert_eq!(plan(&items,f,d).unwrap_err().1,error); }
    }
    #[test]
    fn private_ignored_slots_do_not_become_named_parameters() {
        assert_eq!(plan(&[Item::Named("$ignored_lambda_0")], &["$ignored_lambda_0"], &[false]).unwrap_err().1,Error::UnknownName);
        assert!(plan(&[Item::Positional], &["$ignored_lambda_0"], &[false]).is_ok());
    }
}
