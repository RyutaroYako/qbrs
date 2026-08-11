//! Rendering `ExprKind` (and, later, whole statements) into a SQL string
//! plus a positional parameter list. Generic over `D: Dialect` for
//! identifier quoting and placeholder style (Postgres: `"ident"` / `$N`;
//! MySQL: `` `ident` `` / `?`; SQLite: `"ident"` / `?`) — but `ExprKind`/
//! `Value` themselves stay closed, non-generic types, so monomorphizing
//! this function costs exactly one instantiation per dialect actually
//! used in a program, not per query shape, unlike diesel's
//! `QueryFragment::walk_ast` (itself generic over the query's type).

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
        ExprKind::Raw {
            text,
            params: raw_params,
        } => {
            // Wrapped in parens defensively — we don't know the fragment's
            // internal precedence (e.g. `a OR b`), so it must never
            // silently change meaning when spliced into a larger AND/OR
            // chain.
            out.push('(');
            splice_raw::<D>(text, raw_params, out, params);
            out.push(')');
        }
        ExprKind::Window {
            func,
            partition_by,
            order_by,
        } => {
            // `func` is rendered as a literal string, *not* wrapped in the
            // defensive parens `Raw` uses — `(row_number()) OVER (..)` is
            // not valid SQL (`OVER` only attaches to a bare function-call
            // syntax node, not an arbitrary parenthesized expression), so
            // this must stay `row_number() OVER (..)`.
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

/// Renumbers a `?`-placeholder fragment (already-rendered SQL text whose
/// bind values live in `raw_params`) into the surrounding query's own
/// placeholder style and shared `params` list — a no-op renumbering for
/// MySQL/SQLite, since `?` is also their native style; for Postgres this is
/// what turns a fragment's independently-numbered placeholders into the
/// right continuation of the outer query's `$N` sequence. Shared by
/// `ExprKind::Raw` (subqueries embedded as a boolean/scalar expression, e.g.
/// `EXISTS (..)`) and `select::SetOp` (whole `SELECT` branches joined by
/// `UNION`/`INTERSECT`/`EXCEPT`) — both need the same "render each piece
/// once with placeholder-agnostic `?`, then renumber at splice time"
/// mechanism, since a branch's own placeholder count isn't known until
/// every other branch preceding it has already been spliced in.
pub(crate) fn splice_raw<D: Dialect>(
    text: &str,
    raw_params: &[Value],
    out: &mut String,
    params: &mut Vec<Value>,
) {
    let mut raw_idx = 0;
    for c in text.chars() {
        if c == '?' {
            params.push(raw_params[raw_idx].clone());
            out.push_str(&D::placeholder(params.len()));
            raw_idx += 1;
        } else {
            out.push(c);
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
