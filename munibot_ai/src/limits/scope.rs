/// A rate limit or spend cap's scope: a signed-in user, a Discord guild, or
/// the whole service.
///
/// One mechanism serves every level rather than three separate ones - see
/// `ai_rate_limits`/`ai_spend_caps`'s own migration comment, which this
/// mirrors on the database side (`scope_type` plus a nullable `scope_id`).
/// Directly usable as a `HashMap` key (derives `Hash`/`Eq`) for in-memory
/// concurrency tracking, which needs no database round trip at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    User(u64),
    Guild(u64),
    Global,
}

impl Scope {
    /// The stable string this scope's kind is stored as in the database.
    pub fn scope_type(&self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Guild(_) => "guild",
            Self::Global => "global",
        }
    }

    /// The database's `scope_id` column value: `None` for [`Scope::Global`],
    /// the id otherwise.
    pub fn scope_id(&self) -> Option<i64> {
        match self {
            Self::User(id) | Self::Guild(id) => Some(*id as i64),
            Self::Global => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_type_names_match_the_database_convention() {
        assert_eq!(Scope::User(1).scope_type(), "user");
        assert_eq!(Scope::Guild(1).scope_type(), "guild");
        assert_eq!(Scope::Global.scope_type(), "global");
    }

    #[test]
    fn test_only_global_has_no_scope_id() {
        assert_eq!(Scope::User(7).scope_id(), Some(7));
        assert_eq!(Scope::Guild(7).scope_id(), Some(7));
        assert_eq!(Scope::Global.scope_id(), None);
    }

    #[test]
    fn test_scope_is_usable_as_a_hashmap_key() {
        use std::collections::HashMap;
        let mut map: HashMap<Scope, u32> = HashMap::new();
        map.insert(Scope::User(1), 1);
        map.insert(Scope::User(2), 2);
        map.insert(Scope::Global, 3);
        assert_eq!(map[&Scope::User(1)], 1);
        assert_eq!(map[&Scope::User(2)], 2);
        assert_eq!(map[&Scope::Global], 3);
    }
}
