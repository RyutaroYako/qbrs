use compile_bench::{
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39,
};
use compile_bench::{assert_contains, assert_map_nullable, assert_superset, req_of, scope_of};

type Scope = scope_of!(
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39
);

fn main() {
    assert_contains::<Scope, T00, _>(); // worst case: last in the list, deepest recursion
    assert_contains::<Scope, T39, _>(); // best case: first in the list (Here)
    assert_superset::<Scope, req_of!(T01, T38), _>();
    assert_map_nullable::<Scope>();
}
