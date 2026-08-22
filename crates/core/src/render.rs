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

/// The character `RawEmbed` emits where a bind parameter will go, used only
/// between rendering a nested query and splitting it into segments. A
/// `sql!{}` literal may not contain one, which `from_authored` checks.
pub(crate) const BIND_MARKER: char = '\u{1}';

/// A piece of SQL destined to be embedded in a larger query: a subquery, a
/// CTE body, a set-operation branch, or a `sql!{}` escape hatch. Held as the
/// text *between* its bind parameters, so a parameter is a position rather
/// than a character — nothing has to be escaped, and re-splicing an already
/// spliced fragment can't confuse the two.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// One more segment than there are params: `s0 ? s1 ? s2`.
    segments: Vec<String>,
    params: Vec<Value>,
}

impl Fragment {
    /// One segment per gap, one param per boundary.
    pub(crate) fn new(segments: Vec<String>, params: Vec<Value>) -> Self {
        debug_assert_eq!(segments.len(), params.len() + 1);
        Fragment { segments, params }
    }

    /// Splits a nested query's rendered text on the markers `RawEmbed` left
    /// where its parameters go.
    pub(crate) fn from_rendered(sql: &str, params: Vec<Value>) -> Self {
        let segments: Vec<String> = sql.split(BIND_MARKER).map(str::to_string).collect();
        Fragment::new(segments, params)
    }

    /// Splits `sql!{}`'s authored text on its `?` placeholders. `??` is a
    /// literal `?`.
    pub(crate) fn from_authored(sql: &'static str, params: Vec<Value>) -> Self {
        assert!(
            !sql.contains(BIND_MARKER),
            "`sql!` text may not contain U+0001"
        );
        let mut segments = vec![String::new()];
        let mut chars = sql.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '?' {
                segments.last_mut().expect("one segment to start").push(c);
                continue;
            }
            if chars.peek() == Some(&'?') {
                chars.next();
                segments.last_mut().expect("one segment to start").push('?');
            } else {
                segments.push(String::new());
            }
        }
        assert_eq!(
            segments.len() - 1,
            params.len(),
            "`sql!` has {} `?` placeholders but was given {} values (write `??` for a literal `?`)",
            segments.len() - 1,
            params.len()
        );
        Fragment { segments, params }
    }

    /// Wraps the fragment in surrounding SQL, e.g. `EXISTS (`..`)`.
    pub(crate) fn enclosed_in(mut self, before: &str, after: &str) -> Self {
        self.segments
            .first_mut()
            .expect("at least one segment")
            .insert_str(0, before);
        self.segments
            .last_mut()
            .expect("at least one segment")
            .push_str(after);
        self
    }

    /// Appends this fragment to a query being rendered, numbering its
    /// parameters into `params`' sequence.
    pub(crate) fn splice_into<D: Dialect>(&self, out: &mut String, params: &mut Vec<Value>) {
        for (i, segment) in self.segments.iter().enumerate() {
            out.push_str(segment);
            if let Some(value) = self.params.get(i) {
                params.push(value.clone());
                out.push_str(&D::placeholder(params.len()));
            }
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
