pub mod geometry;
pub mod naming;

#[cfg(test)]
mod smoke_tests {
    #[test]
    fn workspace_builds() { assert_eq!(1 + 1, 2); }
}
