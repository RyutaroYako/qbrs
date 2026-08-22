//! Rendering `ExprKind` into a SQL string plus a positional parameter list.
//! Generic over `D: Dialect` for identifier quoting and placeholder style,
//! but `ExprKind`/`Value` themselves stay closed, non-generic types, so this
//! costs one instantiation per dialect used in a program rather than one per
//! query shape.

use crate::dialect::Dialect;
use crate::expr::{BinOp, CastTarget, ExprKind, SortDir, Value};

/// Where rendered SQL goes. A bind parameter is *told* to the sink rather
/// than written as text, which is what lets the same renderer produce either
/// a finished statement or a `Fragment` whose parameters aren't numbered
/// yet — with no character standing in for one, and so nothing to escape.
pub(crate) trait Sink {
    fn text(&mut self, s: &str);
    fn ch(&mut self, c: char);
    fn bind(&mut self, value: &Value);
}

/// Builds a finished statement, numbering each parameter as it arrives.
pub(crate) struct QuerySink<D> {
    sql: String,
    params: Vec<Value>,
    _dialect: std::marker::PhantomData<fn() -> D>,
}

impl<D: Dialect> QuerySink<D> {
    pub(crate) fn new() -> Self {
        QuerySink {
            sql: String::new(),
            params: Vec::new(),
            _dialect: std::marker::PhantomData,
        }
    }

    pub(crate) fn finish(self) -> (String, Vec<Value>) {
        (self.sql, self.params)
    }
}

impl<D: Dialect> Sink for QuerySink<D> {
    fn text(&mut self, s: &str) {
        self.sql.push_str(s);
    }
    fn ch(&mut self, c: char) {
        self.sql.push(c);
    }
    fn bind(&mut self, value: &Value) {
        self.params.push(value.clone());
        self.sql.push_str(&D::placeholder(self.params.len()));
    }
}

/// Builds a `Fragment`: a parameter starts a new segment instead of being
/// written, so its eventual number is decided by whoever splices it.
pub(crate) struct FragmentSink {
    segments: Vec<String>,
    params: Vec<Value>,
}

impl FragmentSink {
    pub(crate) fn new() -> Self {
        FragmentSink {
            segments: vec![String::new()],
            params: Vec::new(),
        }
    }

    pub(crate) fn finish(self) -> Fragment {
        Fragment::new(self.segments, self.params)
    }

    fn last(&mut self) -> &mut String {
        self.segments.last_mut().expect("one segment to start")
    }
}

impl Sink for FragmentSink {
    fn text(&mut self, s: &str) {
        self.last().push_str(s);
    }
    fn ch(&mut self, c: char) {
        self.last().push(c);
    }
    fn bind(&mut self, value: &Value) {
        self.params.push(value.clone());
        self.segments.push(String::new());
    }
}

