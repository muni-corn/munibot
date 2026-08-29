use dioxus::prelude::*;

use crate::chat::{ChatResult, UsageBreakdown, UsageSummary};

/// The signed-in user's own usage: all-time totals, and (if a per-user
/// spend cap is configured) their current spend against it.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    ai: axum::extract::Extension<Option<std::sync::Arc<munibot_ai::Ai>>>,
)]
pub async fn get_my_usage() -> ChatResult<UsageSummary> {
    use munibot_ai::limits::Scope;
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    let user = auth.current_user.clone().ok_or(ChatError::NotSignedIn)?;
    let ai_service = ai.0.clone().ok_or(ChatError::AiDisabled)?;

    let totals = ai::sum_usage_for_user(&pool, user.id).await?;
    let spend_cap = ai_service
        .spend_cap_status(Scope::User(user.id as u64))
        .await
        .map(Into::into);

    Ok(UsageSummary {
        totals: totals.into(),
        spend_cap,
    })
}

/// Service-wide usage: all-time totals across every user, and (if a global
/// spend cap is configured) the current spend against it.
///
/// Requires `Permission::Operator` - showing people their own cost (see
/// `get_my_usage`) is honest and a cheap abuse deterrent, but the whole
/// service's spend is not everyone's business.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
    ai: axum::extract::Extension<Option<std::sync::Arc<munibot_ai::Ai>>>,
)]
pub async fn get_global_usage() -> ChatResult<UsageSummary> {
    use munibot_ai::limits::Scope;
    use munibot_core::db::operations::ai;

    use crate::chat::ChatError;

    crate::auth::operator::require_operator(&auth).await?;
    let ai_service = ai.0.clone().ok_or(ChatError::AiDisabled)?;

    let totals = ai::sum_usage_global(&pool).await?;
    let spend_cap = ai_service
        .spend_cap_status(Scope::Global)
        .await
        .map(Into::into);

    Ok(UsageSummary {
        totals: totals.into(),
        spend_cap,
    })
}

/// Global usage broken down by persona, model, user, and day - the view
/// that catches a problem across everyone at once, alongside
/// `get_global_usage`'s own flat totals.
///
/// A trailing 30-day window for the daily breakdown: long enough to see a
/// trend, short enough that the query never has to scan a month-over-month
/// growing table's full history to answer it.
#[server(
    auth: crate::auth::server::AuthSession,
    pool: axum::extract::Extension<munibot_core::db::DbPool>,
)]
pub async fn get_usage_breakdown() -> ChatResult<UsageBreakdown> {
    use munibot_core::db::operations::ai;

    const DAILY_WINDOW_DAYS: i64 = 30;

    crate::auth::operator::require_operator(&auth).await?;

    let by_persona = ai::sum_usage_by_persona(&pool).await?;
    let by_model = ai::sum_usage_by_model(&pool).await?;
    let by_user = ai::sum_usage_by_user(&pool).await?;
    let daily = ai::sum_usage_daily(&pool, DAILY_WINDOW_DAYS).await?;

    Ok(UsageBreakdown::assemble(
        by_persona, by_model, by_user, daily,
    ))
}
