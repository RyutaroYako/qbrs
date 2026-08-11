//! Reusable named-placeholder prepared statements (`prepare!{}`).

use std::marker::PhantomData;

use super::{Select, Selection};
use crate::dialect::Dialect;
use crate::expr::Value;

/// Implemented by the `prepare!{}`-generated `Params` struct. `self` is
/// consumed (not borrowed) so a value field can move straight into its
/// `Value` without cloning — a `Prepared` query is meant to be reused
/// across many `.execute(params)` calls, but each call gets its own fresh
/// `Params` value, not a shared one.
pub trait PreparedParams {
    fn into_named_values(self) -> Vec<(&'static str, Value)>;
}

/// A query rendered once, with some `Value::Placeholder(name)` slots left
/// unresolved, reusable across many `.execute(params)` calls with different
/// `Params` values — this is the reusable-prepared-statement half of what
/// Drizzle's `.prepare()` + `sql.placeholder()` does, with one concrete
/// improvement: `execute` takes the exact `Params` struct `prepare!{}`
/// generated for this query, not an untyped `Record<string, unknown>`, so a
/// missing or mistyped value is a compile error rather than a runtime one
/// (Drizzle's own documented gap — see the design plan's "Prepared
/// Statements" section).
///
/// **Soundness note**: this guarantee holds as long as every placeholder in
/// the query was built via `Params::field()` accessors from the *same*
/// `prepare!{}` invocation as `Params` — the low-level `placeholder(name)`
/// function is `#[doc(hidden)]` for exactly this reason, since a
/// hand-written mismatched name would only fail at `.execute()` time (an
/// error, not a panic — see `execute`'s doc comment).
pub struct Prepared<Params, Output> {
    sql: String,
    template: Vec<Value>,
    _marker: PhantomData<fn() -> (Params, Output)>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Renders this query once, leaving any `Value::Placeholder` slots
    /// (built via `Params::field()` accessors) unresolved — see
    /// `Prepared`'s doc comment. `Output` (the decoded row type) is
    /// captured here, the same way `.erase()` captures it for `DynSelect`,
    /// so the execution layer knows what to decode into without needing
    /// `Sel` (and therefore `Scope`) to still be around.
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
/// template has no matching field in the `Params` value passed in — only
/// reachable by hand-constructing a mismatched `placeholder(name)` outside
/// the `prepare!{}` macro's generated accessors, see `Prepared`'s doc
/// comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedPlaceholder(pub &'static str);

impl std::fmt::Display for UnresolvedPlaceholder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no value provided for placeholder `{}`", self.0)
    }
}
impl std::error::Error for UnresolvedPlaceholder {}

impl<Params: PreparedParams, Output> Prepared<Params, Output> {
    /// Substitutes every named placeholder in the template with the
    /// matching value from `params`, producing the same `(String,
    /// Vec<Value>)` shape `to_sql()` returns — the execution layer
    /// (`qbrs-sqlx`) binds this exactly like any other rendered query.
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
