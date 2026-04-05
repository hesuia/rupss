use users::{Users, UsersCache};

pub trait OwnerNameResolver {
    fn owner_name(&self, uid: u32) -> String;
}

/// Resolves process owner UIDs through the system user database.
pub struct OwnerNameResolverImpl {
    cache: UsersCache,
}

impl OwnerNameResolverImpl {
    pub(super) fn new() -> Self {
        Self {
            cache: UsersCache::new(),
        }
    }

    pub(super) fn owner_name(&self, uid: u32) -> String {
        self.lookup_owner_name(uid)
            .unwrap_or_else(|| uid.to_string())
    }

    fn lookup_owner_name(&self, uid: u32) -> Option<String> {
        self.cache
            .get_user_by_uid(uid)
            .map(|user| user.name().to_string_lossy().into_owned())
    }
}

impl Default for OwnerNameResolverImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl OwnerNameResolver for OwnerNameResolverImpl {
    fn owner_name(&self, uid: u32) -> String {
        self.lookup_owner_name(uid)
            .unwrap_or_else(|| uid.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::OwnerNameResolverImpl;
    use users::{get_current_uid, get_user_by_uid};

    #[test]
    fn resolves_current_uid_from_users_database() {
        let resolver = OwnerNameResolverImpl::new();
        let uid = get_current_uid();
        let expected = get_user_by_uid(uid)
            .map(|user| user.name().to_string_lossy().into_owned())
            .unwrap_or_else(|| uid.to_string());

        assert_eq!(resolver.owner_name(uid), expected);
    }

    #[test]
    fn falls_back_to_numeric_uid_when_user_is_missing() {
        let resolver = OwnerNameResolverImpl::new();
        let missing_uid = u32::MAX;

        assert_eq!(resolver.owner_name(missing_uid), missing_uid.to_string());
    }
}
