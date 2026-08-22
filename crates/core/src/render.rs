//! Rendering `ExprKind` into a SQL string plus a positional parameter list.
//! Generic over `D: Dialect` for identifier quoting and placeholder style,
//! but `ExprKind`/`Value` themselves stay closed, non-generic types, so this
//! costs one instantiation per dialect used in a program rather than one per
//! query shape.

use crate::dialect::Dialect;
use crate::expr::{BinOp, CastTarget, ExprKind, SortDir, Value};

pub(crate) fn render_expr<D: Dialect>(expr: &ExprKind, out: &mut String, params: &mut Vec<Value>) {
    match expr {
        ExprKind::Column { table, name } => {
            push_ident::<D>(out, table);
            out.push('.');
            push_ident::<D>(out, name);
        }
        ExprKind::Value(v) => {
            params.push(v.clone());
            out.push_str(&D::placeholder(params.len()));
        }
        ExprKind::BinOp { op, lhs, rhs } => {
            out.push('(');
            render_expr::<D>(lhs, out, params);
            out.push_str(match op {
                BinOp::Eq => " = ",
                BinOp::Ne => " <> ",
                BinOp::Lt => " < ",
                BinOp::Lte => " <= ",
                BinOp::Gt => " > ",
                BinOp::Gte => " >= ",
                BinOp::Like => " LIKE ",
            });
            render_expr::<D>(rhs, out, params);
            out.push(')');
        }
        ExprKind::And(parts) => render_bool_list::<D>(parts, "AND", out, params),
        ExprKind::Or(parts) => render_bool_list::<D>(parts, "OR", out, params),
        ExprKind::Not(inner) => {
            out.push_str("(NOT ");
            render_expr::<D>(inner, out, params);
            out.push(')');
        }
        ExprKind::Cast { expr, target } => {
            out.push_str("CAST(");
            render_expr::<D>(expr, out, params);
            out.push_str(" AS ");
            out.push_str(match target {
                CastTarget::BigInt => D::CAST_BIGINT,
                CastTarget::Double => D::CAST_DOUBLE,
            });
            out.push(')');
        }
        ExprKind::Func { name, arg } => {
            out.push_str(name);
            out.push('(');
            render_expr::<D>(arg, out, params);
            out.push(')');
        }
        ExprKind::IsNull { expr, negated } => {
            out.push('(');
            render_expr::<D>(expr, out, params);
            out.push_str(if *negated {
                " IS NOT NULL)"
            } else {
                " IS NULL)"
            });
        }
        ExprKind::InList { expr, values } => {
            if values.is_empty() {
                out.push_str("FALSE");
                return;
            }
            out.push('(');
            render_expr::<D>(expr, out, params);
            out.push_str(" IN (");
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                render_expr::<D>(v, out, params);
            }
            out.push_str("))");
        }
        ExprKind::Raw(fragment) => {
            // A fragment's internal precedence is unknown (it may be `a OR
            // b`), so parenthesize: it must not change meaning when spliced
            // into a larger AND/OR chain.
            out.push('(');
            fragment.splice_into::<D>(out, params);
            out.push(')');
        }
        ExprKind::Window {
            func,
            partition_by,
            order_by,
        } => {
            // No defensive parens here, unlike `Raw`: `OVER` only attaches
            // to a bare function-call syntax node, so `(row_number()) OVER
            // (..)` would not be valid SQL.
            out.push_str(func);
            out.push_str(" OVER (");
            if !partition_by.is_empty() {
                out.push_str("PARTITION BY ");
                for (i, p) in partition_by.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render_expr::<D>(p, out, params);
                }
            }
            if !order_by.is_empty() {
                if !partition_by.is_empty() {
                    out.push(' ');
                }
                out.push_str("ORDER BY ");
                for (i, (e, dir)) in order_by.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render_expr::<D>(e, out, params);
                    out.push_str(match dir {
                        SortDir::Asc => " ASC",
                        SortDir::Desc => " DESC",
                    });
                }
            }
            out.push(')');
        }
    }
}

#[doc(hidden)]
/// One item in a rendered `SELECT`/`RETURNING` list. `label` is `Some` only
/// for an item given a `row::AliasKey` alias, which is the only thing that
/// emits `AS`.
pub struct SelectItem {
    pub(crate) kind: ExprKind,
    pub(crate) label: Option<&'static str>,
}

impl SelectItem {
    pub(crate) fn bare(kind: ExprKind) -> Self {
        SelectItem { kind, label: None }
    }

    pub(crate) fn labeled(kind: ExprKind, label: &'static str) -> Self {
        SelectItem {
            kind,
            label: Some(label),
        }
    }
}

