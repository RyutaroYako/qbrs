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
/// unresolved, reusable across many `.load(executor, params)` calls. `load`
/// takes the exact `Params` struct `prepare!{}` generated for this query, so
/// a missing or mistyped value is a compile error. The dialect it was
/// rendered in stays in its type, so it can only be run by an executor of
/// that dialect — the same rule `Select` and `DynSelect` follow.
///
/// That holds as long as every placeholder came from `Params::field()`
/// accessors of the same `prepare!{}` invocation, which is why the
/// lower-level `expr::placeholder` is `#[doc(hidden)]`: a hand-written name
/// that matches nothing fails at `.load()` time instead.
pub struct Prepared<D, Params, Output> {
    sql: String,
    template: Vec<Value>,
    _marker: PhantomData<fn() -> (D, Params, Output)>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Renders this query once, leaving its `Value::Placeholder` slots
    /// unresolved. `Output` is captured here, as `.erase()` does for
    /// `DynSelect`, so the execution layer can decode rows without `Sel`
    /// (and therefore `Scope`) still being around.
    pub fn prepare<Params, Idx>(&self) -> Prepared<D, Params, Sel::Output>
    where
        D: Dialect,
        Sel: Selection<Scope, Idx>,
    {
        let (sql, template) = self.render_as::<Idx>();
        Prepared {
            sql,
            template,
            _marker: PhantomData,
        }
    }

    /// The same query prepared as its own total — `count_sql` with the
    /// placeholders still unresolved, so a paginated endpoint reuses one
    /// rendering for the page and one for the count. `Total` rather than
    /// `i64`: what a statement produces is what decides how it is run, and a
    /// total is a number, not a row.
    pub fn prepare_count<Params, Idx>(&self) -> Prepared<D, Params, Total>
    where
        D: Dialect,
        Sel: Selection<Scope, Idx>,
    {
        let (sql, template) = self.count_sql::<Idx>();
        Prepared {
            sql,
            template,
            _marker: PhantomData,
        }
    }
}

/// The output of a `prepare_count()`-built query: deliberately not a
/// decodable row, so a total is counted and never loaded, and a prepared
/// `SELECT` of one `i64` column is never mistaken for one.
pub struct Total;

/// Returned by `Prepared::resolve` — and so by the `.load()` that calls
/// it — when a placeholder in the
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

impl<D, Params: PreparedParams, Output> Prepared<D, Params, Output> {
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
