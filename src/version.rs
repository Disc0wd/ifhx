//! Just enough version comparison for `vMAJOR.MINOR.PATCH` tags.

fn parse(v: &str) -> Option<Vec<u64>> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

/// Is `latest` newer than `current`? Tags that are not plain numeric versions (a source build
/// recorded as "main") count as different, so a real release replaces them.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => latest.trim() != current.trim(),
    }
}

/// The tag as shown to people: no leading `v`.
pub fn display(tag: &str) -> &str {
    tag.trim().trim_start_matches('v')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numerically_not_lexically() {
        assert!(is_newer("v0.10.0", "v0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.2.0"));
        assert!(is_newer("0.1.1", "0.1"));
        assert!(!is_newer("0.1.0", "0.1"));
    }

    #[test]
    fn prerelease_suffix_is_ignored_and_odd_tags_differ() {
        assert!(!is_newer("v1.0.0-rc1", "1.0.0"));
        assert!(is_newer("v0.1.0", "main"));
        assert!(!is_newer("main", "main"));
        assert_eq!(display("v1.2.3"), "1.2.3");
    }
}
