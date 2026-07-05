//! UUIDv7 ids (spec §9): time-ordered, sync-safe, sortable as strings.

/// A fresh UUIDv7 as a lowercase hyphenated string.
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_is_a_uuid_v7() {
        let id = new_id();
        let parsed = uuid::Uuid::parse_str(&id).expect("valid uuid");
        assert_eq!(parsed.get_version_num(), 7);
    }

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert!(a <= b, "uuid v7 string order follows creation order");
    }
}
