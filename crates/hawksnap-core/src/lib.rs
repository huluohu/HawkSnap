pub mod capture;
pub mod crop;
pub mod geometry;
pub mod naming;
pub mod stitch;

#[cfg(test)]
mod smoke_tests {
    #[test]
    fn workspace_builds() { assert_eq!(1 + 1, 2); }
}
