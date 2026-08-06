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
    audit::ToolAuditor,
    crisis::{CrisisClassifier, CrisisSeverity},
    harness::{Harness, HarnessEvent, TurnOutcome, TurnRequest},
    memory::{
        CompactionSettings, ConversationScope, MemoryStore, SessionStore, Summariser,
        assemble_context, compact_if_needed,
    },
    persona::{AiConfig, MemoryPolicy, Persona, PersonaId, PersonaRegistry},
    provider::{Provider, ProviderRegistry, ProviderResolver, estimate_cost},
    tools::{ConversationId, RiskTier, ToolCtx, ToolRegistry},
    types::{AiError, Cost, History, Message, ModelRef, Usage, rough_token_estimate},
    usage::{UsageRecord, UsageRecorder},
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
    /// When `true`, `message` is not appended to the session store before
    /// building history, because the caller already persisted it.
    ///
    /// Every adapter so far leaves this `false`: `message` is fresh text
    /// [`Ai::turn`]/[`Ai::turn_streamed`] is trusted to store on the
    /// caller's behalf. The web chat surface needs the opposite - its
    /// `send_message` server function already writes the user's message to
    /// the same table, since SSE (a `GET`) can't carry a pasted code block
    /// as a query string, so by the time a stream request reaches here,
    /// storing `message` again would duplicate it.
    pub already_persisted: bool,
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
    /// The persona's stable id, for a [`crate::usage::UsageRecord`] - distinct
    /// from `persona_label`, which is the display name shown to a user rather
    /// than the identifier a usage dashboard groups and filters by.
    persona_id: String,
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
    /// `None` until [`Self::with_usage_recorder`] enables it. Without it, a
    /// turn's cost is never persisted anywhere - exactly the behaviour every
    /// turn had before this existed.
    usage_recorder: Option<Arc<dyn UsageRecorder>>,
    /// `None` until [`Self::with_tool_auditor`] enables it.
    tool_auditor: Option<Arc<dyn ToolAuditor>>,
    /// `None` until [`Self::with_memory_store`] enables it. Without it, a
    /// `MemoryPolicy::User` persona's `{{memories}}` renders as a neutral
    /// placeholder rather than any real memories - see
    /// [`Self::load_memories_text`].
    memory_store: Option<Arc<dyn MemoryStore>>,
    /// `None` until [`Self::with_crisis_classifier`] enables it. Without it,
    /// no inbound message is ever screened at all - exactly the behaviour
    /// every turn had before this existed.
    crisis_classifier: Option<CrisisClassifier>,
}

