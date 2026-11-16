//! Pinned #136 Slice C doc-check for the documentation fixture. It checks
//! the *documentation content*, not the implementation: the README must
//! describe `sum` truthfully (`a + b`, `sum(2, 2) == 4`) and must not claim
//! the faulty behaviour (`a - b`, `sum(2, 2) == 0`).

const README: &str = include_str!("../README.md");

#[test]
fn readme_documents_sum_truthfully() {
    assert!(
        README.contains("a + b"),
        "README must state that sum returns a + b"
    );
    assert!(
        README.contains("sum(2, 2) == 4"),
        "README must state sum(2, 2) == 4"
    );
    assert!(
        !README.contains("a - b"),
        "README must not claim sum returns a - b"
    );
    assert!(
        !README.contains("sum(2, 2) == 0"),
        "README must not claim sum(2, 2) == 0"
    );
}
