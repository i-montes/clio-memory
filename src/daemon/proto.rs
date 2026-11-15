//! The daemon's protocol: one JSON line from the client, one JSON line back.

/// The protocol this binary speaks; a request in another one is refused.
pub const PROTOCOL: u32 = 1;
/// The longest request line, in bytes.
pub const MAX_LINE: usize = 1 << 20;

/// Whether version `a` (`X.Y.Z`) is newer than `b`; `false` when either is not understood.
pub fn newer(a: &str, b: &str) -> bool {
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

fn parse(v: &str) -> Option<[u64; 3]> {
    let mut parts = v.split('.').map(|p| p.parse().ok());
    let out = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn versions_compare_by_number_and_garbage_is_never_newer() {
        assert!(newer("0.10.1", "0.10.0"));
        assert!(newer("1.0.0", "0.99.9"));
        assert!(newer("0.10.0", "0.9.1"));
        assert!(!newer("0.10.0", "0.10.0"));
        assert!(!newer("0.9.1", "0.10.0"));
        assert!(!newer("x", "0.1.0"));
        assert!(!newer("0.1.0", "x"));
        assert!(!newer("1.0", "0.1.0"));
        assert!(!newer("1.0.0.0", "0.1.0"));
    }
}
