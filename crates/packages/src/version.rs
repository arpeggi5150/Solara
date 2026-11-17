//! Comparing package versions such as `1.0.1` or `0.2.0-beta.1`.

use std::cmp::Ordering;

/// Whether version `a` is newer than `b`: dot-separated numbers compared as numbers (a missing
/// part counts as 0), then a release beats its pre-release (`1.0.0` > `1.0.0-rc.1`), then the
/// pre-release tags compare as text. A non-numeric part compares as text.
pub fn is_newer(a: &str, b: &str) -> bool {
    compare(a, b) == Ordering::Greater
}

fn compare(a: &str, b: &str) -> Ordering {
    let (a_core, a_pre) = split(a);
    let (b_core, b_pre) = split(b);
    let (a_parts, b_parts): (Vec<&str>, Vec<&str>) = (a_core.split('.').collect(), b_core.split('.').collect());
    for i in 0..a_parts.len().max(b_parts.len()) {
        let x = a_parts.get(i).copied().unwrap_or("0");
        let y = b_parts.get(i).copied().unwrap_or("0");
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

fn split(v: &str) -> (&str, Option<&str>) {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split_once('+').map_or(v, |(core, _)| core);
    match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_versions() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(is_newer("2", "1.9"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_newer("1.0.0-rc.2", "1.0.0-rc.1"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0+build.5"));
    }
}
