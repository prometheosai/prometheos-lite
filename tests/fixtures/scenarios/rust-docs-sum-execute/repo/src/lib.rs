//! Pinned #136 Slice C evaluation fixture. The implementation is CORRECT;
//! the README documents the wrong behaviour. Do not "fix" this file in
//! place - the content digest is pinned by the scenario descriptor.

pub fn sum(a: i64, b: i64) -> i64 {
    a + b
}
