//! The `Ai` service handle: the one surface every platform adapter touches.
//!
//! Everything below this module - personas, the harness, memory, tools, and
//! providers - exists to make [`Ai::turn`] and [`Ai::turn_streamed`] possible.
//! An adapter never reaches past this into the harness, a provider, or a
//! session store directly. Output filtering (mention stripping, length caps,
//! `decancer`) is deliberately not applied here: it is platform-specific and
//! is each adapter's own job, applied to `TurnOutcome::text` after this
//! returns.

use std::{collections::HashMap, sync::Arc};

use futures::{StreamExt, stream::BoxStream};
use tokio_util::sync::CancellationToken;

use crate::{
    harness::{Harness, HarnessEvent, TurnOutcome, TurnRequest},
    memory::{
        CompactionSettings, ConversationScope, SessionStore, Summariser, assemble_context,
        compact_if_needed,
    },
    persona::{AiConfig, MemoryPolicy, Persona, PersonaId, PersonaRegistry},
    provider::{Provider, ProviderRegistry, ProviderResolver},
    tools::{ConversationId, RiskTier, ToolCtx, ToolRegistry},
    types::{AiError, History, Message, ModelRef, rough_token_estimate},
};

/// How many tokens of prior conversation history to include when assembling
/// context for a turn whose persona wants memory.
///
/// Independent of a persona's own `Budget::max_input_tokens`, which caps spend
/// across a turn's own round trips, not how much history goes in - the two
/// concerns just happen to both be measured in tokens.
const CONTEXT_TOKEN_BUDGET: usize = 6_000;

/// Resolves a model reference to a working provider.
///
/// A trait rather than depending on [`ProviderResolver`] directly, so tests
/// can substitute a fixed provider without real credentials or a network
/// call. Production always uses [`ProviderResolver`] itself, via its impl
/// below.
pub trait ProviderSource: Send + Sync {
    fn resolve(&self, model: &ModelRef) -> Result<Arc<dyn Provider>, AiError>;
}

impl ProviderSource for ProviderResolver {
    fn resolve(&self, model: &ModelRef) -> Result<Arc<dyn Provider>, AiError> {
        ProviderResolver::resolve(self, model)
    }
}

/// Everything a platform adapter needs to run one turn of conversation.
///
/// Names a persona explicitly rather than leaving it to be inferred -
/// automatic routing between personas arrives in milestone 2.
pub struct AiTurnRequest {
    pub persona_id: PersonaId,
    pub scope: ConversationScope,
    /// The invoking human's internal `users.id`, matching
    /// [`crate::tools::ToolCtx::user_id`].
    pub user_id: u64,
    /// The invoker's display name, rendered into a persona's `{{user_name}}`
    /// system prompt variable.
    pub user_name: String,
    /// The tier this invocation is authorized for, set by the adapter from
    /// the invoker's actual platform permissions.
    pub granted_tier: RiskTier,
    pub guild_id: Option<u64>,
    pub message: String,
    pub cancellation: CancellationToken,
}

/// Everything [`Ai::prepare`] resolves once, shared by both [`Ai::turn`] and
/// [`Ai::turn_streamed`].
struct PreparedTurn {
    request: TurnRequest,
    provider: Arc<dyn Provider>,
    conversation_id: ConversationId,
    /// The persona's display name, for [`HarnessEvent::TurnStarted`] - the
    /// harness itself only knows the model reference, not the persona.
    persona_label: String,
}

/// The one surface every platform adapter touches.
///
/// Wraps a resolved [`PersonaRegistry`], the shared tool registry, a session
/// store, and a provider source into the two entry points a turn ever needs.
/// Nothing above this reaches into the harness, a provider, or memory
/// directly.
pub struct Ai {
    personas: PersonaRegistry,
    tools: Arc<ToolRegistry>,
    sessions: Arc<dyn SessionStore>,
    providers: Arc<dyn ProviderSource>,
    /// `None` until [`Self::with_summariser`] enables it. Without it, a
    /// conversation simply keeps growing forever, bounded only by whatever a
    /// `SessionStore` itself enforces (the in-memory store's message cap) -
    /// exactly the behaviour every turn had before this existed.
    compaction: Option<(Summariser, CompactionSettings)>,
}

