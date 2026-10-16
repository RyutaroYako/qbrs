//! Rendering `ExprKind` into a SQL string plus a positional parameter list.
//! Generic over `D: Dialect` for identifier quoting and placeholder style,
//! but `ExprKind`/`Value` themselves stay closed, non-generic types, so this
//! costs one instantiation per dialect used in a program rather than one per
//! query shape.

use crate::dialect::{Dialect, StringAggSyntax};
use crate::expr::{BinOp, CastTarget, ExprKind, SortDir, Value};

/// `#[doc(hidden)] pub` rather than private, because
/// [`Statement::render_into`](crate::statement::Statement::render_into)
/// names it. Sealed rather than open: a bind is *told* to a sink so that no
/// character in the text ever stands in for one, and a foreign sink could
/// write it into the text anyway.
#[doc(hidden)]
pub trait Sink: sink::Sealed {
    fn text(&mut self, s: &str);
    fn ch(&mut self, c: char);
    fn bind(&mut self, value: &Value);
}

mod sink {
    pub trait Sealed {}
}

/// Keyed by hash and confirmed with `Value::binds_same_as` rather than a
/// `HashMap<Value, _>`: `Value` has no `Eq`, and `==` is too coarse to share
/// a parameter on. A dialect with positional placeholders cannot name a
/// parameter twice, so its `QuerySink` holds `None` rather than an index it
/// never reads.
struct BoundValues {
    by_key: std::collections::HashMap<u64, Vec<usize>>,
}

impl BoundValues {
    fn new() -> Self {
        BoundValues {
            by_key: std::collections::HashMap::new(),
        }
    }

    fn key(&self, value: &Value) -> u64 {
        use std::hash::{BuildHasher as _, Hasher as _};
        let mut hasher = self.by_key.hasher().build_hasher();
        value.hash_into(&mut hasher);
        hasher.finish()
    }

    fn existing_placeholder_number(&self, params: &[Value], value: &Value) -> Option<usize> {
        self.by_key
            .get(&self.key(value))?
            .iter()
            .find(|&&i| params[i].binds_same_as(value))
            .map(|&i| i + 1)
    }

    fn record(&mut self, index: usize, value: &Value) {
        let key = self.key(value);
        self.by_key.entry(key).or_default().push(index);
    }
}

/// Names an already-bound value again rather than binding it afresh, where
/// the dialect numbers its placeholders: a `sql!{}` fragment with binds then
/// renders the same text everywhere it recurs, which Postgres's syntactic
/// `GROUP BY` matching requires.
#[doc(hidden)]
pub struct QuerySink<D> {
    sql: String,
    params: Vec<Value>,
    bound: Option<BoundValues>,
    _dialect: std::marker::PhantomData<fn() -> D>,
}

impl<D: Dialect> QuerySink<D> {
    pub(crate) fn new() -> Self {
        QuerySink {
            sql: String::new(),
            params: Vec::new(),
            bound: D::PLACEHOLDERS_ARE_NUMBERED.then(BoundValues::new),
            _dialect: std::marker::PhantomData,
        }
    }

    pub(crate) fn finish(self) -> (String, Vec<Value>) {
        (self.sql, self.params)
    }

    fn new_placeholder_number(&mut self, value: &Value) -> usize {
        self.params.push(value.clone());
        if let Some(bound) = &mut self.bound {
            bound.record(self.params.len() - 1, value);
        }
        self.params.len()
    }
}

impl<D> sink::Sealed for QuerySink<D> {}

impl<D: Dialect> Sink for QuerySink<D> {
    fn text(&mut self, s: &str) {
        self.sql.push_str(s);
    }
    fn ch(&mut self, c: char) {
        self.sql.push(c);
    }
    fn bind(&mut self, value: &Value) {
        let already = self
            .bound
            .as_ref()
            .and_then(|bound| bound.existing_placeholder_number(&self.params, value));
        let n = match already {
            Some(n) => n,
            None => self.new_placeholder_number(value),
        };
        D::write_placeholder(n, &mut self.sql);
    }
}

/// A bind starts a new segment rather than writing a placeholder: its number
/// is not known until the fragment is spliced.
pub(crate) struct FragmentSink(Fragment);

impl FragmentSink {
    pub(crate) fn new() -> Self {
        FragmentSink(Fragment::empty())
    }

    pub(crate) fn finish(self) -> Fragment {
        self.0
    }
}

