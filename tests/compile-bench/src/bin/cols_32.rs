//! Maximum-arity selection: 32 columns through `RowField` per element,
//! `Row` assembly, and a worst-case `Field` read at the far end of the
//! key list. The join-count binaries hold scope depth constant; this one
//! holds it at one table and grows the selection instead.

use compile_bench::T00;
use compile_bench::{assert_field, row_of};
use qbrs_core::dialect::Postgres;
use qbrs_core::select::select;

compile_bench::declare_columns!(
    T00, c00, c01, c02, c03, c04, c05, c06, c07, c08, c09, c10, c11, c12, c13, c14, c15, c16, c17,
    c18, c19, c20, c21, c22, c23, c24, c25, c26, c27, c28, c29, c30, c31,
);

type Fields = row_of!(
    columns::c00,
    columns::c01,
    columns::c02,
    columns::c03,
    columns::c04,
    columns::c05,
    columns::c06,
    columns::c07,
    columns::c08,
    columns::c09,
    columns::c10,
    columns::c11,
    columns::c12,
    columns::c13,
    columns::c14,
    columns::c15,
    columns::c16,
    columns::c17,
    columns::c18,
    columns::c19,
    columns::c20,
    columns::c21,
    columns::c22,
    columns::c23,
    columns::c24,
    columns::c25,
    columns::c26,
    columns::c27,
    columns::c28,
    columns::c29,
    columns::c30,
    columns::c31,
);

fn main() {
    let (sql, _) = select((
        c00, c01, c02, c03, c04, c05, c06, c07, c08, c09, c10, c11, c12, c13, c14, c15, c16, c17,
        c18, c19, c20, c21, c22, c23, c24, c25, c26, c27, c28, c29, c30, c31,
    ))
    .from(T00)
    .to_sql(Postgres);
    assert!(sql.starts_with("SELECT "));

    assert_field::<Fields, columns::c00, _>(); // best case: head of the list
    assert_field::<Fields, columns::c31, _>(); // worst case: deepest `There<..>`
}