impl Ai {
    /// Builds the service from configuration: checks provider credentials
    /// from the environment and resolves every configured persona, failing at
    /// startup rather than mid-conversation if either is wrong.
    ///
    /// `tools` and `sessions` are supplied rather than built here - which
    /// tools are wired in depends on what credentials and infrastructure the
    /// caller has available (an Exa key, a database), and this service has no
    /// business deciding that on its own.
    pub fn new(
        config: &AiConfig,
        tools: Arc<ToolRegistry>,
        sessions: Arc<dyn SessionStore>,
    ) -> Result<Self, AiError> {
        let provider_registry = ProviderRegistry::from_env();
        let personas = PersonaRegistry::load(config, &provider_registry)?;
        Ok(Self::from_parts(
            personas,
            tools,
            sessions,
            Arc::new(ProviderResolver::new()),
        ))
    }

    /// Builds the service from already-resolved parts.
    ///
    /// The lower-level constructor tests use to substitute a
    /// [`ProviderSource`] that never touches the network, and that a caller
    /// with its own source of providers can use directly.
    pub fn from_parts(
        personas: PersonaRegistry,
        tools: Arc<ToolRegistry>,
        sessions: Arc<dyn SessionStore>,
        providers: Arc<dyn ProviderSource>,
    ) -> Self {
        Self {
            personas,
            tools,
            sessions,
            providers,
            compaction: None,
        }
    }

    /// Enables automatic conversation compaction.
    ///
    /// Checked before assembling context for any turn whose persona reads
    /// memory (`MemoryPolicy::Conversation` or `User`), and a no-op below
    /// `settings.threshold_tokens` - see [`compact_if_needed`]. Additive: a
    /// service built without ever calling this behaves exactly as it did
    /// before compaction existed.
    pub fn with_summariser(mut self, summariser: Summariser, settings: CompactionSettings) -> Self {
        self.compaction = Some((summariser, settings));
        self
    }

    /// Runs one full turn, returning only once it has finished.
    pub async fn turn(&self, req: AiTurnRequest) -> Result<TurnOutcome, AiError> {
        let prepared = self.prepare(&req).await?;
        let harness = Harness::new(prepared.provider, self.tools.clone());
        let outcome = harness.run_turn(prepared.request).await?;

        if let Some(text) = &outcome.text {
            self.sessions
                .append(prepared.conversation_id, Message::assistant(text.clone()))
                .await?;
        }

        Ok(outcome)
    }

    /// Runs one full turn, yielding progress events as they happen.
    ///
    /// The harness's own [`HarnessEvent::TurnStarted`] carries the model
    /// reference, since the harness has no persona type of its own to name
    /// instead; this replaces it with the persona's display name, which is
    /// what an adapter actually wants to show a user.
    pub async fn turn_streamed(
        &self,
        req: AiTurnRequest,
    ) -> Result<BoxStream<'static, HarnessEvent>, AiError> {
        let prepared = self.prepare(&req).await?;
        let harness = Arc::new(Harness::new(prepared.provider, self.tools.clone()));
        let sessions = self.sessions.clone();
        let conversation_id = prepared.conversation_id;
        let persona_label = prepared.persona_label;

        let stream = async_stream::stream! {
            let mut assistant_text = String::new();
            let mut inner = harness.run_turn_streamed(prepared.request);

            while let Some(event) = inner.next().await {
                match event {
                    HarnessEvent::TurnStarted { .. } => {
                        yield HarnessEvent::TurnStarted { persona: persona_label.clone() };
                    }
                    HarnessEvent::TextDelta(text) => {
                        assistant_text.push_str(&text);
                        yield HarnessEvent::TextDelta(text);
                    }
                    HarnessEvent::TurnFinished { usage, cost } => {
                        if !assistant_text.is_empty() {
                            // best-effort: a session store failure here should not turn an
                            // otherwise-successful streamed turn into a Failed event this late
                            let _ = sessions
                                .append(conversation_id, Message::assistant(assistant_text.clone()))
                                .await;
                        }
                        yield HarnessEvent::TurnFinished { usage, cost };
                    }
                    other => yield other,
                }
            }
        };

