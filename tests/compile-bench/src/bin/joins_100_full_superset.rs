// At ~100 nested tables, Superset's recursive resolution (which walks the
// scope list once per required table, i.e. up to depth ~200 in the worst
// case) exceeds rustc's default trait-recursion limit of 128 — a purely
// mechanical ceiling, not evidence of exponential blowup (see the timing
// results this binary is used to produce). Real schemas joining 100 tables
// in one query are not a realistic case; this is deliberately over the
// top to stress-test the design's scaling behavior.
#![recursion_limit = "1024"]

use compile_bench::{
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39, T40, T41, T42, T43, T44, T45, T46, T47, T48, T49, T50, T51, T52, T53, T54, T55, T56,
    T57, T58, T59, T60, T61, T62, T63, T64, T65, T66, T67, T68, T69, T70, T71, T72, T73, T74, T75,
    T76, T77, T78, T79, T80, T81, T82, T83, T84, T85, T86, T87, T88, T89, T90, T91, T92, T93, T94,
    T95, T96, T97, T98, T99,
};
use compile_bench::{assert_contains, assert_map_nullable, assert_superset, req_of, scope_of};

type Scope = scope_of!(
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39, T40, T41, T42, T43, T44, T45, T46, T47, T48, T49, T50, T51, T52, T53, T54, T55, T56,
    T57, T58, T59, T60, T61, T62, T63, T64, T65, T66, T67, T68, T69, T70, T71, T72, T73, T74, T75,
    T76, T77, T78, T79, T80, T81, T82, T83, T84, T85, T86, T87, T88, T89, T90, T91, T92, T93, T94,
    T95, T96, T97, T98, T99
);
type FullReq = req_of!(
    T00, T01, T02, T03, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16, T17, T18,
    T19, T20, T21, T22, T23, T24, T25, T26, T27, T28, T29, T30, T31, T32, T33, T34, T35, T36, T37,
    T38, T39, T40, T41, T42, T43, T44, T45, T46, T47, T48, T49, T50, T51, T52, T53, T54, T55, T56,
    T57, T58, T59, T60, T61, T62, T63, T64, T65, T66, T67, T68, T69, T70, T71, T72, T73, T74, T75,
    T76, T77, T78, T79, T80, T81, T82, T83, T84, T85, T86, T87, T88, T89, T90, T91, T92, T93, T94,
    T95, T96, T97, T98, T99
);

fn main() {
    assert_contains::<Scope, T00, _>(); // worst case for Find: last in the list, deepest recursion
    assert_contains::<Scope, T99, _>(); // best case: first in the list (Here)
    // Worst case for Superset: requires ALL 100 tables, each independently
    // walking the scope list -> O(n^2) Find resolutions if merely polynomial.
    assert_superset::<Scope, FullReq, _>();
    assert_map_nullable::<Scope>();
}
