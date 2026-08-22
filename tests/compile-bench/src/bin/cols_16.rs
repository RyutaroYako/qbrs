//! Maximum-arity selection: 16 columns through `RowField` per element,
//! `Row` assembly, and a worst-case `Field` read at the far end of the
//! key list. The join-count binaries hold scope depth constant; this one
//! holds it at one table and grows the selection instead.

use compile_bench::T00;
use compile_bench::{assert_field, row_of};
use qbrs_core::dialect::Postgres;
use qbrs_core::select::select;

compile_bench::declare_columns!(
    T00, c00, c01, c02, c03, c04, c05, c06, c07, c08, c09, c10, c11, c12, c13, c14, c15,
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
);

fn main() {
    let (sql, _) = select((
        c00, c01, c02, c03, c04, c05, c06, c07, c08, c09, c10, c11, c12, c13, c14, c15,
    ))
    .from::<Postgres, _>(T00)
    .to_sql();
    assert!(sql.starts_with("SELECT "));

    assert_field::<Fields, columns::c00, _>(); // best case: head of the list
    assert_field::<Fields, columns::c15, _>(); // worst case: deepest `There<..>`
}
