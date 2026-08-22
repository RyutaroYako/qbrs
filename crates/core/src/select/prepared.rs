//! Reusable named-placeholder prepared statements (`prepare!{}`).

use std::marker::PhantomData;

use super::{Select, Selection};
use crate::dialect::Dialect;
use crate::expr::Value;

/// Implemented by the `prepare!{}`-generated `Params` struct. Consuming
/// `self` lets each field move straight into its `Value`; a `Prepared` query
/// is reused across calls, but each call brings its own `Params`.
pub trait PreparedParams {
    fn into_named_values(self) -> Vec<(&'static str, Value)>;
}

/// A query rendered once, with its `Value::Placeholder(name)` slots left
/// unresolved, reusable across many `.execute(params)` calls. `execute`
/// takes the exact `Params` struct `prepare!{}` generated for this query, so
/// a missing or mistyped value is a compile error.
///
/// That holds as long as every placeholder came from `Params::field()`
/// accessors of the same `prepare!{}` invocation, which is why the
/// lower-level `expr::placeholder` is `#[doc(hidden)]`: a hand-written name
/// that matches nothing fails at `.execute()` time instead.
pub struct Prepared<Params, Output> {
    sql: String,
    template: Vec<Value>,
    _marker: PhantomData<fn() -> (Params, Output)>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Renders this query once, leaving its `Value::Placeholder` slots
    /// unresolved. `Output` is captured here, as `.erase()` does for
    /// `DynSelect`, so the execution layer can decode rows without `Sel`
    /// (and therefore `Scope`) still being around.
    pub fn prepare<Params, Idx>(&self) -> Prepared<Params, Sel::Output>
    where
        D: Dialect,
        Sel: Selection<Scope, Idx>,
    {
        let (sql, template) = self.render_as::<D, Idx>();
        Prepared {
            sql,
            template,
            _marker: PhantomData,
        }
    }
}

/// Returned by `Prepared::execute`/`resolve` when a placeholder in the
/// template has no matching field in the `Params` passed in. Reachable only
/// by hand-constructing a mismatched `expr::placeholder` name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedPlaceholder(pub &'static str);

impl std::fmt::Display for UnresolvedPlaceholder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no value provided for placeholder `{}`", self.0)
    }
}
impl std::error::Error for UnresolvedPlaceholder {}

impl<Params: PreparedParams, Output> Prepared<Params, Output> {
    /// Substitutes every named placeholder with the matching value from
    /// `params`, producing the same `(String, Vec<Value>)` shape `to_sql()`
    /// returns.
    pub fn resolve(&self, params: Params) -> Result<(String, Vec<Value>), UnresolvedPlaceholder> {
        let named = params.into_named_values();
        let mut resolved = Vec::with_capacity(self.template.len());
        for v in &self.template {
            match v {
                Value::Placeholder(name) => {
                    let found = named
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone())
                        .ok_or(UnresolvedPlaceholder(name))?;
                    resolved.push(found);
                }
                other => resolved.push(other.clone()),
            }
        }
        Ok((self.sql.clone(), resolved))
    }
}
