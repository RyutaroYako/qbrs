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
/// a missing or mistyped value is caught. The dialect it was rendered in
/// stays in its type, so it can only be run by an executor of that dialect,
/// the same rule `Select` and `DynSelect` follow.
///
/// `Params` is a free parameter, though: nothing ties the placeholder names
/// baked into the template to the struct that fills them. A query built from
/// one `prepare!` struct and run with another is therefore caught at
/// `resolve` rather than at compile time. Placeholder names are qualified by
/// the module and struct they were declared in, so that mismatch is always
/// an `UnresolvedPlaceholder` and never a value bound to the wrong slot.
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
    pub fn prepare<Params, Idx>(&self, _dialect: D) -> Prepared<D, Params, Sel::Output>
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

    /// The same query prepared as its own total, meaning `count_sql` with
    /// the placeholders still unresolved, so a paginated endpoint reuses one
    /// rendering for the page and one for the count. `Total` rather than
    /// `i64`: what a statement produces is what decides how it is run, and a
    /// total is a number, not a row.
    pub fn prepare_count<Params, Idx>(&self, _dialect: D) -> Prepared<D, Params, Total>
    where
        D: Dialect,
        Sel: Selection<Scope, Idx>,
    {
        let (sql, template) = self.count_sql::<Idx>(D::default());
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

/// Returned by `Prepared::resolve` (and so by the `.load()` that calls it)
/// when a placeholder in the template has no matching field in the `Params`
/// passed in: a query prepared with one `prepare!` struct and run with
/// another, or a hand-constructed `expr::placeholder` name.
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