impl sink::Sealed for FragmentSink {}

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
        ExprKind::Excluded { name } => {
            sink.text(crate::insert::EXCLUDED);
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
        ExprKind::StringAgg { arg, separator } => {
            let (func, escapes) = match D::STRING_AGG {
                StringAggSyntax::Argument(func) => (func, None),
                StringAggSyntax::SeparatorKeyword {
                    func,
                    backslash_escapes,
                } => (func, Some(backslash_escapes)),
            };
            sink.text(func);
            sink.ch('(');
            render_expr::<D>(arg, sink);
            match escapes {
                None => {
                    sink.text(", ");
                    sink.bind(&Value::Text((*separator).to_string()));
                }
                Some(backslash_escapes) => {
                    sink.text(" SEPARATOR ");
                    render_string_literal(sink, separator, backslash_escapes);
                }
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
        ExprKind::EqAny { expr, array } => {
            sink.ch('(');
            render_expr::<D>(expr, sink);
            sink.text(" = ANY(");
            render_expr::<D>(array, sink);
            sink.text("))");
        }
        ExprKind::Exists {
            body,
            selection,
            negated,
        } => {
            sink.text(if *negated {
                "(NOT EXISTS ("
            } else {
                "(EXISTS ("
            });
            body.render_into::<D>(selection, sink);
            sink.text("))");
        }
        ExprKind::InSubquery {
            lhs,
            body,
            selection,
            negated,
        } => {
            sink.ch('(');
            render_expr::<D>(lhs, sink);
            sink.text(if *negated { " NOT IN (" } else { " IN (" });
            body.render_into::<D>(selection, sink);
            sink.text("))");
        }
        ExprKind::Template { head, rest } => {
            // Parenthesised even where it looks redundant: authored text has
            // no precedence the renderer knows about.
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
            // No parentheses around `func`, unlike `Template`: `OVER` attaches
            // only to a bare function call, so `(row_number()) OVER (..)` is
            // not valid SQL.
            sink.text(func);
            sink.text(" OVER (");
            render_expr_list::<D>(sink, "PARTITION BY ", partition_by);
            let keyword = if partition_by.is_empty() {
                "ORDER BY "
            } else {
                " ORDER BY "
            };
            render_order_by::<D>(sink, keyword, order_by);
            sink.ch(')');
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
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

/// Held as the text between its binds rather than as one string with a
/// marker character per bind: a marker would need escaping, and re-splicing
/// a spliced fragment could mistake text for a bind.
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

    fn tail(&mut self) -> &mut String {
        match self.rest.last_mut() {
            Some((_, text)) => text,
            None => &mut self.head,
        }
    }

    pub(crate) fn splice_into(&self, sink: &mut dyn Sink) {
        sink.text(&self.head);
        for (value, text) in &self.rest {
            sink.bind(value);
            sink.text(text);
        }
    }
}

/// Splits on `.` rather than quoting the whole name: `analytics.events`
/// (which only `#[table(name = "..")]` can spell) is a table in a schema, and
/// quoted whole it asks for a relation with a dot in its name.
pub(crate) fn render_ident<D: Dialect>(sink: &mut dyn Sink, ident: &str) {
    for (i, part) in ident.split('.').enumerate() {
        if i > 0 {
            sink.ch('.');
        }
        render_ident_part::<D>(sink, part);
    }
}

/// Writes the value into SQL rather than binding it, which only MySQL's
/// `SEPARATOR` needs: its grammar refuses a parameter there. A backslash is
/// doubled too where the dialect reads one as an escape, since a lone
/// trailing one would carry the closing quote away.
fn render_string_literal(sink: &mut dyn Sink, text: &str, backslash_escapes: bool) {
    sink.ch('\'');
    for c in text.chars() {
        if c == '\'' || (backslash_escapes && c == '\\') {
            sink.ch(c);
        }
        sink.ch(c);
    }
    sink.ch('\'');
}

fn render_ident_part<D: Dialect>(sink: &mut dyn Sink, ident: &str) {
    sink.ch(D::IDENTIFIER_QUOTE);
    for c in ident.chars() {
        // Doubled rather than passed through: `#[table(name = "..")]` takes
        // an arbitrary string, and an unescaped quote would end the
        // identifier.
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

/// Its own function rather than inlined into `render_order_by`: a set
/// operation orders by ordinal position and cannot go through that.
pub(crate) fn dir_keyword(dir: SortDir) -> &'static str {
    match dir {
        SortDir::Asc => " ASC",
        SortDir::Desc => " DESC",
    }
}

pub(crate) fn render_count_wrapped<D: Dialect>(
    sink: &mut QuerySink<D>,
    body: impl FnOnce(&mut QuerySink<D>),
) {
    sink.text("SELECT count(*) FROM (");
    body(sink);
    sink.text(") AS ");
    render_ident::<D>(sink, "qbrs_total");
}

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
