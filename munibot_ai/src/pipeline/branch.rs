//! Naming the branch a pipeline run works on.

/// The slug portion of a branch name is capped at this many characters --
/// long enough to keep a title recognizable, short enough that
/// `munibot/{issue_number}-{slug}` never approaches a forge's own branch
/// name length limit.
const MAX_SLUG_LENGTH: usize = 60;

/// Lowercases `text`, replaces every run of non-alphanumeric characters
/// with a single dash, trims leading/trailing dashes, and caps the result
/// at [`MAX_SLUG_LENGTH`] characters (trimming a trailing dash a
/// mid-word truncation might leave behind).
///
/// A pure function, deliberately: a wrong branch name here means a wrong
/// pull request, so this is tested directly rather than only ever
/// exercised indirectly through [`resolve_branch_name`].
fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut last_was_dash = false;

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }

    // a trailing dash from e.g. "fix: the crash!" ending on punctuation
    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.len() > MAX_SLUG_LENGTH {
        slug.truncate(MAX_SLUG_LENGTH);
        while slug.ends_with('-') {
            slug.pop();
        }
    }

    slug
}

/// The branch name a fresh pipeline run for `issue_number`/`title` would
/// use, ignoring whatever already exists -- see [`resolve_branch_name`]
/// for the idempotent, collision-aware version an executor should
/// actually call.
fn branch_name(issue_number: u64, title: &str) -> String {
    let slug = slugify(title);
    if slug.is_empty() {
        format!("munibot/{issue_number}")
    } else {
        format!("munibot/{issue_number}-{slug}")
    }
}

/// The branch name a pipeline run should actually use.
///
/// `already_assigned` is whatever branch name this exact pipeline run
/// already committed to earlier in its own history (its own event log, in
/// practice) -- when present, it wins unconditionally and verbatim,
/// regardless of what a freshly computed slug would look like now. This
/// is the whole idempotent-reuse guarantee: a rejected plan's retry, or a
/// resume after a restart, always continues on the exact same branch it
/// started on, even if the issue's title changed in the meantime.
///
/// Only when `already_assigned` is `None` -- this run has never decided on
/// a branch before -- is a fresh name computed from `title`, and only
/// then can `existing_branches` cause an attempt suffix to be appended:
/// an unrelated branch (a human's own, or a stale one from some earlier,
/// unrelated run) that happens to already occupy the exact name this run
/// would otherwise have picked.
pub fn resolve_branch_name(
    issue_number: u64,
    title: &str,
    already_assigned: Option<&str>,
    existing_branches: &[String],
) -> String {
    if let Some(existing) = already_assigned {
        return existing.to_string();
    }

    let base = branch_name(issue_number, title);
    if !existing_branches.iter().any(|branch| branch == &base) {
        return base;
    }

    (2..)
        .map(|attempt| format!("{base}-{attempt}"))
        .find(|candidate| !existing_branches.contains(candidate))
        .expect("an unbounded range always yields a name eventually")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_lowercases_and_dashes_a_title() {
        assert_eq!(slugify("Fix the Login Crash"), "fix-the-login-crash");
    }

    #[test]
    fn test_slugify_collapses_runs_of_punctuation_into_one_dash() {
        assert_eq!(slugify("fix: the crash!!"), "fix-the-crash");
    }

    #[test]
    fn test_slugify_trims_leading_and_trailing_punctuation() {
        assert_eq!(slugify("--fix the crash--"), "fix-the-crash");
    }

    #[test]
    fn test_slugify_keeps_alphanumerics_only() {
        assert_eq!(slugify("crash() on login #42"), "crash-on-login-42");
    }

    #[test]
    fn test_slugify_of_an_empty_title_is_empty() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn test_slugify_of_only_punctuation_is_empty() {
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn test_slugify_caps_at_the_max_length() {
        let long_title = "a ".repeat(100);
        let slug = slugify(&long_title);
        assert!(slug.len() <= MAX_SLUG_LENGTH);
    }

    #[test]
    fn test_slugify_never_ends_in_a_dash_after_truncation() {
        // built so the cap lands mid-word, forcing a trailing dash to trim
        let title = "word ".repeat(30);
        let slug = slugify(&title);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn test_branch_name_combines_issue_number_and_slug() {
        assert_eq!(
            branch_name(42, "Fix the Login Crash"),
            "munibot/42-fix-the-login-crash"
        );
    }

    #[test]
    fn test_branch_name_falls_back_to_bare_issue_number_for_an_untitled_issue() {
        assert_eq!(branch_name(42, "!!!"), "munibot/42");
    }

    #[test]
    fn test_resolve_branch_name_uses_the_computed_name_when_nothing_exists() {
        assert_eq!(
            resolve_branch_name(42, "Fix the Login Crash", None, &[]),
            "munibot/42-fix-the-login-crash"
        );
    }

    #[test]
    fn test_resolve_branch_name_reuses_an_already_assigned_branch_verbatim() {
        assert_eq!(
            resolve_branch_name(
                42,
                "Fix the Login Crash",
                Some("munibot/42-fix-the-login-crash"),
                &[]
            ),
            "munibot/42-fix-the-login-crash"
        );
    }

    #[test]
    fn test_resolve_branch_name_reuses_the_already_assigned_branch_even_if_the_title_changed() {
        // the issue's title changed since the branch was first assigned --
        // the same run must continue on the same branch regardless
        assert_eq!(
            resolve_branch_name(
                42,
                "A Completely Different Title",
                Some("munibot/42-old-title-entirely"),
                &[]
            ),
            "munibot/42-old-title-entirely"
        );
    }

    #[test]
    fn test_resolve_branch_name_ignores_existing_branches_for_other_issues() {
        let existing = vec!["munibot/99-some-other-issue".to_string()];
        assert_eq!(
            resolve_branch_name(42, "Fix the Login Crash", None, &existing),
            "munibot/42-fix-the-login-crash"
        );
    }

    #[test]
    fn test_resolve_branch_name_appends_an_attempt_suffix_on_an_unrelated_collision() {
        // nothing has been assigned to this run yet, but the freshly
        // computed name already exists -- a human's own branch, or a
        // stale one from an unrelated earlier run
        let existing = vec!["munibot/7-fix-the-crash".to_string()];
        assert_eq!(
            resolve_branch_name(7, "Fix The Crash", None, &existing),
            "munibot/7-fix-the-crash-2"
        );
    }

    #[test]
    fn test_resolve_branch_name_keeps_incrementing_past_multiple_collisions() {
        let existing = vec![
            "munibot/7-fix-the-crash".to_string(),
            "munibot/7-fix-the-crash-2".to_string(),
            "munibot/7-fix-the-crash-3".to_string(),
        ];
        assert_eq!(
            resolve_branch_name(7, "Fix The Crash", None, &existing),
            "munibot/7-fix-the-crash-4"
        );
    }

    #[test]
    fn test_resolve_branch_name_prefers_already_assigned_over_any_collision_logic() {
        // even if the "fresh" computation would collide, an already
        // assigned branch always wins outright -- there is nothing left
        // to resolve
        let existing = vec!["munibot/7-fix-the-crash".to_string()];
        assert_eq!(
            resolve_branch_name(
                7,
                "Fix The Crash",
                Some("munibot/7-fix-the-crash-2"),
                &existing
            ),
            "munibot/7-fix-the-crash-2"
        );
    }
}