        Ok(Box::pin(stream))
    }

    /// Clears a conversation's history and summary, for an adapter's
    /// `/reset`-style command.
    ///
    /// `persona_id` only matters if `scope` has no conversation yet: creating
    /// one just to immediately clear it is harmless, and cheaper than adding
    /// a second `SessionStore` method purely to check existence first.
    pub async fn reset_conversation(
        &self,
        scope: &ConversationScope,
        persona_id: &PersonaId,
    ) -> Result<(), AiError> {
        let conversation = self.sessions.load_or_create(scope, &persona_id.0).await?;
        self.sessions.clear(conversation.id).await
    }

    /// Looks a persona up by id, failing with a named error rather than
    /// panicking when an adapter names one that does not exist.
    fn require_persona(&self, id: &PersonaId) -> Result<&Persona, AiError> {
        self.persona(id)
            .ok_or_else(|| AiError::Config(format!("no persona named {id} :<")))
    }

    /// Looks a resolved persona up by id, for an adapter that wants to show
    /// its description or display name without starting a turn.
    pub fn persona(&self, id: &PersonaId) -> Option<&Persona> {
        self.personas.get(id)
    }

    /// Every resolved persona, for populating a persona-selection command.
    pub fn personas(&self) -> impl Iterator<Item = &Persona> {
        self.personas.ids().filter_map(|id| self.personas.get(id))
    }

    /// The configured default persona's id, for an adapter deciding which
    /// persona to use when a user has not named one explicitly - an
    /// auto-triggered mention or direct message, rather than an explicit
    /// `/ask persona:researcher`.
    pub fn default_persona_id(&self) -> Option<&PersonaId> {
        self.personas.default_persona().map(|persona| &persona.id)
    }

    /// Everything shared between [`Self::turn`] and [`Self::turn_streamed`]:
    /// persona lookup, provider resolution, conversation loading, context
    /// assembly, and system prompt rendering.
    async fn prepare(&self, req: &AiTurnRequest) -> Result<PreparedTurn, AiError> {
        let persona = self.require_persona(&req.persona_id)?;
        let provider = self.providers.resolve(&persona.model)?;

        let mut conversation = self
            .sessions
            .load_or_create(&req.scope, &persona.id.0)
            .await?;
        // stored regardless of the persona's memory policy: a durable per-scope
        // record is harmless to keep even for a persona that does not read it back,
        // and a later policy change should not start from a conversation with a gap
        // in it
        self.sessions
            .append(conversation.id, Message::user(req.message.clone()))
            .await?;

        let history = match persona.memory {
            MemoryPolicy::None => History::from(vec![Message::user(req.message.clone())]),
            MemoryPolicy::Conversation | MemoryPolicy::User => {
                if let Some((summariser, settings)) = &self.compaction {
                    match compact_if_needed(
                        summariser,
                        self.sessions.as_ref(),
                        &conversation,
                        *settings,
                        rough_token_estimate,
                    )
                    .await
                    {
                        Ok(Some(new_summary)) => conversation.summary = Some(new_summary),
                        Ok(None) => {}
                        // best-effort: a failed compaction should not break the turn - the
                        // conversation simply stays uncompacted and gets another chance once it
                        // grows past the threshold again next time
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                conversation_id = conversation.id.0,
                                "conversation compaction failed; continuing uncompacted"
                            );
                        }
                    }
                }

                assemble_context(
                    self.sessions.as_ref(),
                    &conversation,
                    CONTEXT_TOKEN_BUDGET,
                    rough_token_estimate,
                )
                .await?
            }
        };

        let system = Self::render_system_prompt(persona, req)?;

        let ctx = ToolCtx {
            user_id: req.user_id,
            platform: req.scope.platform,
            granted_tier: req.granted_tier,
            guild_id: req.guild_id,
            conversation_id: conversation.id,
            cancellation: req.cancellation.clone(),
        };

        let mut turn_request = TurnRequest::new(persona.model.clone(), history, ctx)
            .with_system(system)
            .with_tools(persona.tools.clone())
            .with_params(persona.params.clone())
            .with_budget(persona.budget.clone());
        if let Some(handoff) = &persona.handoff {
            turn_request = turn_request.with_handoff(handoff.clone());
        }

        Ok(PreparedTurn {
            request: turn_request,
            provider,
            conversation_id: conversation.id,
            persona_label: persona.display_name.clone(),
        })
    }

    /// Renders a persona's system prompt with the variables every persona
    /// prompt references: `{{user_name}}` and `{{platform}}`.
    fn render_system_prompt(persona: &Persona, req: &AiTurnRequest) -> Result<String, AiError> {
        let context = HashMap::from([
            ("user_name".to_string(), req.user_name.clone()),
            ("platform".to_string(), req.scope.platform.to_string()),
        ]);
        persona.system_prompt.render(&context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory::{CompactionPersona, InMemorySessionStore},
        persona::PersonaConfig,
        provider::MockProvider,
        tools::Platform,
        types::ModelRef,
    };

    /// A [`ProviderSource`] that always returns the same fixed provider,
    /// regardless of which model is asked for - enough to test [`Ai`] without
    /// any real credentials or network access.
    struct FixedProviderSource(Arc<dyn Provider>);

    impl ProviderSource for FixedProviderSource {
        fn resolve(&self, _model: &ModelRef) -> Result<Arc<dyn Provider>, AiError> {
            Ok(self.0.clone())
        }
    }

    /// Builds a real, offline-resolved [`PersonaRegistry`] with one persona
    /// named `companion`, whose prompt is the embedded `companion.md` and
    /// whose memory policy is `memory`.
    fn personas_with_memory(memory: MemoryPolicy) -> PersonaRegistry {
        let mut config = AiConfig {
            enabled: true,
            default_persona: Some(PersonaId::new("companion")),
            prompt_dir: None,
            personas: HashMap::new(),
        };
        config
            .personas
            .insert(PersonaId::new("companion"), PersonaConfig {
                model: ModelRef::new("anthropic", "claude-opus-5"),
                prompt: "companion.md".to_string(),
                display_name: Some("Companion".to_string()),
                description: "warm, playful conversation".to_string(),
                temperature: None,
                tools: crate::tools::ToolSelection::none(),
                budget: crate::persona::BudgetConfig::default(),
                memory,
                sandbox: crate::persona::SandboxPolicy::default(),
            });

        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        PersonaRegistry::load(&config, &providers).expect("should resolve")
    }

    fn ai_with(memory: MemoryPolicy, provider: Arc<dyn Provider>) -> Ai {
        Ai::from_parts(
            personas_with_memory(memory),
            Arc::new(ToolRegistry::new()),
            Arc::new(InMemorySessionStore::new()),
            Arc::new(FixedProviderSource(provider)),
        )
    }

    fn request(persona_id: &str, message: &str) -> AiTurnRequest {
        AiTurnRequest {
            persona_id: PersonaId::new(persona_id),
            scope: ConversationScope::new(Platform::Discord, "channel-1"),
            user_id: 1,
            user_name: "muni".to_string(),
            granted_tier: RiskTier::Safe,
            guild_id: None,
            message: message.to_string(),
            cancellation: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn test_turn_returns_the_providers_reply() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_text("hi there"));
        let ai = ai_with(MemoryPolicy::None, provider);

        let outcome = ai
            .turn(request("companion", "hello"))
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("hi there"));
    }

    #[tokio::test]
    async fn test_turn_fails_for_an_unknown_persona() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_text("unused"));
        let ai = ai_with(MemoryPolicy::None, provider);

        let result = ai.turn(request("does-not-exist", "hello")).await;
        assert!(result.is_err(), "an unknown persona must not silently run");
    }

    #[tokio::test]
    async fn test_system_prompt_is_rendered_with_user_name_and_platform() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let ai = ai_with(MemoryPolicy::None, provider.clone());

        ai.turn(request("companion", "hello"))
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        let system = sent.system.as_deref().expect("should have a system prompt");
        assert!(
            system.contains("muni") && system.contains("Discord"),
            "the rendered prompt should mention the user and platform: {system:?}"
        );
    }

    #[tokio::test]
    async fn test_memory_none_persona_never_sees_earlier_turns() {
        let provider = Arc::new(
            MockProvider::new()
                .respond_text("first reply")
                .respond_text("second reply"),
        );
        let ai = ai_with(MemoryPolicy::None, provider.clone());

        ai.turn(request("companion", "message one"))
            .await
            .expect("should succeed");
        ai.turn(request("companion", "message two"))
            .await
            .expect("should succeed");

        let second_sent = &provider.requests()[1];
        assert_eq!(
            second_sent.history.len(),
            1,
            "a MemoryPolicy::None persona should only ever see the current message, got {:?}",
            second_sent.history
        );
    }

    #[tokio::test]
    async fn test_memory_conversation_persona_sees_earlier_turns() {
        let provider = Arc::new(
            MockProvider::new()
                .respond_text("first reply")
                .respond_text("second reply"),
        );
        let ai = ai_with(MemoryPolicy::Conversation, provider.clone());

        ai.turn(request("companion", "message one"))
            .await
            .expect("should succeed");
        ai.turn(request("companion", "message two"))
            .await
            .expect("should succeed");

        let second_sent = &provider.requests()[1];
        assert!(
            second_sent.history.len() > 1,
            "a MemoryPolicy::Conversation persona should see prior turns too, got {:?}",
            second_sent.history
        );
    }

    #[tokio::test]
    async fn test_assistant_reply_is_recorded_in_the_session_store() {
        let provider = Arc::new(
            MockProvider::new()
                .respond_text("remember this reply")
                .respond_text("second reply"),
        );
        let ai = ai_with(MemoryPolicy::Conversation, provider.clone());

        ai.turn(request("companion", "hello"))
            .await
            .expect("should succeed");
        ai.turn(request("companion", "hello again"))
            .await
            .expect("should succeed");

        let second_sent = &provider.requests()[1];
        let texts: Vec<String> = second_sent.history.iter().map(Message::text).collect();
        assert!(
            texts.iter().any(|text| text == "remember this reply"),
            "the first turn's assistant reply should appear in the next turn's history, got \
             {texts:?}"
        );
    }

    #[tokio::test]
    async fn test_turn_streamed_replaces_turn_started_with_the_persona_label() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_text("hi"));
        let ai = ai_with(MemoryPolicy::None, provider);

        let events: Vec<HarnessEvent> = ai
            .turn_streamed(request("companion", "hello"))
            .await
            .expect("should succeed")
            .collect()
            .await;

        assert!(
            matches!(
                events.first(),
                Some(HarnessEvent::TurnStarted { persona }) if persona == "Companion"
            ),
            "TurnStarted should carry the persona's display name, got {:?}",
            events.first()
        );
    }

    #[tokio::test]
    async fn test_turn_streamed_records_the_assembled_reply() {
        let provider = Arc::new(
            MockProvider::new()
                .respond_text("streamed reply")
                .respond_text("second reply"),
        );
        let ai = ai_with(MemoryPolicy::Conversation, provider.clone());

        let _events: Vec<HarnessEvent> = ai
            .turn_streamed(request("companion", "hello"))
            .await
            .expect("should succeed")
            .collect()
            .await;

        // a follow-up turn should see the streamed reply in its history, proving
        // turn_streamed persisted it even though it never calls turn()
        let _events2: Vec<HarnessEvent> = ai
            .turn_streamed(request("companion", "and then?"))
            .await
            .expect("should succeed")
            .collect()
            .await;

        let second_sent = &provider.requests()[1];
        let texts: Vec<String> = second_sent.history.iter().map(Message::text).collect();
        assert!(
            texts.iter().any(|text| text == "streamed reply"),
            "the streamed turn's assistant text should have been recorded, got {texts:?}"
        );
    }

    #[tokio::test]
    async fn test_turn_propagates_a_fatal_provider_error() {
        let provider: Arc<dyn Provider> =
            Arc::new(MockProvider::new().respond_error(AiError::Rejected("bad key".to_string())));
        let ai = ai_with(MemoryPolicy::None, provider);

        let result = ai.turn(request("companion", "hello")).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_default_persona_id_reflects_the_configured_default() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new());
        let ai = ai_with(MemoryPolicy::None, provider);

        assert_eq!(ai.default_persona_id(), Some(&PersonaId::new("companion")));
    }

    #[test]
    fn test_persona_looks_up_a_resolved_persona_by_id() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new());
        let ai = ai_with(MemoryPolicy::None, provider);

        let persona = ai
            .persona(&PersonaId::new("companion"))
            .expect("should resolve");
        assert_eq!(persona.display_name, "Companion");
    }

    #[test]
    fn test_persona_returns_none_for_an_unknown_id() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new());
        let ai = ai_with(MemoryPolicy::None, provider);

        assert!(ai.persona(&PersonaId::new("does-not-exist")).is_none());
    }

    #[test]
    fn test_personas_lists_every_resolved_persona() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new());
        let ai = ai_with(MemoryPolicy::None, provider);

        let ids: Vec<&PersonaId> = ai.personas().map(|persona| &persona.id).collect();
        assert_eq!(ids, vec![&PersonaId::new("companion")]);
    }

    #[tokio::test]
    async fn test_reset_conversation_clears_history_a_later_turn_would_have_seen() {
        let provider = Arc::new(
            MockProvider::new()
                .respond_text("first reply")
                .respond_text("second reply"),
        );
        let ai = ai_with(MemoryPolicy::Conversation, provider.clone());

        ai.turn(request("companion", "message one"))
            .await
            .expect("should succeed");

        ai.reset_conversation(
            &ConversationScope::new(Platform::Discord, "channel-1"),
            &PersonaId::new("companion"),
        )
        .await
        .expect("should succeed");

        ai.turn(request("companion", "message two"))
            .await
            .expect("should succeed");

        let second_sent = &provider.requests()[1];
        assert_eq!(
            second_sent.history.len(),
            1,
            "a reset conversation should not carry the earlier turn's history, got {:?}",
            second_sent.history
        );
    }

    #[tokio::test]
    async fn test_reset_conversation_on_a_scope_with_no_history_yet_is_not_an_error() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new());
        let ai = ai_with(MemoryPolicy::None, provider);

        let result = ai
            .reset_conversation(
                &ConversationScope::new(Platform::Discord, "brand-new-channel"),
                &PersonaId::new("companion"),
            )
            .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_provider_resolver_implements_provider_source() {
        // a compile-time check: production code must be able to hand a real
        // ProviderResolver to Ai::from_parts without an adapter
        fn assert_impl<T: ProviderSource>() {}
        assert_impl::<ProviderResolver>();
    }

    #[tokio::test]
    async fn test_a_conversation_is_left_alone_without_a_summariser() {
        // regression guard: every other test in this module builds an Ai with no
        // summariser wired, so this is really asserting the default is inert
        let provider = Arc::new(
            MockProvider::new()
                .respond_text("first")
                .respond_text("second"),
        );
        let ai = ai_with(MemoryPolicy::Conversation, provider.clone());

        ai.turn(request("companion", "message one")).await.unwrap();
        ai.turn(request("companion", "message two")).await.unwrap();

        let second_sent = &provider.requests()[1];
        assert!(
            second_sent.history.len() >= 3,
            "with no summariser wired, nothing should ever be removed from history"
        );
    }

    #[tokio::test]
    async fn test_ai_compacts_a_conversation_automatically_when_wired_with_a_summariser() {
        let chat_provider = Arc::new(
            MockProvider::new()
                .respond_text("first reply")
                .respond_text("second reply"),
        );
        let compaction_provider = Arc::new(MockProvider::new().respond_text("condensed"));
        let summariser = Summariser::new(
            compaction_provider.clone(),
            CompactionPersona::embedded(ModelRef::new("anthropic", "claude-haiku-4")),
        );

        let ai = ai_with(MemoryPolicy::Conversation, chat_provider.clone()).with_summariser(
            summariser,
            CompactionSettings {
                threshold_tokens: 1,
                keep_recent_messages: 1,
            },
        );

        // first turn: only one message exists once the user's text is appended, which
        // is never more than keep_recent_messages - nothing to compact yet
        ai.turn(request("companion", "message one"))
            .await
            .expect("should succeed");
        assert_eq!(
            compaction_provider.request_count(),
            0,
            "a single message must never be summarised away"
        );

        // second turn: history now has three messages (the first exchange, plus this
        // turn's new user message), comfortably over both thresholds
        ai.turn(request("companion", "message two"))
            .await
            .expect("should succeed");
        assert_eq!(
            compaction_provider.request_count(),
            1,
            "growing past the threshold should trigger exactly one compaction call"
        );
    }

    #[tokio::test]
    async fn test_a_failed_compaction_does_not_fail_the_turn() {
        let chat_provider = Arc::new(
            MockProvider::new()
                .respond_text("first reply")
                .respond_text("second reply"),
        );
        let compaction_provider =
            Arc::new(MockProvider::new().respond_error(AiError::Provider("outage".to_string())));
        let summariser = Summariser::new(
            compaction_provider,
            CompactionPersona::embedded(ModelRef::new("anthropic", "claude-haiku-4")),
        );

        let ai = ai_with(MemoryPolicy::Conversation, chat_provider).with_summariser(
            summariser,
            CompactionSettings {
                threshold_tokens: 1,
                keep_recent_messages: 1,
            },
        );

        ai.turn(request("companion", "message one"))
            .await
            .expect("should succeed");
        let outcome = ai.turn(request("companion", "message two")).await;

        assert!(
            outcome.is_ok(),
            "a compaction failure must not surface as a failed turn"
        );
    }
}
