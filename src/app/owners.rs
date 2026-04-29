use users::{Users, UsersCache};

pub trait OwnerNameResolver {
    fn owner_name(&self, uid: u32) -> String;
}

/// Resolves process owner UIDs through the system user database.
pub struct OwnerNameCache {
    cache: UsersCache,
}

impl OwnerNameCache {
    pub(super) fn new() -> Self {
        Self {
            cache: UsersCache::new(),
        }
    }
}

impl Default for OwnerNameCache {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerNameResolver for OwnerNameCache {
    fn owner_name(&self, uid: u32) -> String {
        self.cache
            .get_user_by_uid(uid)
            .map(|user| user.name().to_string_lossy().into_owned())
            .unwrap_or_else(|| uid.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnerNameCache, OwnerNameResolver};
    use users::{get_current_uid, get_user_by_uid};

    #[test]
    fn resolves_current_uid_from_users_database() {
        let resolver = OwnerNameCache::new();
        let uid = get_current_uid();
        let expected = get_user_by_uid(uid)
            .map(|user| user.name().to_string_lossy().into_owned())
            .unwrap_or_else(|| uid.to_string());

        assert_eq!(resolver.owner_name(uid), expected);
    }

    #[test]
    fn falls_back_to_numeric_uid_when_user_is_missing() {
        let resolver = OwnerNameCache::new();
        let missing_uid = u32::MAX;

        assert_eq!(resolver.owner_name(missing_uid), missing_uid.to_string());
    }
}
