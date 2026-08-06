use std::time::Duration;

/// The spend cap configured for one scope kind (every
/// [`crate::limits::Scope::User`] instance, say, regardless of which specific
/// user).
#[derive(Clone, Debug)]
pub struct SpendCapPolicy {
    /// `None` means no spend cap at this scope at all.
    pub limit_micros: Option<i64>,
    /// The period's own name, stored in `ai_spend_caps.period` - `"monthly"`
    /// by default, but any label an operator wants (`"daily"`, `"weekly"`)
    /// works the same way, since it is only ever compared against itself.
    pub period: String,
    /// How long a period lasts before it rolls over and starts counting
    /// from zero again.
    pub duration: Duration,
}

impl Default for SpendCapPolicy {
    /// No cap at all, with a 30-day period - a policy this permissive is
    /// only ever meaningful once `limit_micros` is actually set.
    fn default() -> Self {
        Self {
            limit_micros: None,
            period: "monthly".to_string(),
            duration: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }
}

/// The scope kinds spend caps actually check, together, for
/// [`crate::limits::SpendCapEnforcer::new`].
///
/// Per user and globally only, per the plan this was built from: there is
/// deliberately no `guild` field here, unlike
/// [`crate::limits::ScopePolicies`]'s rate limits - a guild's own members are
/// already covered individually by their own user-level caps, and the global
/// cap is the backstop above that.
#[derive(Clone, Debug, Default)]
pub struct SpendCapPolicies {
    pub user: SpendCapPolicy,
    pub global: SpendCapPolicy,
}
