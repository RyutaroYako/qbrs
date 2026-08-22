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
#[doc(hidden)]
pub struct QuerySink<D> {
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
        D::write_placeholder(self.params.len(), &mut self.sql);
    }
}

/// Builds a `Fragment`: a parameter starts a new segment instead of being
/// written, so its eventual number is decided by whoever splices it.
pub(crate) struct FragmentSink(Fragment);

impl FragmentSink {
    pub(crate) fn new() -> Self {
        FragmentSink(Fragment::empty())
    }

    pub(crate) fn finish(self) -> Fragment {
        self.0
    }
}

impl Sink for FragmentSink {
    fn text(&mut self, s: &str) {
        self.0.tail().push_str(s);
    }
    fn ch(&mut self, c: char) {
        self.0.tail().push(c);
    }
    fn bind(&mut self, value: &Value) {
        self.0.rest.push((value.clone(), String::new()));
    }
}

pub(crate) fn render_expr<D: Dialect>(expr: &ExprKind, sink: &mut dyn Sink) {
    match expr {
        ExprKind::Column { table, name } => {
            render_ident::<D>(sink, table);
            sink.ch('.');
            render_ident::<D>(sink, name);
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
        ExprKind::And(lhs, rhs) => render_bool_pair::<D>(lhs, "AND", rhs, sink),
        ExprKind::Or(lhs, rhs) => render_bool_pair::<D>(lhs, "OR", rhs, sink),
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
            match arg {
                Some(arg) => render_expr::<D>(arg, sink),
                None => sink.ch('*'),
            }
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
        ExprKind::Always(yes) => sink.text(if *yes { "TRUE" } else { "FALSE" }),
        ExprKind::InList { expr, values } => {
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
        ExprKind::Template { head, rest } => {
            // Same defensive parentheses as an embedded fragment: authored
            // text has no precedence the renderer knows about.
            sink.ch('(');
            sink.text(head);
            for (arg, text) in rest {
                render_expr::<D>(arg, sink);
                sink.text(text);
            }
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
            render_expr_list::<D>(sink, "PARTITION BY ", partition_by);
            let separator = if partition_by.is_empty() { "" } else { " " };
            render_order_by::<D>(sink, &format!("{separator}ORDER BY "), order_by);
            sink.ch(')');
        }
    }
}

#[doc(hidden)]
/// One item in a rendered `SELECT`/`RETURNING` list. `label` is `Some` only
/// for an item given a `expr::LabelKey` label, which is the only thing that
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
            render_ident::<D>(sink, label);
        }
    }
}

/// A piece of SQL destined to be embedded in a larger query: a subquery, a
/// CTE body, a set-operation branch. Held as the
/// text *between* its bind parameters — `head`, then one `(param, text)`
/// pair per parameter — so a parameter is a position rather than a
/// character: nothing has to be escaped, re-splicing an already-spliced
/// fragment can't confuse the two, and there is no way to hold a parameter
/// with no text on either side of it.
#[derive(Debug, Clone)]
pub(crate) struct Fragment {
    head: String,
    rest: Vec<(Value, String)>,
}

impl Fragment {
    fn empty() -> Self {
        Fragment {
            head: String::new(),
            rest: Vec::new(),
        }
    }

    /// Where the next text goes: after the last parameter, or in `head`
    /// while there are none.
    fn tail(&mut self) -> &mut String {
        match self.rest.last_mut() {
            Some((_, text)) => text,
            None => &mut self.head,
        }
    }

    /// Wraps the fragment in surrounding SQL, e.g. `EXISTS (`..`)`.
    pub(crate) fn enclosed_in(mut self, before: &str, after: &str) -> Self {
        self.head.insert_str(0, before);
        self.tail().push_str(after);
        self
    }

    /// Appends this fragment to whatever is being rendered, handing each of
    /// its parameters to the sink in turn.
    pub(crate) fn splice_into(&self, sink: &mut dyn Sink) {
        sink.text(&self.head);
        for (value, text) in &self.rest {
            sink.bind(value);
            sink.text(text);
        }
    }
}

/// Renders an identifier with the dialect's quoting.
pub(crate) fn render_ident<D: Dialect>(sink: &mut dyn Sink, ident: &str) {
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

fn render_bool_pair<D: Dialect>(lhs: &ExprKind, joiner: &str, rhs: &ExprKind, sink: &mut dyn Sink) {
    sink.ch('(');
    render_expr::<D>(lhs, sink);
    sink.ch(' ');
    sink.text(joiner);
    sink.ch(' ');
    render_expr::<D>(rhs, sink);
    sink.ch(')');
}

/// How a sort direction is spelled. Shared by a statement's `ORDER BY`, a
/// window's, and a set operation's — which orders by ordinal position and so
/// can't go through `render_order_by`.
pub(crate) fn dir_keyword(dir: SortDir) -> &'static str {
    match dir {
        SortDir::Asc => " ASC",
        SortDir::Desc => " DESC",
    }
}

/// A comma-separated expression list behind a keyword — `GROUP BY`,
/// `PARTITION BY` — or nothing at all when there are none.
pub(crate) fn render_expr_list<D: Dialect>(sink: &mut dyn Sink, keyword: &str, list: &[ExprKind]) {
    if list.is_empty() {
        return;
    }
    sink.text(keyword);
    for (i, e) in list.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_expr::<D>(e, sink);
    }
}

/// The same, with each key's sort direction — a statement's `ORDER BY` and a
/// window's `OVER (.. ORDER BY ..)` are one clause written in two places.
pub(crate) fn render_order_by<D: Dialect>(
    sink: &mut dyn Sink,
    keyword: &str,
    keys: &[(ExprKind, SortDir)],
) {
    if keys.is_empty() {
        return;
    }
    sink.text(keyword);
    for (i, (e, dir)) in keys.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_expr::<D>(e, sink);
        sink.text(dir_keyword(*dir));
    }
}

/// `WHERE`/`HAVING`: a keyword, then the conditions AND-folded, or nothing
/// at all when there are none. Shared by every statement that has such a
/// clause, so all four spell it the same way.
pub(crate) fn render_and_list<D: Dialect>(sink: &mut dyn Sink, keyword: &str, list: &[ExprKind]) {
    if list.is_empty() {
        return;
    }
    sink.text(keyword);
    for (i, e) in list.iter().enumerate() {
        if i > 0 {
            sink.text(" AND ");
        }
        render_expr::<D>(e, sink);
    }
}
