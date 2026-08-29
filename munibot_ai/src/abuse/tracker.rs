use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{limits::Scope, persona::PersonaId};

/// One scope's recent activity: just enough to notice a burst of
/// near-identical prompts or rapid persona switching.
///
/// Never persisted - the same reasoning
/// [`crate::limits::concurrency::ConcurrencyTracker`] documents for its own
/// in-memory-only state: this is inherently live, short-window history, and
/// losing it on a restart costs nothing but a few minutes of it.
#[derive(Default)]
struct ScopeActivity {
    recent_prompts: Vec<(String, Instant)>,
    recent_personas: Vec<(PersonaId, Instant)>,
}

/// Tracks each scope's recent prompts and persona choices in memory, for
/// [`crate::abuse::AbuseDetector`] to notice a burst pattern within a short
/// window.
#[derive(Default)]
pub(crate) struct ActivityTracker(Mutex<HashMap<Scope, ScopeActivity>>);

impl ActivityTracker {
    /// Records `message` (normalized) for `scope`, then returns how many of
    /// its prompts within `window` now match it - including this one.
    pub(crate) fn record_prompt(&self, scope: Scope, message: &str, window: Duration) -> u32 {
        let normalized = normalize(message);
        let now = Instant::now();
        let mut activity = self.0.lock().unwrap();
        let entry = activity.entry(scope).or_default();
        entry
            .recent_prompts
            .retain(|(_, seen)| now.duration_since(*seen) < window);
        entry.recent_prompts.push((normalized.clone(), now));
        entry
            .recent_prompts
            .iter()
            .filter(|(text, _)| *text == normalized)
            .count() as u32
    }

    /// Records `persona_id` for `scope`, then returns how many *distinct*
    /// personas it has used within `window` - including this one.
    pub(crate) fn record_persona(
        &self,
        scope: Scope,
        persona_id: &PersonaId,
        window: Duration,
    ) -> u32 {
        let now = Instant::now();
        let mut activity = self.0.lock().unwrap();
        let entry = activity.entry(scope).or_default();
        entry
            .recent_personas
            .retain(|(_, seen)| now.duration_since(*seen) < window);
        entry.recent_personas.push((persona_id.clone(), now));
        entry
            .recent_personas
            .iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>()
            .len() as u32
    }
}

/// Cheap "near-identical" normalization: lowercased, with runs of
/// whitespace collapsed to a single space and leading/trailing whitespace
/// trimmed.
///
/// Deliberately not a real similarity metric (no edit distance, no
/// shingling): it only catches someone replaying the exact same prompt
/// with nothing but casing or whitespace changed, which is the actual
/// cheap token-farming pattern this exists for - a real fuzzy match would
/// cost far more per message than this is worth spending on every single
/// one.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_single_prompt_matches_only_itself() {
        let tracker = ActivityTracker::default();
        let count = tracker.record_prompt(Scope::User(1), "hello", Duration::from_secs(60));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_repeating_the_same_prompt_increases_the_match_count() {
        let tracker = ActivityTracker::default();
        tracker.record_prompt(Scope::User(1), "hello", Duration::from_secs(60));
        tracker.record_prompt(Scope::User(1), "hello", Duration::from_secs(60));
        let count = tracker.record_prompt(Scope::User(1), "hello", Duration::from_secs(60));
        assert_eq!(count, 3);
    }

    #[test]
    fn test_normalization_ignores_case_and_whitespace() {
        let tracker = ActivityTracker::default();
        tracker.record_prompt(Scope::User(1), "Hello   there", Duration::from_secs(60));
        let count = tracker.record_prompt(Scope::User(1), "hello there", Duration::from_secs(60));
        assert_eq!(count, 2);
    }

    #[test]
    fn test_different_prompts_do_not_match_each_other() {
        let tracker = ActivityTracker::default();
        tracker.record_prompt(Scope::User(1), "hello", Duration::from_secs(60));
        let count = tracker.record_prompt(Scope::User(1), "goodbye", Duration::from_secs(60));
        assert_eq!(count, 1);
    }

    #[test]
    fn test_prompts_outside_the_window_are_not_counted() {
        let tracker = ActivityTracker::default();
        tracker.record_prompt(Scope::User(1), "hello", Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(30));
        let count = tracker.record_prompt(Scope::User(1), "hello", Duration::from_millis(10));
        assert_eq!(count, 1, "the earlier prompt should have aged out");
    }

    #[test]
    fn test_scopes_are_tracked_independently() {
        let tracker = ActivityTracker::default();
        tracker.record_prompt(Scope::User(1), "hello", Duration::from_secs(60));
        let count = tracker.record_prompt(Scope::User(2), "hello", Duration::from_secs(60));
        assert_eq!(count, 1, "a different scope should have its own history");
    }

    #[test]
    fn test_a_single_persona_counts_as_one_distinct_persona() {
        let tracker = ActivityTracker::default();
        let count = tracker.record_persona(
            Scope::User(1),
            &PersonaId::new("companion"),
            Duration::from_secs(60),
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn test_switching_personas_increases_the_distinct_count() {
        let tracker = ActivityTracker::default();
        tracker.record_persona(
            Scope::User(1),
            &PersonaId::new("companion"),
            Duration::from_secs(60),
        );
        let count = tracker.record_persona(
            Scope::User(1),
            &PersonaId::new("researcher"),
            Duration::from_secs(60),
        );
        assert_eq!(count, 2);
    }

    #[test]
    fn test_reusing_the_same_persona_does_not_inflate_the_distinct_count() {
        let tracker = ActivityTracker::default();
        tracker.record_persona(
            Scope::User(1),
            &PersonaId::new("companion"),
            Duration::from_secs(60),
        );
        let count = tracker.record_persona(
            Scope::User(1),
            &PersonaId::new("companion"),
            Duration::from_secs(60),
        );
        assert_eq!(count, 1);
    }
}