/// Renders a comma-separated `SELECT`/`RETURNING` list, emitting each item's
/// `AS` label where it has one.
pub(crate) fn render_select_list<D: Dialect>(
    items: &[SelectItem],
    out: &mut String,
    params: &mut Vec<Value>,
) {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        render_expr::<D>(&item.kind, out, params);
        if let Some(label) = item.label {
            out.push_str(" AS ");
            push_ident::<D>(out, label);
        }
    }
}

/// The byte a not-yet-numbered bind parameter occupies inside a `Fragment`.
/// Not `?`, because `sql!{}` text may contain a literal one and a fragment
/// is spliced more than once on its way into a query — a subquery inside a
/// `UNION` branch renders twice — so the marker has to be something the
/// authored text cannot hold.
pub(crate) const BIND_MARKER: char = '\u{1}';

/// A piece of SQL destined to be embedded in a larger query: a subquery, a
/// CTE body, a set-operation branch, or a `sql!{}` escape hatch. Its bind
/// parameters are always written as `?`, never in a dialect's own style,
/// because their final numbering depends on how much of the host query has
/// already been rendered — `splice_into` assigns it.
#[derive(Debug, Clone)]
pub struct Fragment {
    sql: String,
    params: Vec<Value>,
}

impl Fragment {
    /// `sql` must use `BIND_MARKER` for every bind parameter, with one entry
    /// in `params` per marker, in order.
    pub(crate) fn new(sql: String, params: Vec<Value>) -> Self {
        Fragment { sql, params }
    }

    /// Builds a fragment from `sql!{}`'s authored text, where a bind slot is
    /// `?` and `??` is a literal one. Doing the substitution once, here,
    /// is what keeps the two from ever being the same character again.
    pub(crate) fn from_authored(sql: &str, params: Vec<Value>) -> Self {
        let mut out = String::with_capacity(sql.len());
        let mut slots = 0usize;
        let mut chars = sql.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '?' {
                out.push(c);
                continue;
            }
            if chars.peek() == Some(&'?') {
                chars.next();
                out.push('?');
            } else {
                out.push(BIND_MARKER);
                slots += 1;
            }
        }
        assert_eq!(
            slots,
            params.len(),
            "`sql!` has {} `?` placeholders but was given {} values (write `??` for a literal `?`)",
            slots,
            params.len()
        );
        Fragment { sql: out, params }
    }

    /// Wraps the fragment in surrounding SQL, e.g. `EXISTS (`..`)`.
    pub(crate) fn enclosed_in(self, before: &str, after: &str) -> Self {
        Fragment {
            sql: format!("{before}{}{after}", self.sql),
            params: self.params,
        }
    }

    /// Appends this fragment to a query being rendered, renumbering its bind
    /// markers to continue `params`' sequence.
    pub(crate) fn splice_into<D: Dialect>(&self, out: &mut String, params: &mut Vec<Value>) {
        let mut next = self.params.iter();
        for c in self.sql.chars() {
            if c != BIND_MARKER {
                out.push(c);
                continue;
            }
            let value = next.next().expect("one value per bind marker");
            params.push(value.clone());
            out.push_str(&D::placeholder(params.len()));
        }
    }
}

fn push_ident<D: Dialect>(out: &mut String, ident: &str) {
    out.push(D::IDENTIFIER_QUOTE);
    for c in ident.chars() {
        // A quote inside an identifier is escaped by doubling it, in every
        // dialect this crate speaks. `#[table(name = "..")]` takes an
        // arbitrary string, so an unescaped one would end the identifier.
        if c == D::IDENTIFIER_QUOTE {
            out.push(c);
        }
        out.push(c);
    }
    out.push(D::IDENTIFIER_QUOTE);
}

fn render_bool_list<D: Dialect>(
    parts: &[ExprKind],
    joiner: &str,
    out: &mut String,
    params: &mut Vec<Value>,
) {
    if parts.is_empty() {
        // An empty AND/OR should never make it into a real query (the
        // builder never pushes one), but render a harmless tautology/
        // contradiction rather than emitting invalid SQL if it ever did.
        out.push_str(if joiner == "AND" { "TRUE" } else { "FALSE" });
        return;
    }
    out.push('(');
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
            out.push_str(joiner);
            out.push(' ');
        }
        render_expr::<D>(part, out, params);
    }
    out.push(')');
}

/// Renders a bare identifier (a table name in a `FROM`/`INSERT INTO`/etc.
/// clause, not part of an `ExprKind`) with the dialect's quoting.
pub(crate) fn render_ident<D: Dialect>(out: &mut String, ident: &str) {
    push_ident::<D>(out, ident);
}