/// Writes `record` through `recorder` if one is configured, logging and
/// swallowing a failure rather than propagating it.
///
/// A free function rather than a method on `Ai`, so [`Ai::turn_streamed`] can
/// call it from inside its `'static` stream body, which cannot hold a borrow
/// of `&self` across an `.await` point - the same reason that body already
/// clones `sessions` out of `self` rather than capturing `self` itself.
///
/// Recording is inherently best-effort: a turn that already finished (or
/// failed) has nothing to gain from a usage-table write also failing it
/// retroactively, and a dashboard being briefly incomplete is a far smaller
/// problem than a working conversation turning into an error because its own
/// bookkeeping stumbled.
async fn write_usage(recorder: &Option<Arc<dyn UsageRecorder>>, record: UsageRecord) {
    let Some(recorder) = recorder else { return };
    if let Err(error) = recorder.record(record).await {
        tracing::warn!(%error, "failed to record ai usage");
    }
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
            usage_recorder: None,
            tool_auditor: None,
            memory_store: None,
            crisis_classifier: None,
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

    /// Enables recording a [`crate::usage::UsageRecord`] after every turn,
    /// on failure as well as success.
    pub fn with_usage_recorder(mut self, recorder: Arc<dyn UsageRecorder>) -> Self {
        self.usage_recorder = Some(recorder);
        self
    }

    /// Enables auditing every tool call a turn makes.
    pub fn with_tool_auditor(mut self, auditor: Arc<dyn ToolAuditor>) -> Self {
        self.tool_auditor = Some(auditor);
        self
    }

    /// Enables rendering a `MemoryPolicy::User` persona's `{{memories}}`
    /// system prompt variable from a real store, rather than a neutral
    /// placeholder.
    pub fn with_memory_store(mut self, store: Arc<dyn MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Enables screening every inbound message a `MemoryPolicy::User`
    /// persona receives for [`CrisisSeverity`].
    ///
    /// Only screens, for now: a positive signal is logged so a pattern of
    /// them is visible to an operator, but nothing yet changes about how the
    /// turn itself proceeds. Bypassing the normal turn for a reviewed,
    /// non-generated response on a positive signal is a separate, later
    /// concern - see `crisis`'s own module doc comment for why that split is
    /// deliberate.
    pub fn with_crisis_classifier(mut self, classifier: CrisisClassifier) -> Self {
        self.crisis_classifier = Some(classifier);
        self
    }

    /// Runs one full turn, returning only once it has finished.
    pub async fn turn(&self, req: AiTurnRequest) -> Result<TurnOutcome, AiError> {
        let prepared = self.prepare(&req).await?;
        let conversation_id = prepared.conversation_id;
        let model = prepared.request.model.clone();
        let persona_id = prepared.persona_id.clone();

        let mut harness = Harness::new(prepared.provider, self.tools.clone());
        if let Some(auditor) = &self.tool_auditor {
            harness = harness.with_auditor(auditor.clone());
        }
        let (result, turn_usage) = harness.run_turn_recording_usage(prepared.request).await;

        write_usage(&self.usage_recorder, UsageRecord {
            conversation_id: Some(conversation_id),
            user_id: Some(req.user_id),
            guild_id: req.guild_id,
            provider: model.provider().to_string(),
            model: model.model().to_string(),
            persona_id,
            usage: turn_usage.usage,
            cost: turn_usage.cost,
            iterations: turn_usage.iterations,
            succeeded: result.is_ok(),
        })
        .await;

        let outcome = result?;

        if let Some(text) = &outcome.text {
            self.sessions
                .append(conversation_id, Message::assistant(text.clone()))
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
        let mut harness = Harness::new(prepared.provider, self.tools.clone());
        if let Some(auditor) = &self.tool_auditor {
            harness = harness.with_auditor(auditor.clone());
        }
        let harness = Arc::new(harness);
        let sessions = self.sessions.clone();
        let usage_recorder = self.usage_recorder.clone();
        let conversation_id = prepared.conversation_id;
        let persona_label = prepared.persona_label;
        let persona_id = prepared.persona_id;
        let model = prepared.request.model.clone();
        let user_id = req.user_id;
        let guild_id = req.guild_id;

        let stream = async_stream::stream! {
            let mut assistant_text = String::new();
            // TurnFinished's own fields already carry the true cumulative totals
            // (straight from the harness's own budget tracker), so this accumulation
            // exists only to have *something* to record on the Handoff and Failed
            // paths, neither of which carries usage in the event itself
            let mut accumulated_usage = Usage::default();
            let mut accumulated_cost = Cost::ZERO;
            let mut iterations = 0usize;
            let mut inner = harness.run_turn_streamed(prepared.request);

            let record = |usage: Usage, cost: Cost, iterations: usize, succeeded: bool| UsageRecord {
                conversation_id: Some(conversation_id),
                user_id: Some(user_id),
                guild_id,
                provider: model.provider().to_string(),
                model: model.model().to_string(),
                persona_id: persona_id.clone(),
                usage,
                cost,
                iterations,
                succeeded,
            };

            while let Some(event) = inner.next().await {
                match event {
                    HarnessEvent::TurnStarted { .. } => {
                        yield HarnessEvent::TurnStarted { persona: persona_label.clone() };
                    }
                    HarnessEvent::TextDelta(text) => {
                        assistant_text.push_str(&text);
                        yield HarnessEvent::TextDelta(text);
                    }
                    HarnessEvent::IterationComplete { iteration, usage } => {
                        accumulated_usage += usage;
                        accumulated_cost += estimate_cost(&model, &usage);
                        iterations = iteration;
                        yield HarnessEvent::IterationComplete { iteration, usage };
                    }
                    HarnessEvent::TurnFinished { usage, cost } => {
                        if !assistant_text.is_empty() {
                            // best-effort: a session store failure here should not turn an
                            // otherwise-successful streamed turn into a Failed event this late
                            let _ = sessions
                                .append(conversation_id, Message::assistant(assistant_text.clone()))
                                .await;
                        }
                        write_usage(&usage_recorder, record(usage, cost, iterations, true)).await;
                        yield HarnessEvent::TurnFinished { usage, cost };
                    }
                    HarnessEvent::Handoff(payload) => {
                        write_usage(
                            &usage_recorder,
                            record(accumulated_usage, accumulated_cost, iterations, true),
                        )
                        .await;
                        yield HarnessEvent::Handoff(payload);
                    }
                    HarnessEvent::Failed(error) => {
                        write_usage(
                            &usage_recorder,
                            record(accumulated_usage, accumulated_cost, iterations, false),
                        )
                        .await;
                        yield HarnessEvent::Failed(error);
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

        if persona.memory == MemoryPolicy::User
            && let Some(classifier) = &self.crisis_classifier
        {
            let severity = classifier.classify(&req.message).await;
            if severity >= CrisisSeverity::Elevated {
                tracing::warn!(
                    user_id = req.user_id,
                    ?severity,
                    "crisis classifier flagged an inbound message :< no automated response path \
                     exists yet"
                );
            }
        }

        let mut conversation = self
            .sessions
            .load_or_create(&req.scope, &persona.id.0)
            .await?;
        // stored regardless of the persona's memory policy: a durable per-scope
        // record is harmless to keep even for a persona that does not read it back,
        // and a later policy change should not start from a conversation with a gap
        // in it. skipped when the caller has already persisted the message
        // themselves, so it is never stored twice
        if !req.already_persisted {
            self.sessions
                .append(conversation.id, Message::user(req.message.clone()))
                .await?;
        }

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

        let system = self.render_system_prompt(persona, req).await?;

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
            persona_id: persona.id.0.clone(),
        })
    }

    /// Renders a persona's system prompt with the variables every persona
    /// prompt references: `{{user_name}}` and `{{platform}}`.
    ///
    /// `memories` is always populated, regardless of `persona.memory` -
    /// [`crate::persona::PromptTemplate`] silently ignores a context entry a
    /// prompt never references, so it costs nothing for a persona whose
    /// prompt has no `{{memories}}` placeholder at all, and it means a
    /// prompt that does reference it never fails to render just because a
    /// persona was configured without `MemoryPolicy::User`.
    async fn render_system_prompt(
        &self,
        persona: &Persona,
        req: &AiTurnRequest,
    ) -> Result<String, AiError> {
        let memories = if persona.memory == MemoryPolicy::User {
            self.load_memories_text(req.user_id).await
        } else {
            String::new()
        };

        let context = HashMap::from([
            ("user_name".to_string(), req.user_name.clone()),
            ("platform".to_string(), req.scope.platform.to_string()),
            ("memories".to_string(), memories),
        ]);
        persona.system_prompt.render(&context)
    }

    /// Renders a `{{memories}}` block for `user_id`, best-effort.
    ///
    /// Retrieval is deliberately not the model's job - see
    /// [`crate::tools::RememberTool`]'s own doc comment - so this is the one
    /// and only place a persona's memories are ever read back, once per
    /// turn, rather than a tool call the model would have to think to make.
    /// A failure to load (no store wired, or a transient error) falls back
    /// to a neutral message rather than failing the whole turn over
    /// something this unimportant to get exactly right every single time.
    async fn load_memories_text(&self, user_id: u64) -> String {
        let Some(store) = &self.memory_store else {
            return "(memory is not set up right now)".to_string();
        };

        match store.list(user_id).await {
            Ok(memories) if memories.is_empty() => "Nothing recorded yet.".to_string(),
            Ok(memories) => memories
                .iter()
                .map(|memory| format!("- {}: {}", memory.key, memory.value))
                .collect::<Vec<_>>()
                .join("\n"),
            Err(error) => {
                tracing::warn!(%error, user_id, "couldn't load memories for a turn");
                "(couldn't load memories just now)".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audit::ToolCallRecord,
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

    /// A [`UsageRecorder`] that keeps everything it was asked to record, for
    /// tests to inspect afterward.
    #[derive(Default)]
    struct FakeUsageRecorder {
        records: std::sync::Mutex<Vec<UsageRecord>>,
    }

    impl FakeUsageRecorder {
        fn records(&self) -> Vec<UsageRecord> {
            self.records.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl UsageRecorder for FakeUsageRecorder {
        async fn record(&self, record: UsageRecord) -> Result<(), AiError> {
            self.records.lock().unwrap().push(record);
            Ok(())
        }
    }

    /// A [`UsageRecorder`] that always fails, for testing that a recording
    /// failure never fails the turn it describes.
    struct FailingUsageRecorder;

    #[async_trait::async_trait]
    impl UsageRecorder for FailingUsageRecorder {
        async fn record(&self, _record: UsageRecord) -> Result<(), AiError> {
            Err(AiError::Other("recording storage is down".to_string()))
        }
    }

    /// A [`ToolAuditor`] that keeps everything it was asked to record - the
    /// harness itself already has thorough coverage of auditing behaviour;
    /// this only needs to prove `Ai::with_tool_auditor` actually reaches the
    /// harness it constructs per turn.
    #[derive(Default)]
    struct FakeToolAuditor {
        records: std::sync::Mutex<Vec<ToolCallRecord>>,
    }

    impl FakeToolAuditor {
        fn records(&self) -> Vec<ToolCallRecord> {
            self.records.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl ToolAuditor for FakeToolAuditor {
        async fn record(&self, record: ToolCallRecord) {
            self.records.lock().unwrap().push(record);
        }
    }

    /// A [`MemoryStore`] returning a fixed, scripted answer to `list`, and
    /// recording how many times it was called - so a test can assert both
    /// what got rendered and whether the store was reached at all.
    struct FakeMemoryStore {
        memories: Vec<crate::memory::Memory>,
        list_calls: std::sync::Mutex<usize>,
    }

    impl FakeMemoryStore {
        fn with(memories: Vec<crate::memory::Memory>) -> Self {
            Self {
                memories,
                list_calls: std::sync::Mutex::new(0),
            }
        }

        fn list_call_count(&self) -> usize {
            *self.list_calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl crate::memory::MemoryStore for FakeMemoryStore {
        async fn list(&self, _user_id: u64) -> Result<Vec<crate::memory::Memory>, AiError> {
            *self.list_calls.lock().unwrap() += 1;
            Ok(self.memories.clone())
        }

        async fn record(&self, _user_id: u64, _key: &str, _value: &str) -> Result<(), AiError> {
            Ok(())
        }

        async fn forget(&self, _user_id: u64, _key: &str) -> Result<(), AiError> {
            Ok(())
        }

        async fn wipe(&self, _user_id: u64) -> Result<(), AiError> {
            Ok(())
        }
    }

    /// A [`MemoryStore`] whose `list` always fails, for testing that a
    /// memory-loading failure falls back gracefully rather than failing the
    /// turn.
    struct FailingMemoryStore;

    #[async_trait::async_trait]
    impl crate::memory::MemoryStore for FailingMemoryStore {
        async fn list(&self, _user_id: u64) -> Result<Vec<crate::memory::Memory>, AiError> {
            Err(AiError::Other("memory storage is down".to_string()))
        }

        async fn record(&self, _user_id: u64, _key: &str, _value: &str) -> Result<(), AiError> {
            Ok(())
        }

        async fn forget(&self, _user_id: u64, _key: &str) -> Result<(), AiError> {
            Ok(())
        }

        async fn wipe(&self, _user_id: u64) -> Result<(), AiError> {
            Ok(())
        }
    }

    /// Builds a persona registry with one persona allowed to use
    /// `current_time`, and a matching tool registry - the setup
    /// [`test_ai_wires_a_tool_auditor_through_to_the_harness`] and its
    /// streamed twin need, which the shared `ai_with` helper does not
    /// provide since every other test in this module needs no real tool.
    fn ai_with_current_time(provider: Arc<dyn Provider>) -> Ai {
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
                tools: crate::tools::ToolSelection::named(["current_time"]),
                budget: crate::persona::BudgetConfig::default(),
                memory: MemoryPolicy::None,
                sandbox: crate::persona::SandboxPolicy::default(),
            });
        let providers = ProviderRegistry::from_available(["anthropic".to_string()]);
        let personas = PersonaRegistry::load(&config, &providers).expect("should resolve");

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(crate::tools::CurrentTimeTool));

        Ai::from_parts(
            personas,
            Arc::new(registry),
            Arc::new(InMemorySessionStore::new()),
            Arc::new(FixedProviderSource(provider)),
        )
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
            already_persisted: false,
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
    async fn test_already_persisted_does_not_store_the_message_a_second_time() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_text("ok"));
        let sessions: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());

        // simulates what the web chat surface's send_message server function
        // already did before a stream request ever reaches Ai::turn
        let scope = ConversationScope::new(Platform::Discord, "channel-1");
        let conversation = sessions.load_or_create(&scope, "companion").await.unwrap();
        sessions
            .append(conversation.id, Message::user("already stored"))
            .await
            .unwrap();

        let ai = Ai::from_parts(
            personas_with_memory(MemoryPolicy::Conversation),
            Arc::new(ToolRegistry::new()),
            sessions.clone(),
            Arc::new(FixedProviderSource(provider)),
        );

        let mut req = request("companion", "already stored");
        req.already_persisted = true;
        ai.turn(req).await.expect("should succeed");

        let history = sessions.history(conversation.id, None).await.unwrap();
        let occurrences = history
            .iter()
            .filter(|message| message.text() == "already stored")
            .count();
        assert_eq!(
            occurrences, 1,
            "already_persisted should skip appending the message again, got {history:?}"
        );
    }

    #[tokio::test]
    async fn test_not_already_persisted_still_stores_the_message_as_before() {
        let provider: Arc<dyn Provider> = Arc::new(MockProvider::new().respond_text("ok"));
        let sessions: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let ai = Ai::from_parts(
            personas_with_memory(MemoryPolicy::Conversation),
            Arc::new(ToolRegistry::new()),
            sessions.clone(),
            Arc::new(FixedProviderSource(provider)),
        );

        ai.turn(request("companion", "hello"))
            .await
            .expect("should succeed");

        let scope = ConversationScope::new(Platform::Discord, "channel-1");
        let conversation = sessions.load_or_create(&scope, "companion").await.unwrap();
        let history = sessions.history(conversation.id, None).await.unwrap();
        assert!(
            history.iter().any(|message| message.text() == "hello"),
            "the default (already_persisted: false) should still store the message itself, got \
             {history:?}"
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

    #[tokio::test]
    async fn test_turn_records_usage_on_success() {
        let provider = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![crate::types::ContentBlock::text("hi")],
                crate::types::StopReason::EndTurn,
                Usage::new(100, 100),
            ),
        )));
        let recorder = Arc::new(FakeUsageRecorder::default());
        let ai = ai_with(MemoryPolicy::None, provider).with_usage_recorder(recorder.clone());

        ai.turn(request("companion", "hello"))
            .await
            .expect("should succeed");

        let records = recorder.records();
        assert_eq!(records.len(), 1);
        assert!(records[0].succeeded);
        assert_eq!(records[0].persona_id, "companion");
        assert_eq!(records[0].provider, "anthropic");
        assert_eq!(records[0].model, "claude-opus-5");
        assert_eq!(records[0].user_id, Some(1));
        assert_eq!(records[0].guild_id, None);
        assert!(
            records[0].cost > Cost::ZERO,
            "a priced model with real usage should produce a nonzero cost"
        );
    }

    #[tokio::test]
    async fn test_turn_records_usage_on_failure_reflecting_prior_spend() {
        // iteration one succeeds with real usage and asks for another round
        // (StopReason::ToolUse, no actual tool call in it); iteration two's
        // provider call itself fails - proving the recorded row reflects what
        // iteration one actually spent, not a zeroed-out failure
        let provider = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![crate::types::ContentBlock::text("partial")],
                    crate::types::StopReason::ToolUse,
                    Usage::new(50, 50),
                )))
                .respond_error(AiError::Provider("outage".to_string())),
        );
        let recorder = Arc::new(FakeUsageRecorder::default());
        let ai = ai_with(MemoryPolicy::None, provider).with_usage_recorder(recorder.clone());

        let result = ai.turn(request("companion", "hello")).await;
        assert!(result.is_err(), "the turn itself should still fail");

        let records = recorder.records();
        assert_eq!(records.len(), 1);
        assert!(!records[0].succeeded);
        assert_eq!(records[0].iterations, 1);
        assert!(
            records[0].usage.input_tokens > 0,
            "the first, successful iteration's spend must not be lost just because the second \
             iteration failed: {:?}",
            records[0]
        );
    }

    #[tokio::test]
    async fn test_a_usage_recording_failure_does_not_fail_the_turn() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let ai = ai_with(MemoryPolicy::None, provider)
            .with_usage_recorder(Arc::new(FailingUsageRecorder));

        let result = ai.turn(request("companion", "hello")).await;

        assert!(
            result.is_ok(),
            "a usage recorder failing to write must never surface as a failed turn"
        );
    }

    #[tokio::test]
    async fn test_without_a_usage_recorder_wired_nothing_breaks() {
        // regression guard: every other test in this module wires no recorder at
        // all, so this only makes that default explicit
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let ai = ai_with(MemoryPolicy::None, provider);

        let result = ai.turn(request("companion", "hello")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_turn_streamed_records_usage_on_finish() {
        let provider = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![crate::types::ContentBlock::text("hi")],
                crate::types::StopReason::EndTurn,
                Usage::new(100, 100),
            ),
        )));
        let recorder = Arc::new(FakeUsageRecorder::default());
        let ai = ai_with(MemoryPolicy::None, provider).with_usage_recorder(recorder.clone());

        let _events: Vec<HarnessEvent> = ai
            .turn_streamed(request("companion", "hello"))
            .await
            .expect("should succeed")
            .collect()
            .await;

        let records = recorder.records();
        assert_eq!(records.len(), 1);
        assert!(records[0].succeeded);
        assert!(records[0].cost > Cost::ZERO);
    }

    #[tokio::test]
    async fn test_turn_streamed_records_usage_on_failure() {
        let provider: Arc<dyn Provider> =
            Arc::new(MockProvider::new().respond_error(AiError::Rejected("bad key".to_string())));
        let recorder = Arc::new(FakeUsageRecorder::default());
        let ai = ai_with(MemoryPolicy::None, provider).with_usage_recorder(recorder.clone());

        let _events: Vec<HarnessEvent> = ai
            .turn_streamed(request("companion", "hello"))
            .await
            .expect("should succeed")
            .collect()
            .await;

        let records = recorder.records();
        assert_eq!(records.len(), 1);
        assert!(!records[0].succeeded);
    }

    #[tokio::test]
    async fn test_ai_wires_a_tool_auditor_through_to_the_harness() {
        let provider: Arc<dyn Provider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is some time"),
        );
        let auditor = Arc::new(FakeToolAuditor::default());
        let ai = ai_with_current_time(provider).with_tool_auditor(auditor.clone());

        ai.turn(request("companion", "what time is it"))
            .await
            .expect("should succeed");

        let records = auditor.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tool_name, "current_time");
    }

    #[tokio::test]
    async fn test_ai_wires_a_tool_auditor_through_turn_streamed_too() {
        let provider: Arc<dyn Provider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is some time"),
        );
        let auditor = Arc::new(FakeToolAuditor::default());
        let ai = ai_with_current_time(provider).with_tool_auditor(auditor.clone());

        let _events: Vec<HarnessEvent> = ai
            .turn_streamed(request("companion", "what time is it"))
            .await
            .expect("should succeed")
            .collect()
            .await;

        assert_eq!(auditor.records().len(), 1);
    }

    #[tokio::test]
    async fn test_without_a_tool_auditor_wired_a_tool_using_turn_still_works() {
        let provider: Arc<dyn Provider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is some time"),
        );
        let ai = ai_with_current_time(provider);

        let outcome = ai
            .turn(request("companion", "what time is it"))
            .await
            .expect("should succeed");
        assert_eq!(outcome.text.as_deref(), Some("it is some time"));
    }

    #[tokio::test]
    async fn test_memory_policy_user_gets_real_memories_rendered_into_the_prompt() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let store = Arc::new(FakeMemoryStore::with(vec![crate::memory::Memory {
            key: "favorite_color".to_string(),
            value: "purple".to_string(),
            updated_at: chrono::Utc::now(),
        }]));
        let ai = ai_with(MemoryPolicy::User, provider.clone()).with_memory_store(store);

        ai.turn(request("companion", "hi"))
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        let system = sent.system.as_deref().expect("should have a system prompt");
        assert!(
            system.contains("favorite_color") && system.contains("purple"),
            "the real memory should reach the rendered prompt: {system:?}"
        );
    }

    #[tokio::test]
    async fn test_memory_policy_conversation_never_reaches_the_memory_store() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let store = Arc::new(FakeMemoryStore::with(Vec::new()));
        let ai = ai_with(MemoryPolicy::Conversation, provider).with_memory_store(store.clone());

        ai.turn(request("companion", "hi"))
            .await
            .expect("should succeed");

        assert_eq!(
            store.list_call_count(),
            0,
            "only MemoryPolicy::User should ever read memories"
        );
    }

    #[tokio::test]
    async fn test_memory_policy_none_never_reaches_the_memory_store() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let store = Arc::new(FakeMemoryStore::with(Vec::new()));
        let ai = ai_with(MemoryPolicy::None, provider).with_memory_store(store.clone());

        ai.turn(request("companion", "hi"))
            .await
            .expect("should succeed");

        assert_eq!(store.list_call_count(), 0);
    }

    #[tokio::test]
    async fn test_no_memory_store_wired_falls_back_to_a_neutral_placeholder() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let ai = ai_with(MemoryPolicy::User, provider.clone());

        ai.turn(request("companion", "hi"))
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        let system = sent.system.as_deref().expect("should have a system prompt");
        assert!(
            !system.contains("{{memories}}"),
            "the placeholder itself must never leak into what the model sees: {system:?}"
        );
    }

    #[tokio::test]
    async fn test_no_memories_yet_renders_a_friendly_placeholder_not_emptiness() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let store = Arc::new(FakeMemoryStore::with(Vec::new()));
        let ai = ai_with(MemoryPolicy::User, provider.clone()).with_memory_store(store);

        ai.turn(request("companion", "hi"))
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        let system = sent.system.as_deref().unwrap();
        assert!(
            system.contains("Nothing recorded yet"),
            "a user with no memories yet should get a friendly note, not a blank section: \
             {system:?}"
        );
    }

    #[tokio::test]
    async fn test_a_memory_store_failure_falls_back_gracefully() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let ai =
            ai_with(MemoryPolicy::User, provider).with_memory_store(Arc::new(FailingMemoryStore));

        let result = ai.turn(request("companion", "hi")).await;
        assert!(
            result.is_ok(),
            "a memory-loading failure must not fail the whole turn over something this minor"
        );
    }

    fn crisis_classifier(response_text: &str) -> (CrisisClassifier, Arc<MockProvider>) {
        let provider = Arc::new(MockProvider::new().respond_text(response_text));
        let classifier = CrisisClassifier::new(
            provider.clone(),
            crate::crisis::CrisisPersona::embedded(ModelRef::new("anthropic", "claude-haiku")),
        );
        (classifier, provider)
    }

    #[tokio::test]
    async fn test_memory_policy_user_screens_the_inbound_message_for_crisis() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let (classifier, classifier_provider) = crisis_classifier("NONE");
        let ai = ai_with(MemoryPolicy::User, provider).with_crisis_classifier(classifier);

        ai.turn(request("companion", "just chatting"))
            .await
            .expect("should succeed");

        assert_eq!(
            classifier_provider.request_count(),
            1,
            "a MemoryPolicy::User persona's inbound message should be screened"
        );
    }

    #[tokio::test]
    async fn test_memory_policy_conversation_never_reaches_the_crisis_classifier() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let (classifier, classifier_provider) = crisis_classifier("NONE");
        let ai = ai_with(MemoryPolicy::Conversation, provider).with_crisis_classifier(classifier);

        ai.turn(request("companion", "just chatting"))
            .await
            .expect("should succeed");

        assert_eq!(
            classifier_provider.request_count(),
            0,
            "only MemoryPolicy::User should ever be screened"
        );
    }

    #[tokio::test]
    async fn test_memory_policy_none_never_reaches_the_crisis_classifier() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let (classifier, classifier_provider) = crisis_classifier("NONE");
        let ai = ai_with(MemoryPolicy::None, provider).with_crisis_classifier(classifier);

        ai.turn(request("companion", "just chatting"))
            .await
            .expect("should succeed");

        assert_eq!(classifier_provider.request_count(), 0);
    }

    #[tokio::test]
    async fn test_no_crisis_classifier_wired_the_turn_still_proceeds_normally() {
        let provider = Arc::new(MockProvider::new().respond_text("hi"));
        let ai = ai_with(MemoryPolicy::User, provider);

        let result = ai.turn(request("companion", "just chatting")).await;
        assert!(
            result.is_ok(),
            "no classifier wired should behave exactly as before it existed"
        );
    }

    #[tokio::test]
    async fn test_a_severe_signal_does_not_yet_change_the_turn_s_outcome() {
        // commit 99 only screens and logs - bypassing the normal turn for a
        // reviewed response on a positive signal is a separate, later
        // concern (crisis's own module doc comment explains why)
        let provider = Arc::new(MockProvider::new().respond_text("the ordinary reply"));
        let (classifier, _) = crisis_classifier("SEVERE");
        let ai = ai_with(MemoryPolicy::User, provider).with_crisis_classifier(classifier);

        let outcome = ai
            .turn(request("companion", "anything"))
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("the ordinary reply"));
    }
}
