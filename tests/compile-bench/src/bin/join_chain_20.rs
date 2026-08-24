//! A real builder chain at 20 joins: every `.inner_join()` is a call, every
//! `ON` predicate a checked expression, and the query is rendered. The
//! `joins_*` binaries measure `Find`/`Superset` over a hand-built scope
//! *type*; this one measures what a user actually pays — one
//! monomorphization of `Select` per join, plus a `Superset` obligation each.

use compile_bench::*;
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::ExprMethods;
use qbrs_core::select::select;

pub mod t00 {
    use super::*;
    compile_bench::declare_columns!(T00, id, ref_id);
}
pub mod t01 {
    use super::*;
    compile_bench::declare_columns!(T01, id, ref_id);
}
pub mod t02 {
    use super::*;
    compile_bench::declare_columns!(T02, id, ref_id);
}
pub mod t03 {
    use super::*;
    compile_bench::declare_columns!(T03, id, ref_id);
}
pub mod t04 {
    use super::*;
    compile_bench::declare_columns!(T04, id, ref_id);
}
pub mod t05 {
    use super::*;
    compile_bench::declare_columns!(T05, id, ref_id);
}
pub mod t06 {
    use super::*;
    compile_bench::declare_columns!(T06, id, ref_id);
}
pub mod t07 {
    use super::*;
    compile_bench::declare_columns!(T07, id, ref_id);
}
pub mod t08 {
    use super::*;
    compile_bench::declare_columns!(T08, id, ref_id);
}
pub mod t09 {
    use super::*;
    compile_bench::declare_columns!(T09, id, ref_id);
}
pub mod t10 {
    use super::*;
    compile_bench::declare_columns!(T10, id, ref_id);
}
pub mod t11 {
    use super::*;
    compile_bench::declare_columns!(T11, id, ref_id);
}
pub mod t12 {
    use super::*;
    compile_bench::declare_columns!(T12, id, ref_id);
}
pub mod t13 {
    use super::*;
    compile_bench::declare_columns!(T13, id, ref_id);
}
pub mod t14 {
    use super::*;
    compile_bench::declare_columns!(T14, id, ref_id);
}
pub mod t15 {
    use super::*;
    compile_bench::declare_columns!(T15, id, ref_id);
}
pub mod t16 {
    use super::*;
    compile_bench::declare_columns!(T16, id, ref_id);
}
pub mod t17 {
    use super::*;
    compile_bench::declare_columns!(T17, id, ref_id);
}
pub mod t18 {
    use super::*;
    compile_bench::declare_columns!(T18, id, ref_id);
}
pub mod t19 {
    use super::*;
    compile_bench::declare_columns!(T19, id, ref_id);
}

fn main() {
    let (sql, _) = select((t00::id, t19::id))
        .from(T00)
        .inner_join(T01, t01::ref_id.eq(t00::id))
        .inner_join(T02, t02::ref_id.eq(t01::id))
        .inner_join(T03, t03::ref_id.eq(t02::id))
        .inner_join(T04, t04::ref_id.eq(t03::id))
        .inner_join(T05, t05::ref_id.eq(t04::id))
        .inner_join(T06, t06::ref_id.eq(t05::id))
        .inner_join(T07, t07::ref_id.eq(t06::id))
        .inner_join(T08, t08::ref_id.eq(t07::id))
        .inner_join(T09, t09::ref_id.eq(t08::id))
        .inner_join(T10, t10::ref_id.eq(t09::id))
        .inner_join(T11, t11::ref_id.eq(t10::id))
        .inner_join(T12, t12::ref_id.eq(t11::id))
        .inner_join(T13, t13::ref_id.eq(t12::id))
        .inner_join(T14, t14::ref_id.eq(t13::id))
        .inner_join(T15, t15::ref_id.eq(t14::id))
        .inner_join(T16, t16::ref_id.eq(t15::id))
        .inner_join(T17, t17::ref_id.eq(t16::id))
        .inner_join(T18, t18::ref_id.eq(t17::id))
        .inner_join(T19, t19::ref_id.eq(t18::id))
        .filter(t00::id.gt(0i64))
        .to_sql(Postgres);
    assert!(sql.starts_with("SELECT "));
}