pub(crate) fn render_expr<D: Dialect>(expr: &ExprKind, sink: &mut dyn Sink) {
    match expr {
        ExprKind::Column { table, name } => {
            push_ident::<D>(sink, table);
            sink.ch('.');
            push_ident::<D>(sink, name);
        }
        ExprKind::Value(v) => sink.bind(v),
        ExprKind::BinOp { op, lhs, rhs } => {
            sink.ch('(');
            render_expr::<D>(lhs, sink);
            sink.text(match op {
                BinOp::Eq => " = ",
                BinOp::Ne => " <> ",
                BinOp::Lt => " < ",
                BinOp::Lte => " <= ",
                BinOp::Gt => " > ",
                BinOp::Gte => " >= ",
                BinOp::Like => " LIKE ",
            });
            render_expr::<D>(rhs, sink);
            sink.ch(')');
        }
        ExprKind::And(parts) => render_bool_list::<D>(parts, "AND", sink),
        ExprKind::Or(parts) => render_bool_list::<D>(parts, "OR", sink),
        ExprKind::Not(inner) => {
            sink.text("(NOT ");
            render_expr::<D>(inner, sink);
            sink.ch(')');
        }
        ExprKind::Cast { expr, target } => {
            sink.text("CAST(");
            render_expr::<D>(expr, sink);
            sink.text(" AS ");
            sink.text(match target {
                CastTarget::BigInt => D::CAST_BIGINT,
                CastTarget::Double => D::CAST_DOUBLE,
            });
            sink.ch(')');
        }
        ExprKind::Func { name, arg } => {
            sink.text(name);
            sink.ch('(');
            render_expr::<D>(arg, sink);
            sink.ch(')');
        }
        ExprKind::IsNull { expr, negated } => {
            sink.ch('(');
            render_expr::<D>(expr, sink);
            sink.text(if *negated {
                " IS NOT NULL)"
            } else {
                " IS NULL)"
            });
        }
        ExprKind::InList { expr, values } => {
            if values.is_empty() {
                sink.text("FALSE");
                return;
            }
            sink.ch('(');
            render_expr::<D>(expr, sink);
            sink.text(" IN (");
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    sink.text(", ");
                }
                render_expr::<D>(v, sink);
            }
            sink.text("))");
        }
        ExprKind::Raw(fragment) => {
            // A fragment's internal precedence is unknown (it may be `a OR
            // b`), so parenthesize: it must not change meaning when spliced
            // into a larger AND/OR chain.
            sink.ch('(');
            fragment.splice_into(sink);
            sink.ch(')');
        }
        ExprKind::Window {
            func,
            partition_by,
            order_by,
        } => {
            // No defensive parens here, unlike `Raw`: `OVER` only attaches
            // to a bare function-call syntax node, so `(row_number()) OVER
            // (..)` would not be valid SQL.
            sink.text(func);
            sink.text(" OVER (");
            if !partition_by.is_empty() {
                sink.text("PARTITION BY ");
                for (i, p) in partition_by.iter().enumerate() {
                    if i > 0 {
                        sink.text(", ");
                    }
                    render_expr::<D>(p, sink);
                }
            }
            if !order_by.is_empty() {
                if !partition_by.is_empty() {
                    sink.ch(' ');
                }
                sink.text("ORDER BY ");
                for (i, (e, dir)) in order_by.iter().enumerate() {
                    if i > 0 {
                        sink.text(", ");
                    }
                    render_expr::<D>(e, sink);
                    sink.text(match dir {
                        SortDir::Asc => " ASC",
                        SortDir::Desc => " DESC",
                    });
                }
            }
            sink.ch(')');
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
pub(crate) fn render_select_list<D: Dialect>(items: &[SelectItem], sink: &mut dyn Sink) {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_expr::<D>(&item.kind, sink);
        if let Some(label) = item.label {
            sink.text(" AS ");
            push_ident::<D>(sink, label);
        }
    }
}

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
    pub(crate) fn new(segments: Vec<String>, params: Vec<Value>) -> Self {
        debug_assert_eq!(segments.len(), params.len() + 1);
        Fragment { segments, params }
    }

    /// Splits `sql!{}`'s authored text on its `?` placeholders. `??` is a
    /// literal `?`.
    pub(crate) fn from_authored(sql: &'static str, params: Vec<Value>) -> Self {
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

    /// Appends this fragment to whatever is being rendered, handing each of
    /// its parameters to the sink in turn.
    pub(crate) fn splice_into(&self, sink: &mut dyn Sink) {
        for (i, segment) in self.segments.iter().enumerate() {
            sink.text(segment);
            if let Some(value) = self.params.get(i) {
                sink.bind(value);
            }
        }
    }
}

fn push_ident<D: Dialect>(sink: &mut dyn Sink, ident: &str) {
    sink.ch(D::IDENTIFIER_QUOTE);
    for c in ident.chars() {
        // A quote inside an identifier is escaped by doubling it, in every
        // dialect this crate speaks. `#[table(name = "..")]` takes an
        // arbitrary string, so an unescaped one would end the identifier.
        if c == D::IDENTIFIER_QUOTE {
            sink.ch(c);
        }
        sink.ch(c);
    }
    sink.ch(D::IDENTIFIER_QUOTE);
}

fn render_bool_list<D: Dialect>(parts: &[ExprKind], joiner: &str, sink: &mut dyn Sink) {
    if parts.is_empty() {
        // An empty AND/OR should never make it into a real query (the
        // builder never pushes one), but render a harmless tautology/
        // contradiction rather than emitting invalid SQL if it ever did.
        sink.text(if joiner == "AND" { "TRUE" } else { "FALSE" });
        return;
    }
    sink.ch('(');
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            sink.ch(' ');
            sink.text(joiner);
            sink.ch(' ');
        }
        render_expr::<D>(part, sink);
    }
    sink.ch(')');
}

/// Renders a bare identifier (a table name in a `FROM`/`INSERT INTO`/etc.
/// clause, not part of an `ExprKind`) with the dialect's quoting.
pub(crate) fn render_ident<D: Dialect>(sink: &mut dyn Sink, ident: &str) {
    push_ident::<D>(sink, ident);
}
