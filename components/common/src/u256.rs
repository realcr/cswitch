use uint::construct_uint;

construct_uint! {
    /// A 256 bit unsigned int
    pub struct U256(4);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u256_basic() {
        let mut num = U256::from(1);
        for _ in 0..255 {
            num <<= 1;
            assert!(!num.is_zero());
        }
        num <<= 1;
        assert!(num.is_zero());
    }
}
