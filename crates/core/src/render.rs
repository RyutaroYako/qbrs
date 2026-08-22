//! Rendering `ExprKind` into a SQL string plus a positional parameter list.
//! Generic over `D: Dialect` for identifier quoting and placeholder style,
//! but `ExprKind`/`Value` themselves stay closed, non-generic types, so this
//! costs one instantiation per dialect used in a program rather than one per
//! query shape.

use crate::dialect::Dialect;
use crate::expr::{BinOp, ExprKind, SortDir, Value};

pub fn render_expr<D: Dialect>(expr: &ExprKind, out: &mut String, params: &mut Vec<Value>) {
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
pub fn render_select_list<D: Dialect>(
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
    /// `sql` must use `?` for every bind parameter, with one entry in
    /// `params` per `?`, in order.
    pub fn new(sql: String, params: Vec<Value>) -> Self {
        Fragment { sql, params }
    }

    /// Wraps the fragment in surrounding SQL, e.g. `EXISTS (`..`)`.
    pub(crate) fn enclosed_in(self, before: &str, after: &str) -> Self {
        Fragment {
            sql: format!("{before}{}{after}", self.sql),
            params: self.params,
        }
    }

    /// Appends this fragment to a query being rendered, renumbering its
    /// placeholders to continue `params`' sequence.
    pub(crate) fn splice_into<D: Dialect>(&self, out: &mut String, params: &mut Vec<Value>) {
        let mut next = self.params.iter();
        for c in self.sql.chars() {
            if c == '?' {
                params.push(next.next().expect("one param per `?`").clone());
                out.push_str(&D::placeholder(params.len()));
            } else {
                out.push(c);
            }
        }
    }
}

fn push_ident<D: Dialect>(out: &mut String, ident: &str) {
    out.push(D::IDENTIFIER_QUOTE);
    out.push_str(ident);
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
pub fn render_ident<D: Dialect>(out: &mut String, ident: &str) {
    push_ident::<D>(out, ident);
}
