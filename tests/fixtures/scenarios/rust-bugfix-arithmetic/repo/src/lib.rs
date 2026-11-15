//! Pinned #136 Slice A evaluation fixture. Intentionally broken: `sum`
//! subtracts instead of adding. Do not "fix" this file in place - the
//! content digest is pinned by the scenario descriptor.

pub fn sum(a: i64, b: i64) -> i64 {
    a - b
}

#[cfg(test)]
mod tests {
    use super::sum;

    #[test]
    fn sums_two_positive_numbers() {
        assert_eq!(sum(2, 2), 4);
    }
}
