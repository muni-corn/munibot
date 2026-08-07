//! The agent loop: model to tools to handoff, with budgets and events.
//!
//! This is the one place that drives a [`crate::provider::Provider`] and a
//! [`crate::tools::ToolRegistry`] together. Everything above it - personas,
//! platform adapters, the eventual pipeline - talks to the harness and never to
//! a provider or a tool directly.

pub mod budget;
pub mod event;
pub mod turn;
pub mod validate;

use std::{future::Future, sync::Arc};

pub use budget::{Budget, BudgetTracker};
pub use event::HarnessEvent;
use futures::{StreamExt, stream::BoxStream};
pub use turn::{HandoffSchema, TurnOutcome, TurnRequest, TurnUsage};
pub use validate::validate_tool_arguments;

use crate::{
    audit::{ToolAuditor, ToolCallRecord},
    provider::{Provider, estimate_cost},
    tools::{ToolCtx, ToolRegistry},
    types::{
        AiError, CompletionRequest, ContentBlock, History, Message, Role, StopReason, StreamEvent,
        Usage,
    },
};

/// Where [`Self::dispatch_calls`] sends [`HarnessEvent::ToolStarted`]/
/// [`HarnessEvent::ToolFinished`] as each call starts and finishes.
///
/// `None` for [`Harness::run_turn`], which has no stream to forward events
/// into. [`Harness::run_turn_streamed`] supplies one and drives the receiving
/// end concurrently with the dispatch itself, since `yield` only works
/// directly inside its own `async_stream::stream!` body.
type ToolEventSink<'a> = Option<&'a tokio::sync::mpsc::UnboundedSender<HarnessEvent>>;

/// Drives a [`Provider`] and a [`ToolRegistry`] together into one full agent
/// turn.
///
/// This is the only thing above it - personas, platform adapters, the eventual
/// pipeline - ever needs to hold. Nothing else touches a provider or a tool
/// directly.
pub struct Harness {
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    /// `None` until [`Self::with_auditor`] enables it. Without it, a tool
    /// call is dispatched and its result used exactly as before - auditing
    /// is a side effect, never a precondition of a call actually happening.
    auditor: Option<Arc<dyn ToolAuditor>>,
}

impl Harness {
    /// Builds a harness over a provider and the tool registry it may draw from.
    pub fn new(provider: Arc<dyn Provider>, tools: Arc<ToolRegistry>) -> Self {
        Self {
            provider,
            tools,
            auditor: None,
        }
    }

    /// Enables auditing every tool call this harness dispatches.
    pub fn with_auditor(mut self, auditor: Arc<dyn ToolAuditor>) -> Self {
        self.auditor = Some(auditor);
        self
    }

    /// Runs one full turn: call the provider, dispatch every tool call it asks
    /// for, and repeat until it answers with plain text.
    ///
    /// Handoff validation and cancellation arrive in later commits; this one
    /// covers the loop's basic shape, tool dispatch and argument
    /// validation, and general budget enforcement.
    pub async fn run_turn(&self, request: TurnRequest) -> Result<TurnOutcome, AiError> {
        self.run_turn_recording_usage(request).await.0
    }

    /// Runs one full turn exactly as [`Self::run_turn`] does, additionally
    /// returning what was spent **regardless of whether the turn succeeded**.
    ///
    /// `TurnOutcome` already carries usage and cost on the success path;
    /// [`TurnUsage`] exists for the failure path; a turn that errors on its
    /// ninth iteration still spent the first eight, and a caller recording
    /// cost (an `ai_usage` row, say) needs that even when the turn itself
    /// failed. `run_turn` is a plain wrapper over this that discards the
    /// second half of the tuple, so its own behaviour is unchanged.
    pub async fn run_turn_recording_usage(
        &self,
        request: TurnRequest,
    ) -> (Result<TurnOutcome, AiError>, TurnUsage) {
        let mut tracker = BudgetTracker::new(request.budget.clone());
        let result = self.run_turn_inner(&request, &mut tracker).await;
        let usage = TurnUsage {
            usage: tracker.usage(),
            cost: tracker.cost(),
            iterations: tracker.iterations(),
        };
        (result, usage)
    }

    /// The loop body shared by [`Self::run_turn_recording_usage`] and, through
    /// it, [`Self::run_turn`].
    ///
    /// Takes `tracker` by mutable reference rather than owning it, so it
    /// keeps accumulating right up to whatever point this returns - including
    /// an early `Err` - and the caller can still read it afterward.
    async fn run_turn_inner(
        &self,
        request: &TurnRequest,
        tracker: &mut BudgetTracker,
    ) -> Result<TurnOutcome, AiError> {
        let mut tool_schemas = self
            .tools
            .schemas_for(&request.tools, request.ctx.granted_tier);
        if let Some(handoff) = &request.handoff {
            // the handoff tool is synthetic: it exists only in the schema list offered to
            // the provider, never in the registry, since it terminates the turn
            // rather than doing anything a Tool impl could invoke
            tool_schemas.push(crate::types::ToolSchema::new(
                handoff.tool_name.clone(),
                handoff.description.clone(),
                handoff.schema.clone(),
            ));
        }

        let mut history = request.history.clone();
        let mut tool_retries = 0usize;
        let mut last_text = String::new();

        loop {
            // don't start another round trip once a prior iteration already spent the
            // budget
            if let Err(reason) = tracker.check() {
                return Ok(Self::truncated_outcome(last_text, tracker, &reason));
            }
            if request.ctx.is_cancelled() {
                return Err(AiError::Cancelled);
            }

            let mut completion_request =
                CompletionRequest::new(request.model.clone(), history.clone())
                    .with_tools(tool_schemas.clone())
                    .with_params(request.params.clone());
            if let Some(system) = &request.system {
                completion_request = completion_request.with_system(system.clone());
            }

            let response = race_cancellation(
                &request.ctx.cancellation,
                Err(AiError::Cancelled),
                self.provider.complete(completion_request),
            )
            .await?;
            tracker.record(
                response.usage,
                estimate_cost(&request.model, &response.usage),
            );
            last_text = response.text();

            // this iteration alone may have just spent what was left
            if let Err(reason) = tracker.check() {
                return Ok(Self::truncated_outcome(last_text, tracker, &reason));
            }

            if !response.stop_reason.wants_another_iteration() {
                if let Some(handoff) = &request.handoff {
                    // the model answered directly instead of calling the handoff tool - not a
                    // valid way to finish when one is required, so nudge it and try again rather
                    // than silently accepting plain text as the answer
                    tool_retries += 1;
                    if let Some(max) = request.budget.max_tool_retries
                        && tool_retries > max
                    {
                        return Err(AiError::SchemaViolation(format!(
                            "gave up after {tool_retries} attempts without a valid `{}` handoff :<",
                            handoff.tool_name
                        )));
                    }
                    history.push(Message::user(format!(
                            "you must call the `{}` tool to finish this turn, rather than \
                             answering                          directly :<",
                            handoff.tool_name
                        )));
                    continue;
                }
                return Ok(TurnOutcome::text(
                    response.text(),
                    tracker.usage(),
                    tracker.cost(),
                    tracker.iterations(),
                ));
            }

            if let Some(outcome) = self
                .handle_tool_calls(
                    response.content,
                    request,
                    tracker,
                    &mut tool_retries,
                    &mut history,
                    None,
                )
                .await?
            {
                return Ok(outcome);
            }
        }
    }

    /// Runs one full turn exactly as [`Self::run_turn`] does, emitting
    /// [`HarnessEvent`]s as it goes rather than returning a single final
    /// [`TurnOutcome`].
    ///
    /// Takes `self` behind an `Arc` rather than `&self`: the returned stream
    /// must be `'static` so it can be handed off and polled independently
    /// of the caller's stack frame (stored, spawned, forwarded to a
    /// platform adapter), which means it cannot merely borrow `self`.
    ///
    /// Text and reasoning deltas are forwarded live as
    /// [`HarnessEvent::TextDelta`] and [`HarnessEvent::Thinking`]. Tool
    /// call announcements are accumulated (a provider may split one call's
    /// JSON arguments across many deltas with no id repeated on each one) into
    /// the same shape [`Self::handle_tool_calls`] expects, so dispatch,
    /// validation, and handoff handling are shared with the non-streaming
    /// path rather than duplicated.
    ///
    /// Per-tool [`HarnessEvent::ToolStarted`]/[`HarnessEvent::ToolFinished`]
    /// events are forwarded live too, via an unbounded channel: `yield` only
    /// works directly inside this `async_stream::stream!` body, so
    /// [`Self::handle_tool_calls`] cannot emit them itself. Instead, its
    /// future is driven with `tokio::select!` alongside the channel's
    /// receiver, so an event queued by a concurrently-dispatched tool call is
    /// yielded the moment it arrives rather than only once every call in the
    /// batch has finished. `run_turn` passes `None` for the same parameter,
    /// since it has no stream to forward events into.
    ///
    /// `TurnStarted` also carries the model reference rather than a persona
    /// name, since `TurnRequest` has no persona field - the eventual
    /// persona-aware service handle can override this when it wraps the
    /// stream.
    pub fn run_turn_streamed(
        self: Arc<Self>,
        request: TurnRequest,
    ) -> BoxStream<'static, HarnessEvent> {
        let events = async_stream::stream! {
            yield HarnessEvent::TurnStarted { persona: request.model.to_string() };

            let mut tool_schemas = self
                .tools
                .schemas_for(&request.tools, request.ctx.granted_tier);
            if let Some(handoff) = &request.handoff {
                tool_schemas.push(crate::types::ToolSchema::new(
                    handoff.tool_name.clone(),
                    handoff.description.clone(),
                    handoff.schema.clone(),
                ));
            }

            let mut history = request.history.clone();
            let mut tracker = BudgetTracker::new(request.budget.clone());
            let mut tool_retries = 0usize;

            'turn: loop {
                if let Err(reason) = tracker.check() {
                    yield HarnessEvent::Failed(reason);
                    return;
                }
                if request.ctx.is_cancelled() {
                    yield HarnessEvent::Failed(AiError::Cancelled);
                    return;
                }

                let mut completion_request =
                    CompletionRequest::new(request.model.clone(), history.clone())
                        .with_tools(tool_schemas.clone())
                        .with_params(request.params.clone());
                if let Some(system) = &request.system {
                    completion_request = completion_request.with_system(system.clone());
                }

                let stream_result = race_cancellation(
                    &request.ctx.cancellation,
                    Err(AiError::Cancelled),
                    self.provider.stream(completion_request),
                )
                .await;

                let mut inner = match stream_result {
                    Ok(inner) => inner,
                    Err(error) => {
                        yield HarnessEvent::Failed(error);
                        return;
                    }
                };

                // reconstruct an equivalent of a whole CompletionResponse from the stream, since
                // dispatch and budget accounting both need the complete picture, not deltas
                let mut text = String::new();
                let mut thinking = String::new();
                let mut tool_calls: Vec<(String, String, String)> = Vec::new();
                let mut current: Option<usize> = None;
                let mut usage = Usage::default();
                let mut stop_reason = StopReason::EndTurn;

                loop {
                    let next = race_cancellation(
                        &request.ctx.cancellation,
                        Some(Err(AiError::Cancelled)),
                        inner.next(),
                    )
                    .await;

                    match next {
                        None => break,
                        Some(Err(error)) => {
                            yield HarnessEvent::Failed(error);
                            return;
                        }
                        Some(Ok(StreamEvent::TextDelta { text: delta })) => {
                            text.push_str(&delta);
                            yield HarnessEvent::TextDelta(delta);
                        }
                        Some(Ok(StreamEvent::ThinkingDelta { thinking: delta })) => {
                            thinking.push_str(&delta);
                            yield HarnessEvent::Thinking(delta);
                        }
                        Some(Ok(StreamEvent::ToolUseStart { call_id, name })) => {
                            tool_calls.push((call_id, name, String::new()));
                            current = Some(tool_calls.len() - 1);
                        }
                        Some(Ok(StreamEvent::ToolUseDelta { partial_json })) => {
                            if let Some(index) = current {
                                tool_calls[index].2.push_str(&partial_json);
                            }
                        }
                        Some(Ok(StreamEvent::ToolUseEnd)) => {
                            current = None;
                        }
                        Some(Ok(StreamEvent::Usage { usage: latest })) => {
                            usage = latest;
                        }
                        Some(Ok(StreamEvent::Done { stop_reason: reason })) => {
                            stop_reason = reason;
                            break;
                        }
                    }
                }

                tracker.record(usage, estimate_cost(&request.model, &usage));
                yield HarnessEvent::IterationComplete { iteration: tracker.iterations(), usage };

                if let Err(reason) = tracker.check() {
                    yield HarnessEvent::Failed(reason);
                    return;
                }

                let mut content = Vec::new();
                if !thinking.is_empty() {
                    content.push(ContentBlock::thinking(thinking));
                }
                if !text.is_empty() {
                    content.push(ContentBlock::text(text));
                }
                for (call_id, name, json_buffer) in tool_calls {
                    // an empty or malformed buffer becomes Null rather than failing the whole
                    // turn - validate_tool_arguments rejects it with a model-visible error next,
                    // exactly as a genuinely invalid call would be
                    let arguments = serde_json::from_str(&json_buffer).unwrap_or(serde_json::Value::Null);
                    content.push(ContentBlock::tool_use(call_id, name, arguments));
                }

                if !stop_reason.wants_another_iteration() {
                    if let Some(handoff) = &request.handoff {
                        tool_retries += 1;
                        if let Some(max) = request.budget.max_tool_retries
                            && tool_retries > max
                        {
                            yield HarnessEvent::Failed(AiError::SchemaViolation(format!(
                                "gave up after {tool_retries} attempts without a valid `{}` \
                                 handoff :<",
                                handoff.tool_name
                            )));
                            return;
                        }
                        history.push(Message::user(format!(
                            "you must call the `{}` tool to finish this turn, rather than \
                             answering directly :<",
                            handoff.tool_name
                        )));
                        continue 'turn;
                    }
                    yield HarnessEvent::TurnFinished { usage: tracker.usage(), cost: tracker.cost() };
                    return;
                }

                let (tool_events_tx, mut tool_events_rx) =
                    tokio::sync::mpsc::unbounded_channel::<HarnessEvent>();
                // scoped so `handle_fut` - and its borrow of `tool_events_tx` - is
                // dropped before we drop our own copy of the sender below
                let result = {
                    let handle_fut = self.handle_tool_calls(
                        content,
                        &request,
                        &tracker,
                        &mut tool_retries,
                        &mut history,
                        Some(&tool_events_tx),
                    );
                    tokio::pin!(handle_fut);

                    loop {
                        tokio::select! {
                            biased;
                            event = tool_events_rx.recv() => {
                                if let Some(event) = event {
                                    yield event;
                                }
                            }
                            outcome = &mut handle_fut => {
                                break outcome;
                            }
                        }
                    }
                };
                // drained after `handle_fut` is gone, catching any event queued in the
                // same instant the future resolved but not yet read by the loop above
                drop(tool_events_tx);
                while let Ok(event) = tool_events_rx.try_recv() {
                    yield event;
                }

                match result {
                    Ok(Some(outcome)) => {
                        if let Some(payload) = outcome.handoff {
                            yield HarnessEvent::Handoff(payload);
                        }
                        return;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        yield HarnessEvent::Failed(error);
                        return;
                    }
                }
            }
        };

        Box::pin(events)
    }

    /// Resolves and validates every tool call in one response, dispatches
    /// everything that passed validation, and appends the results to
    /// `history`.
    ///
    /// An unknown tool name or arguments that fail schema validation never
    /// reach `invoke()` at all, and the model sees exactly why, so it can
    /// correct itself on the next iteration.
    ///
    /// Returns `Some(outcome)` when a valid handoff call ended the turn right
    /// here, or `None` to let the caller continue looping. Shared between
    /// the non-streaming and streaming turn loops, so this logic exists in
    /// exactly one place.
    async fn handle_tool_calls(
        &self,
        content: Vec<ContentBlock>,
        request: &TurnRequest,
        tracker: &BudgetTracker,
        tool_retries: &mut usize,
        history: &mut History,
        events: ToolEventSink<'_>,
    ) -> Result<Option<TurnOutcome>, AiError> {
        // capture what each call needs before content moves into history below
        let calls: Vec<(String, String, serde_json::Value)> = content
            .iter()
            .filter_map(|block| block.as_tool_use())
            .map(|(call_id, name, arguments)| {
                (call_id.to_string(), name.to_string(), arguments.clone())
            })
            .collect();

        history.push(Message::new(Role::Assistant, content));

        let mut pending: Vec<Option<ContentBlock>> = calls.iter().map(|_| None).collect();
        let mut dispatchable: Vec<(
            usize,
            String,
            Arc<dyn crate::tools::Tool>,
            serde_json::Value,
        )> = Vec::new();

        for (index, (call_id, name, arguments)) in calls.iter().enumerate() {
            if let Some(handoff) = &request.handoff
                && name == &handoff.tool_name
            {
                match validate_tool_arguments(&handoff.schema, arguments) {
                    Ok(()) => {
                        return Ok(Some(TurnOutcome::handoff(
                            arguments.clone(),
                            tracker.usage(),
                            tracker.cost(),
                            tracker.iterations(),
                        )));
                    }
                    Err(validation_error) => {
                        *tool_retries += 1;
                        if let Some(max) = request.budget.max_tool_retries
                            && *tool_retries > max
                        {
                            return Err(AiError::SchemaViolation(format!(
                                "gave up after {tool_retries} invalid handoff attempts :< \
                                 {validation_error}"
                            )));
                        }
                        pending[index] = Some(ContentBlock::tool_error(
                            call_id.clone(),
                            format!("invalid handoff payload :< {validation_error}"),
                        ));
                    }
                }
                continue;
            }

            match self.tools.get(name) {
                None => {
                    let available = self.tools.names().join(", ");
                    pending[index] = Some(ContentBlock::tool_error(
                        call_id.clone(),
                        format!("no such tool {name:?} :< available tools are: {available}"),
                    ));
                }
                Some(tool) => match validate_tool_arguments(&tool.input_schema(), arguments) {
                    Ok(()) => {
                        dispatchable.push((index, call_id.clone(), tool, arguments.clone()));
                    }
                    Err(validation_error) => {
                        *tool_retries += 1;
                        if let Some(max) = request.budget.max_tool_retries
                            && *tool_retries > max
                        {
                            return Err(AiError::SchemaViolation(format!(
                                "gave up after {tool_retries} invalid tool calls in this turn :< \
                                 {validation_error}"
                            )));
                        }
                        pending[index] = Some(ContentBlock::tool_error(
                            call_id.clone(),
                            format!("invalid arguments for {name:?} :< {validation_error}"),
                        ));
                    }
                },
            }
        }

        let to_dispatch: Vec<(Arc<dyn crate::tools::Tool>, serde_json::Value)> = dispatchable
            .iter()
            .map(|(_, _, tool, arguments)| (Arc::clone(tool), arguments.clone()))
            .collect();
        let outcomes = self
            .dispatch_calls(&to_dispatch, &request.ctx, tracker, events)
            .await;

        for ((index, call_id, ..), outcome) in dispatchable.into_iter().zip(outcomes) {
            pending[index] = Some(match outcome {
                crate::tools::ToolOutcome::Ok(text) => ContentBlock::tool_result(call_id, text),
                crate::tools::ToolOutcome::Err(text) => ContentBlock::tool_error(call_id, text),
                crate::tools::ToolOutcome::Fatal(error) => return Err(error),
            });
        }

        let results: Vec<ContentBlock> = pending
            .into_iter()
            .map(|result| {
                result.expect("every call index is set during either validation or dispatch")
            })
            .collect();
        history.push(Message::tool_results(results));

        Ok(None)
    }

    /// Builds the outcome for a turn that stopped early because its budget ran
    /// out, rather than because the model finished answering.
    ///
    /// A partial answer beats no answer: whatever text the last response
    /// carried is kept and marked as truncated, naming which limit was hit,
    /// rather than the turn failing outright. When no text has been
    /// produced yet at all - a degenerately tight budget tripping before the
    /// first response even arrives - the marker stands alone.
    fn truncated_outcome(
        last_text: String,
        tracker: &BudgetTracker,
        reason: &AiError,
    ) -> TurnOutcome {
        let text = if last_text.is_empty() {
            format!("(no response yet :< {reason})")
        } else {
            format!("{last_text}\n\n(response truncated :< {reason})")
        };
        TurnOutcome::text(text, tracker.usage(), tracker.cost(), tracker.iterations())
    }

    /// Dispatches every already-resolved, already-validated call, preserving
    /// result order to match call order regardless of which ones ran
    /// concurrently.
    ///
    /// Calls to a tool that reports [`crate::tools::Tool::is_serial`] run one
    /// at a time, in their relative order; every other call runs
    /// concurrently via [`futures::future::join_all`]. A tool with shared
    /// mutable state across calls - a persistent shell session inside one
    /// sandbox is the motivating case - is the only thing that needs the serial
    /// path; independent calls (a search alongside a file read) gain
    /// nothing from being serialized.
    ///
    /// A `Fatal` outcome is not short-circuited: every already-dispatched call
    /// in this batch is allowed to finish before `run_turn` aborts the
    /// turn, since cancelling siblings mid-flight is a separate concern for
    /// a later commit.
    async fn dispatch_calls(
        &self,
        calls: &[(Arc<dyn crate::tools::Tool>, serde_json::Value)],
        ctx: &ToolCtx,
        tracker: &BudgetTracker,
        events: ToolEventSink<'_>,
    ) -> Vec<crate::tools::ToolOutcome> {
        let mut parallel_indices = Vec::new();
        let mut serial_indices = Vec::new();

        for (index, (tool, _)) in calls.iter().enumerate() {
            if tool.is_serial() {
                serial_indices.push(index);
            } else {
                parallel_indices.push(index);
            }
        }

        // one snapshot for the whole parallel batch: truly concurrent calls
        // cannot observe each other's spend regardless of how this is
        // computed, so there is nothing more precise to give them
        let parallel_ctx = ctx_for_dispatch(tracker, ctx);
        // each parallel call announces itself as soon as `join_all` polls it - not
        // once every call in the batch has finished - so `events` must be captured
        // by every future in the batch, not sent once up front by this function
        let parallel_futures = parallel_indices
            .iter()
            .map(|&index| self.invoke_and_audit(calls, index, &parallel_ctx, events));
        let parallel_outcomes = futures::future::join_all(parallel_futures).await;

        // recomputed fresh before *every* serial call, not once for the whole
        // batch: is_serial's entire point is that these run one at a time, so
        // each one must see what the one before it in this same batch actually
        // spent (via ctx.delegation_spend, updated synchronously before the
        // previous await returns) - not the same stale snapshot every sibling
        // in a naively-shared batch ctx would otherwise see
        let mut serial_outcomes = Vec::with_capacity(serial_indices.len());
        for &index in &serial_indices {
            let serial_ctx = ctx_for_dispatch(tracker, ctx);
            serial_outcomes.push(
                self.invoke_and_audit(calls, index, &serial_ctx, events)
                    .await,
            );
        }

        let mut outcomes: Vec<Option<crate::tools::ToolOutcome>> =
            (0..calls.len()).map(|_| None).collect();
        for (index, outcome) in parallel_indices.into_iter().zip(parallel_outcomes) {
            outcomes[index] = Some(outcome);
        }
        for (index, outcome) in serial_indices.into_iter().zip(serial_outcomes) {
            outcomes[index] = Some(outcome);
        }

        outcomes
            .into_iter()
            .map(|outcome| {
                outcome
                    .expect("every call index is assigned by exactly one of the two groups above")
            })
            .collect()
    }

    /// Invokes `calls[index]`, timing it and auditing the result - shared by
    /// both the parallel and serial dispatch loops in [`Self::dispatch_calls`]
    /// so neither can forget to record.
    ///
    /// Auditing happens here specifically, at the point a real
    /// [`crate::tools::Tool::invoke`] runs, and nowhere earlier in
    /// [`Self::handle_tool_calls`]. An unknown tool name, invalid arguments,
    /// or an invalid handoff payload are all rejected before a call ever
    /// reaches here - there is no real invocation to time, and the model
    /// already sees the rejection in its own tool result. Auditing only what
    /// actually ran keeps `ai_tool_calls` a record of real calls, not a
    /// second copy of what the conversation transcript already shows.
    async fn invoke_and_audit(
        &self,
        calls: &[(Arc<dyn crate::tools::Tool>, serde_json::Value)],
        index: usize,
        ctx: &ToolCtx,
        events: ToolEventSink<'_>,
    ) -> crate::tools::ToolOutcome {
        let (tool, arguments) = &calls[index];
        // the delegate tool's own arguments are otherwise opaque here - see
        // crate::tools::delegate::NAME's own doc comment for why this reads
        // them directly rather than deserializing into its private args type
        let is_delegation = tool.name() == crate::tools::delegate::NAME;
        let delegation_persona = || {
            arguments
                .get("persona")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?")
                .to_string()
        };

        // sent before any `.await` on the call itself, so a batch of parallel
        // calls all announce themselves as soon as this future is first
        // polled, rather than one at a time as each happens to finish
        if let Some(events) = events {
            let event = if is_delegation {
                HarnessEvent::DelegationStarted {
                    persona: delegation_persona(),
                    task: arguments
                        .get("task")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }
            } else {
                HarnessEvent::ToolStarted {
                    name: tool.name().to_string(),
                }
            };
            let _ = events.send(event);
        }

        let started = std::time::Instant::now();
        let outcome = race_cancellation(
            &ctx.cancellation,
            crate::tools::ToolOutcome::Fatal(AiError::Cancelled),
            tool.invoke(arguments.clone(), ctx),
        )
        .await;
        let duration = started.elapsed();

        if let Some(events) = events {
            let ok = matches!(outcome, crate::tools::ToolOutcome::Ok(_));
            let event = if is_delegation {
                HarnessEvent::DelegationFinished {
                    persona: delegation_persona(),
                    ok,
                }
            } else {
                HarnessEvent::ToolFinished {
                    name: tool.name().to_string(),
                    duration,
                    ok,
                    result: crate::audit::outcome_text(&outcome),
                }
            };
            let _ = events.send(event);
        }

        if let Some(auditor) = &self.auditor {
            let record = ToolCallRecord::from_outcome(
                ctx.conversation_id,
                tool.name(),
                arguments,
                &outcome,
                duration,
            );
            auditor.record(record).await;
        }

        outcome
    }
}

/// The ctx one dispatch should actually see: `tracker`'s own live remaining
/// budget, its cost ceiling further reduced by whatever
/// [`ToolCtx::delegation_spend`] already holds.
///
/// Recomputed fresh by [`Harness::dispatch_calls`] before every individual
/// serial call (not once for a whole batch) - see that method's own doc
/// comment for why a stale, once-per-batch snapshot would defeat the entire
/// point of a serial dispatch order for a cost-sensitive tool like
/// `delegate`.
fn ctx_for_dispatch(tracker: &BudgetTracker, ctx: &ToolCtx) -> ToolCtx {
    let mut remaining_budget = tracker.remaining();
    if let Some(max_cost) = remaining_budget.max_cost {
        let already_spent = ctx
            .delegation_spend
            .load(std::sync::atomic::Ordering::SeqCst);
        remaining_budget.max_cost = Some(crate::types::Cost((max_cost.0 - already_spent).max(0)));
    }
    ToolCtx {
        remaining_budget,
        ..ctx.clone()
    }
}

/// Races `future` against `cancellation`, resolving to `on_cancel` if the token
/// fires first.
///
/// `biased` so an already-cancelled token is detected immediately rather than
/// by chance: without it, `select!` polls its branches in random order, and a
/// future that happens to be ready on the same poll as an already-fired
/// cancellation could still complete instead of being interrupted.
/// Dropping `future` when cancellation wins is what stops it from running any
/// further - the standard Rust async cancellation story, and why this never
/// leaks a tool that keeps running in the background after `run_turn` has
/// already returned.
async fn race_cancellation<T>(
    cancellation: &tokio_util::sync::CancellationToken,
    on_cancel: T,
    future: impl Future<Output = T>,
) -> T {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => on_cancel,
        value = future => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        provider::MockProvider,
        tools::{ConversationId, Platform, RiskTier, ToolCtx, ToolSelection},
        types::{History, Message, ModelRef, Usage},
    };

    fn ctx(granted_tier: RiskTier) -> ToolCtx {
        ToolCtx {
            user_id: 1,
            platform: Platform::Discord,
            granted_tier,
            guild_id: None,
            conversation_id: ConversationId(1),
            cancellation: tokio_util::sync::CancellationToken::new(),
            delegation_depth: 0,
            remaining_budget: crate::harness::Budget::default(),
            delegation_spend: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
        }
    }

    fn request() -> TurnRequest {
        TurnRequest::new(
            ModelRef::new("anthropic", "claude-opus-5"),
            History::from(vec![Message::user("hi")]),
            ctx(RiskTier::Safe),
        )
    }

    #[tokio::test]
    async fn test_run_turn_returns_text_on_end_turn() {
        let provider = Arc::new(MockProvider::new().respond_text("hello there"));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let outcome = harness.run_turn(request()).await.expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("hello there"));
        assert_eq!(outcome.iterations, 1);
        assert!(!outcome.is_handoff());
    }

    #[tokio::test]
    async fn test_run_turn_computes_cost_from_usage() {
        let provider = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![crate::types::ContentBlock::text("hi")],
                crate::types::StopReason::EndTurn,
                crate::types::Usage::new(1_000_000, 1_000_000),
            ),
        )));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let outcome = harness.run_turn(request()).await.expect("should succeed");

        assert!(
            outcome.cost > crate::types::Cost::ZERO,
            "a priced model with real usage should produce a nonzero cost"
        );
    }

    #[tokio::test]
    async fn test_run_turn_recording_usage_matches_the_outcome_on_success() {
        let provider = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![crate::types::ContentBlock::text("hi")],
                crate::types::StopReason::EndTurn,
                crate::types::Usage::new(100, 100),
            ),
        )));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let (result, usage) = harness.run_turn_recording_usage(request()).await;
        let outcome = result.expect("should succeed");

        assert_eq!(usage.usage, outcome.usage);
        assert_eq!(usage.cost, outcome.cost);
        assert_eq!(usage.iterations, outcome.iterations);
    }

    #[tokio::test]
    async fn test_run_turn_recording_usage_reflects_spend_up_to_a_fatal_failure() {
        // three tool-use responses in a row exhaust the default budget's
        // max_tool_retries (3) via repeated invalid arguments, aborting the turn -
        // but every one of those round trips still cost real usage first
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        let mut builder = MockProvider::new();
        for _ in 0..4 {
            builder = builder.respond(Ok(crate::types::CompletionResponse::new(
                vec![crate::types::ContentBlock::tool_use(
                    "c1",
                    "search",
                    serde_json::json!({}),
                )],
                crate::types::StopReason::ToolUse,
                Usage::new(10, 10),
            )));
        }
        let provider: Arc<MockProvider> = Arc::new(builder);

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let (result, usage) = harness.run_turn_recording_usage(turn_request).await;

        assert!(result.is_err(), "the turn itself should still fail");
        assert!(
            usage.usage.input_tokens > 0,
            "every round trip before the abort spent real usage, and that must not be lost just \
             because the turn ultimately failed: {usage:?}"
        );
        assert_eq!(
            usage.iterations, 4,
            "all four attempted round trips should be counted"
        );
    }

    #[tokio::test]
    async fn test_run_turn_recording_usage_is_zero_when_cancelled_before_the_first_call() {
        let provider = Arc::new(MockProvider::new());
        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));

        let turn_request = request();
        turn_request.ctx.cancellation.cancel();

        let (result, usage) = harness.run_turn_recording_usage(turn_request).await;

        assert!(matches!(result, Err(AiError::Cancelled)));
        assert_eq!(usage, TurnUsage::default());
        assert_eq!(provider.request_count(), 0);
    }

    #[tokio::test]
    async fn test_run_turn_propagates_provider_errors() {
        let provider =
            Arc::new(MockProvider::new().respond_error(AiError::Rejected("bad key".to_string())));
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let result = harness.run_turn(request()).await;
        assert!(result.is_err());
    }

    /// Always succeeds, echoing a fixed reply so tests can assert on it
    /// downstream.
    struct EchoTool {
        name: &'static str,
        reply: &'static str,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "echoes a fixed reply"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::ok(self.reply)
        }
    }

    /// Records the `remaining_budget` it was invoked with on every call, so a
    /// test can assert it reflects the live tracker rather than whatever
    /// `TurnRequest.ctx` carried at turn start.
    struct CtxCapturingTool {
        captured: std::sync::Arc<std::sync::Mutex<Vec<crate::harness::Budget>>>,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for CtxCapturingTool {
        fn name(&self) -> &str {
            "capture"
        }

        fn description(&self) -> &str {
            "captures the remaining budget it was invoked with"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            self.captured
                .lock()
                .unwrap()
                .push(ctx.remaining_budget.clone());
            crate::tools::ToolOutcome::ok("captured")
        }
    }

    /// A serial tool standing in for `delegate` without needing a real
    /// `Delegator`: on every call, records the cost ceiling it was invoked
    /// with, then reports `spend_per_call` micros into
    /// `ctx.delegation_spend` - exactly what `Ai::delegate` itself does
    /// after a real nested turn finishes.
    struct SpendingSerialTool {
        spend_per_call: i64,
        observed_max_cost: std::sync::Arc<std::sync::Mutex<Vec<Option<i64>>>>,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for SpendingSerialTool {
        fn name(&self) -> &str {
            "spend"
        }

        fn description(&self) -> &str {
            "spends a fixed amount every time it's called"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn is_serial(&self) -> bool {
            true
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            self.observed_max_cost
                .lock()
                .unwrap()
                .push(ctx.remaining_budget.max_cost.map(|cost| cost.0));
            ctx.delegation_spend
                .fetch_add(self.spend_per_call, std::sync::atomic::Ordering::SeqCst);
            crate::tools::ToolOutcome::ok("spent")
        }
    }

    #[tokio::test]
    async fn test_sequential_serial_calls_in_one_batch_see_each_others_delegation_spend() {
        // two calls to a cost-tracking serial tool, batched into the same
        // iteration - is_serial's entire point is that these run one at a
        // time, so the second must see a smaller remaining budget than the
        // first, reflecting exactly what the first one just spent
        let observed_max_cost = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SpendingSerialTool {
            spend_per_call: 300_000,
            observed_max_cost: observed_max_cost.clone(),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "spend", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "spend", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["spend"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.budget = crate::harness::Budget {
            max_cost: Some(crate::types::Cost::from_micros(1_000_000)),
            ..Budget::default()
        };

        let harness = Harness::new(provider, Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let observed = observed_max_cost.lock().unwrap();
        assert_eq!(observed.len(), 2, "both calls should have run");
        assert_eq!(
            observed[0],
            Some(1_000_000),
            "the first call should see the full ceiling - nothing spent yet"
        );
        assert_eq!(
            observed[1],
            Some(700_000),
            "the second call should see the ceiling reduced by exactly what the first call just \
             spent, not the same stale allowance"
        );
    }

    /// Requires a string `query` argument; echoes it back on success.
    struct SchemaedTool;

    #[async_trait::async_trait]
    impl crate::tools::Tool for SchemaedTool {
        fn name(&self) -> &str {
            "search"
        }

        fn description(&self) -> &str {
            "requires a query argument"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            })
        }

        async fn invoke(
            &self,
            input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::ok(format!("searched for {}", input["query"]))
        }
    }

    /// Always aborts the turn, for testing that a Fatal outcome stops the loop.
    struct FatalTool;

    #[async_trait::async_trait]
    impl crate::tools::Tool for FatalTool {
        fn name(&self) -> &str {
            "explode"
        }

        fn description(&self) -> &str {
            "always fails fatally"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            crate::tools::ToolOutcome::fatal(AiError::Cancelled)
        }
    }

    #[tokio::test]
    async fn test_run_turn_dispatches_a_registered_tool_and_returns_final_text() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("it is 12:00"));
        assert_eq!(
            outcome.iterations, 2,
            "one tool round trip plus the final text round trip"
        );

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        assert!(
            matches!(last_message.role, crate::types::Role::Tool),
            "the tool result should have been appended as a Tool-role message"
        );
        let ContentBlock::ToolResult { content, .. } = &last_message.content[0] else {
            panic!("expected the appended message to carry a tool result block");
        };
        assert_eq!(
            content, "12:00",
            "the tool's own reply should be what the model sees back"
        );
    }

    #[tokio::test]
    async fn test_run_turn_unknown_tool_is_recoverable_not_fatal() {
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "does_not_exist", serde_json::json!({}))
                .respond_text("okay, moving on"),
        );

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(request())
            .await
            .expect("an unknown tool must not be fatal");

        assert_eq!(outcome.text.as_deref(), Some("okay, moving on"));

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        let ContentBlock::ToolResult {
            content, is_error, ..
        } = &last_message.content[0]
        else {
            panic!("expected the appended message to carry a tool result block");
        };
        assert!(
            *is_error,
            "an unknown tool should be reported as an error result"
        );
        assert!(
            content.contains("no such tool"),
            "the model should see why its call failed, so it can pick a real tool next time: \
             {content:?}"
        );
    }

    #[tokio::test]
    async fn test_run_turn_fatal_tool_outcome_aborts_the_turn() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FatalTool));

        let provider: Arc<MockProvider> =
            Arc::new(MockProvider::new().respond_tool_use("c1", "explode", serde_json::json!({})));
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let result = harness.run_turn(turn_request).await;

        assert!(
            result.is_err(),
            "a Fatal tool outcome must abort the whole turn"
        );
    }

    #[tokio::test]
    async fn test_run_turn_dispatches_every_call_in_a_multi_tool_response() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "first",
            reply: "one",
        }));
        registry.register(Arc::new(EchoTool {
            name: "second",
            reply: "two",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "first", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "second", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("both done"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["first", "second"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("both done"));

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        assert_eq!(
            last_message.content.len(),
            2,
            "both tool calls from the one response should produce one result each"
        );
    }

    #[tokio::test]
    async fn test_run_turn_accumulates_usage_and_cost_across_iterations() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![ContentBlock::tool_use(
                        "c1",
                        "current_time",
                        serde_json::json!({}),
                    )],
                    crate::types::StopReason::ToolUse,
                    Usage::new(100, 100),
                )))
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![ContentBlock::text("done")],
                    crate::types::StopReason::EndTurn,
                    Usage::new(50, 50),
                ))),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(
            outcome.usage,
            Usage::new(150, 150),
            "usage from both iterations should be summed, not just the last one"
        );
    }

    #[tokio::test]
    async fn test_run_turn_offers_only_tools_the_selection_and_tier_both_permit() {
        use async_trait::async_trait;
        use serde_json::{Value, json};

        struct StubTool;

        #[async_trait]
        impl crate::tools::Tool for StubTool {
            fn name(&self) -> &str {
                "web_search"
            }

            fn description(&self) -> &str {
                "search the web"
            }

            fn tier(&self) -> RiskTier {
                RiskTier::NetworkRead
            }

            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }

            async fn invoke(&self, _input: Value, _ctx: &ToolCtx) -> crate::tools::ToolOutcome {
                crate::tools::ToolOutcome::ok("unused")
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(StubTool));

        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_text("hi"));
        let harness = Harness::new(provider.clone(), Arc::new(registry));

        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::NetworkRead);
        turn_request.ctx = ctx(RiskTier::Safe); // authorized only for Safe, not NetworkRead

        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        assert!(
            sent.tools.is_empty(),
            "web_search should not be offered when the invoker is only authorized for Safe"
        );
    }

    /// Sleeps for `delay`, tracking how many instances are in flight at once
    /// via shared counters, then replies with `reply`.
    struct ConcurrencyTrackingTool {
        name: &'static str,
        serial: bool,
        delay: std::time::Duration,
        reply: &'static str,
        in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        max_observed: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for ConcurrencyTrackingTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "tracks how many instances run concurrently"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn is_serial(&self) -> bool {
            self.serial
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            use std::sync::atomic::Ordering;

            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_observed.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            crate::tools::ToolOutcome::ok(self.reply)
        }
    }

    #[tokio::test]
    async fn test_two_calls_to_a_serial_tool_never_overlap() {
        use std::sync::{Arc as StdArc, atomic::AtomicUsize};

        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_observed = StdArc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "shell",
            serial: true,
            delay: std::time::Duration::from_millis(20),
            reply: "ran",
            in_flight: in_flight.clone(),
            max_observed: max_observed.clone(),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "shell", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "shell", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["shell"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(
            max_observed.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "two calls to the same serial tool in one batch must never run at the same time"
        );
    }

    #[tokio::test]
    async fn test_independent_tools_run_concurrently() {
        use std::sync::{Arc as StdArc, atomic::AtomicUsize};

        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_observed = StdArc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        for name in ["first", "second"] {
            registry.register(Arc::new(ConcurrencyTrackingTool {
                name,
                serial: false,
                delay: std::time::Duration::from_millis(20),
                reply: "ran",
                in_flight: in_flight.clone(),
                max_observed: max_observed.clone(),
            }));
        }

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "first", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "second", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["first", "second"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(
            max_observed.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "two independent, non-serial tools should overlap rather than run one after the other"
        );
    }

    #[tokio::test]
    async fn test_result_order_matches_call_order_despite_differing_completion_times() {
        use std::sync::{Arc as StdArc, atomic::AtomicUsize};

        let in_flight = StdArc::new(AtomicUsize::new(0));
        let max_observed = StdArc::new(AtomicUsize::new(0));

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "slow",
            serial: false,
            delay: std::time::Duration::from_millis(30),
            reply: "slow-result",
            in_flight: in_flight.clone(),
            max_observed: max_observed.clone(),
        }));
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "fast",
            serial: false,
            delay: std::time::Duration::from_millis(1),
            reply: "fast-result",
            in_flight: in_flight.clone(),
            max_observed: max_observed.clone(),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    // slow is called first but finishes last; fast is called second but
                    // finishes first - the result order must still follow the call order
                    vec![
                        ContentBlock::tool_use("c1", "slow", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "fast", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["slow", "fast"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");

        let ContentBlock::ToolResult { content: first, .. } = &last_message.content[0] else {
            panic!("expected the first result to be a tool result block");
        };
        let ContentBlock::ToolResult {
            content: second, ..
        } = &last_message.content[1]
        else {
            panic!("expected the second result to be a tool result block");
        };

        assert_eq!(
            first, "slow-result",
            "the first call's result must appear first in output order"
        );
        assert_eq!(
            second, "fast-result",
            "even though it finished before the slow call did"
        );
    }

    #[tokio::test]
    async fn test_invalid_arguments_never_reach_invoke_and_are_recoverable() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                // missing the required "query" field
                .respond_tool_use("c1", "search", serde_json::json!({}))
                .respond_text("okay, trying differently"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("invalid arguments should be recoverable, not fatal");

        assert_eq!(outcome.text.as_deref(), Some("okay, trying differently"));

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        let ContentBlock::ToolResult {
            content, is_error, ..
        } = &last_message.content[0]
        else {
            panic!("expected a tool result block");
        };
        assert!(
            *is_error,
            "invalid arguments should be reported as an error result"
        );
        assert!(
            content.contains("invalid arguments"),
            "the model should see why validation failed: {content:?}"
        );
    }

    #[tokio::test]
    async fn test_valid_arguments_reach_invoke() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "search", serde_json::json!({"query": "cats"}))
                .respond_text("found some cats"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert_eq!(outcome.text.as_deref(), Some("found some cats"));
    }

    #[tokio::test]
    async fn test_repeated_invalid_arguments_eventually_abort_the_turn() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SchemaedTool));

        // one more invalid call than the default budget's max_tool_retries (3) allows
        let mut provider_builder = MockProvider::new();
        for _ in 0..4 {
            provider_builder =
                provider_builder.respond_tool_use("c1", "search", serde_json::json!({}));
        }
        let provider: Arc<MockProvider> = Arc::new(provider_builder);

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["search"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let result = harness.run_turn(turn_request).await;

        assert!(
            result.is_err(),
            "a model that never produces valid arguments must not loop forever"
        );
    }

    #[tokio::test]
    async fn test_run_turn_truncates_gracefully_when_iteration_budget_is_reached() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "loop_tool",
            reply: "again",
        }));

        // every response asks for another tool call, forever - only the budget stops
        // this
        let mut builder = MockProvider::new();
        for _ in 0..5 {
            builder = builder.respond_tool_use("c1", "loop_tool", serde_json::json!({}));
        }
        let provider: Arc<MockProvider> = Arc::new(builder);

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["loop_tool"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.budget = Budget {
            max_iterations: Some(2),
            ..Budget::default()
        };

        let harness = Harness::new(provider.clone(), Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("an exhausted budget should truncate gracefully, not error");

        assert_eq!(
            outcome.iterations, 2,
            "should stop exactly at the configured limit"
        );
        assert_eq!(
            provider.request_count(),
            2,
            "must not make a third round trip once the iteration budget is spent"
        );
        let text = outcome.text.unwrap();
        assert!(
            text.contains("budget"),
            "the truncation reason should be visible in the returned text, got {text:?}"
        );
    }

    #[tokio::test]
    async fn test_a_dispatched_tool_call_sees_the_live_remaining_budget() {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(CtxCapturingTool {
            captured: captured.clone(),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![ContentBlock::tool_use(
                        "c1",
                        "capture",
                        serde_json::json!({}),
                    )],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["capture"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.budget = Budget {
            max_iterations: Some(5),
            ..Budget::default()
        };

        let harness = Harness::new(provider, Arc::new(registry));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0].max_iterations,
            Some(4),
            "the first iteration's own usage is recorded before its tool calls dispatch, so \
             remaining_budget should already reflect it rather than the full starting budget"
        );
    }

    #[tokio::test]
    async fn test_run_turn_truncates_gracefully_when_cost_budget_is_exceeded() {
        // claude-opus-5 is priced at $15/mtok input and $75/mtok output in
        // pricing.toml, so one million of each costs $90 - comfortably over a
        // one dollar budget in a single iteration
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![
                    ContentBlock::text("partial answer"),
                    ContentBlock::tool_use("c1", "current_time", serde_json::json!({})),
                ],
                crate::types::StopReason::ToolUse,
                Usage::new(1_000_000, 1_000_000),
            ),
        )));

        let mut turn_request = request();
        turn_request.budget = Budget {
            max_cost: Some(crate::types::Cost::from_dollars(1.0)),
            ..Budget::default()
        };

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("an exhausted cost budget should truncate gracefully, not error");

        assert_eq!(
            provider.request_count(),
            1,
            "must not make a second round trip once the cost budget is already spent"
        );
        let text = outcome.text.expect("partial text should be preserved");
        assert!(
            text.contains("partial answer"),
            "the assistant's own text should survive: {text:?}"
        );
        assert!(
            text.contains("truncated"),
            "the truncation should be visible: {text:?}"
        );
    }

    #[tokio::test]
    async fn test_run_turn_truncation_marker_names_the_limit() {
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_tool_use("c2", "current_time", serde_json::json!({})),
        );
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["current_time"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.budget = Budget {
            max_iterations: Some(1),
            ..Budget::default()
        };

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should truncate gracefully");

        let text = outcome.text.expect("should have a truncation marker");
        assert!(
            text.contains("iterations"),
            "the marker should name which limit was hit, got {text:?}"
        );
    }

    #[tokio::test]
    async fn test_run_turn_fails_fast_when_already_cancelled() {
        let provider = Arc::new(MockProvider::new());
        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));

        let turn_request = request();
        turn_request.ctx.cancellation.cancel();

        let result = harness.run_turn(turn_request).await;

        assert!(
            matches!(result, Err(AiError::Cancelled)),
            "an already-cancelled context must fail immediately"
        );
        assert_eq!(
            provider.request_count(),
            0,
            "a pre-cancelled turn should never even reach the provider"
        );
    }

    /// A provider that sleeps for a fixed delay before responding, so a test
    /// can cancel it mid-flight and confirm the wait is actually
    /// interrupted rather than merely ignored.
    struct SlowProvider {
        delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for SlowProvider {
        fn name(&self) -> &str {
            "slow"
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<crate::types::CompletionResponse, AiError> {
            tokio::time::sleep(self.delay).await;
            Ok(crate::types::CompletionResponse::new(
                vec![ContentBlock::text("too slow")],
                crate::types::StopReason::EndTurn,
                Usage::default(),
            ))
        }
    }

    #[tokio::test]
    async fn test_run_turn_cancels_promptly_mid_provider_call() {
        let provider = Arc::new(SlowProvider {
            delay: std::time::Duration::from_secs(3600),
        });
        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));

        let turn_request = request();
        let cancellation = turn_request.ctx.cancellation.clone();

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            cancellation.cancel();
        });

        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            harness.run_turn(turn_request),
        )
        .await
        .expect("the turn must return well before the provider's own hour-long delay");

        assert!(matches!(result, Err(AiError::Cancelled)));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "cancellation should interrupt the wait almost immediately, not after the full delay"
        );
    }

    /// A tool that sleeps for a fixed delay, for the same mid-flight
    /// cancellation proof as `SlowProvider`, but on the tool-dispatch path
    /// instead of the provider call.
    struct SlowTool {
        delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl crate::tools::Tool for SlowTool {
        fn name(&self) -> &str {
            "slow_tool"
        }

        fn description(&self) -> &str {
            "sleeps for a long time"
        }

        fn tier(&self) -> RiskTier {
            RiskTier::Safe
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        async fn invoke(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> crate::tools::ToolOutcome {
            tokio::time::sleep(self.delay).await;
            crate::tools::ToolOutcome::ok("too slow")
        }
    }

    #[tokio::test]
    async fn test_dispatch_cancels_a_running_tool_promptly() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SlowTool {
            delay: std::time::Duration::from_secs(3600),
        }));

        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_tool_use(
            "c1",
            "slow_tool",
            serde_json::json!({}),
        ));

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["slow_tool"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        let cancellation = turn_request.ctx.cancellation.clone();

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            cancellation.cancel();
        });

        let harness = Harness::new(provider, Arc::new(registry));

        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            harness.run_turn(turn_request),
        )
        .await
        .expect("the turn must return well before the tool's own hour-long delay");

        assert!(matches!(result, Err(AiError::Cancelled)));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "cancelling mid-tool-call should interrupt it almost immediately"
        );
    }

    fn handoff_schema() -> HandoffSchema {
        HandoffSchema::new(
            "call this to submit your decision",
            serde_json::json!({
                "type": "object",
                "properties": {"action": {"type": "string"}},
                "required": ["action"]
            }),
        )
    }

    #[tokio::test]
    async fn test_valid_handoff_ends_the_turn_with_its_payload() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_tool_use(
            "c1",
            "handoff",
            serde_json::json!({"action": "ApprovePlan"}),
        ));

        let mut turn_request = request();
        turn_request.handoff = Some(handoff_schema());

        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert!(outcome.is_handoff());
        assert_eq!(outcome.handoff.unwrap()["action"], "ApprovePlan");
    }

    #[tokio::test]
    async fn test_handoff_tool_is_offered_to_the_provider() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_tool_use(
            "c1",
            "handoff",
            serde_json::json!({"action": "ApprovePlan"}),
        ));

        let mut turn_request = request();
        turn_request.handoff = Some(handoff_schema());

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let sent = &provider.requests()[0];
        assert!(
            sent.tools.iter().any(|schema| schema.name == "handoff"),
            "the handoff tool must be offered even though no Tool impl registers it"
        );
    }

    #[tokio::test]
    async fn test_invalid_handoff_payload_is_recoverable_not_fatal() {
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                // missing the required "action" field
                .respond_tool_use("c1", "handoff", serde_json::json!({}))
                .respond_tool_use("c2", "handoff", serde_json::json!({"action": "ApprovePlan"})),
        );

        let mut turn_request = request();
        turn_request.handoff = Some(handoff_schema());

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("an invalid handoff payload should be recoverable, not fatal");

        assert!(
            outcome.is_handoff(),
            "the retried, valid handoff should still succeed"
        );

        let second_request = &provider.requests()[1];
        let last_message = second_request
            .history
            .iter()
            .last()
            .expect("history should not be empty");
        let ContentBlock::ToolResult {
            content, is_error, ..
        } = &last_message.content[0]
        else {
            panic!("expected a tool result block");
        };
        assert!(
            *is_error,
            "an invalid handoff payload should be an error result"
        );
        assert!(content.contains("invalid handoff"), "got {content:?}");
    }

    #[tokio::test]
    async fn test_plain_text_answer_is_rejected_when_handoff_is_required() {
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_text("here's my answer") // wrong: must call handoff instead
                .respond_tool_use("c1", "handoff", serde_json::json!({"action": "ApprovePlan"})),
        );

        let mut turn_request = request();
        turn_request.handoff = Some(handoff_schema());

        let harness = Harness::new(provider.clone(), Arc::new(ToolRegistry::new()));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should retry rather than accept plain text");

        assert!(
            outcome.is_handoff(),
            "the eventual valid handoff should be what the turn returns"
        );
        assert_eq!(
            provider.request_count(),
            2,
            "the model should have been nudged into a second attempt"
        );
    }

    #[tokio::test]
    async fn test_repeated_invalid_handoffs_eventually_abort_the_turn() {
        let mut builder = MockProvider::new();
        // one more than the default budget's max_tool_retries (3) allows
        for _ in 0..4 {
            builder = builder.respond_tool_use("c1", "handoff", serde_json::json!({}));
        }
        let provider: Arc<MockProvider> = Arc::new(builder);

        let mut turn_request = request();
        turn_request.handoff = Some(handoff_schema());

        let harness = Harness::new(provider, Arc::new(ToolRegistry::new()));
        let result = harness.run_turn(turn_request).await;

        assert!(
            result.is_err(),
            "a model that never produces a valid handoff must not loop forever"
        );
    }

    #[tokio::test]
    async fn test_handoff_ends_the_turn_even_with_other_calls_in_the_same_batch() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond(Ok(
            crate::types::CompletionResponse::new(
                vec![
                    ContentBlock::tool_use("c1", "current_time", serde_json::json!({})),
                    ContentBlock::tool_use(
                        "c2",
                        "handoff",
                        serde_json::json!({"action": "ApprovePlan"}),
                    ),
                ],
                crate::types::StopReason::ToolUse,
                Usage::default(),
            ),
        )));

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["current_time"]);
        turn_request.ctx = ctx(RiskTier::Safe);
        turn_request.handoff = Some(handoff_schema());

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        assert!(
            outcome.is_handoff(),
            "a valid handoff call should end the turn even alongside other tool calls"
        );
    }

    /// Drains a streamed turn into a plain `Vec`, for tests that only care
    /// about the final sequence of events rather than genuine incremental
    /// arrival.
    async fn collect_streamed(harness: Arc<Harness>, request: TurnRequest) -> Vec<HarnessEvent> {
        harness.run_turn_streamed(request).collect().await
    }

    #[tokio::test]
    async fn test_streamed_turn_starts_with_turn_started() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_text("hi"));
        let harness = Arc::new(Harness::new(provider, Arc::new(ToolRegistry::new())));

        let events = collect_streamed(harness, request()).await;

        assert!(
            matches!(events.first(), Some(HarnessEvent::TurnStarted { .. })),
            "the first event should always be TurnStarted, got {:?}",
            events.first()
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_yields_text_delta_then_finishes() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_text("hello there"));
        let harness = Arc::new(Harness::new(provider, Arc::new(ToolRegistry::new())));

        let events = collect_streamed(harness, request()).await;

        let text_deltas: String = events
            .iter()
            .filter_map(|event| match event {
                HarnessEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_deltas, "hello there");

        assert!(
            matches!(events.last(), Some(HarnessEvent::TurnFinished { .. })),
            "the last event should be TurnFinished, got {:?}",
            events.last()
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_reports_iteration_completion() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_text("hi"));
        let harness = Arc::new(Harness::new(provider, Arc::new(ToolRegistry::new())));

        let events = collect_streamed(harness, request()).await;

        assert!(
            events
                .iter()
                .any(|event| matches!(event, HarnessEvent::IterationComplete { iteration: 1, .. })),
            "a single round trip should report iteration 1 complete, got {events:?}"
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_dispatches_a_tool_and_finishes_with_text() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Arc::new(Harness::new(provider, Arc::new(registry)));
        let events = collect_streamed(harness, turn_request).await;

        let text_deltas: String = events
            .iter()
            .filter_map(|event| match event {
                HarnessEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_deltas, "it is 12:00");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, HarnessEvent::IterationComplete { .. }))
                .count(),
            2,
            "one iteration for the tool call, one for the final answer"
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_emits_tool_started_and_finished_around_the_call() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        // one leading text delta and one trailing text delta, either side of the
        // tool call, so ordering can be checked against both rather than just
        // "somewhere in the stream"
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::text("let me check..."),
                        ContentBlock::tool_use("c1", "current_time", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Arc::new(Harness::new(provider, Arc::new(registry)));
        let events = collect_streamed(harness, turn_request).await;

        let text_before = events
            .iter()
            .position(
                |event| matches!(event, HarnessEvent::TextDelta(text) if text == "let me check..."),
            )
            .expect("expected the leading text delta");
        let started = events
            .iter()
            .position(|event| matches!(event, HarnessEvent::ToolStarted { name } if name == "current_time"))
            .expect("expected a ToolStarted event");
        let finished = events
            .iter()
            .position(|event| matches!(event, HarnessEvent::ToolFinished { name, .. } if name == "current_time"))
            .expect("expected a ToolFinished event");
        let text_after = events
            .iter()
            .position(
                |event| matches!(event, HarnessEvent::TextDelta(text) if text == "it is 12:00"),
            )
            .expect("expected the trailing text delta");

        assert!(
            text_before < started,
            "ToolStarted should come after the text that led up to the call"
        );
        assert!(
            started < finished,
            "ToolStarted must be yielded before ToolFinished for the same call"
        );
        assert!(
            finished < text_after,
            "ToolFinished should come before the next iteration's reply"
        );

        match &events[finished] {
            HarnessEvent::ToolFinished { ok, .. } => assert!(ok, "the call succeeded"),
            other => panic!("expected ToolFinished, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_streamed_turn_signals_parallel_tool_starts_promptly_not_batched() {
        // both tools take the same, deliberately long time to finish - if starts
        // were only ever forwarded once the whole batch was done (the bug this
        // is guarding against), both ToolStarted events would only appear
        // alongside the ToolFinished ones, roughly `delay` after the turn began
        let delay = std::time::Duration::from_millis(200);
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(SlowTool { delay }));
        registry.register(Arc::new(ConcurrencyTrackingTool {
            name: "other_slow_tool",
            serial: false,
            delay,
            reply: "done",
            in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_observed: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "slow_tool", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "other_slow_tool", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("done"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["slow_tool", "other_slow_tool"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Arc::new(Harness::new(provider, Arc::new(registry)));
        let mut stream = harness.run_turn_streamed(turn_request);
        let start = std::time::Instant::now();

        let mut started_count = 0usize;
        let mut both_started_after = None;
        let mut finished_after = None;
        while let Some(event) = stream.next().await {
            match event {
                HarnessEvent::ToolStarted { .. } => {
                    started_count += 1;
                    if started_count == 2 {
                        both_started_after = Some(start.elapsed());
                    }
                }
                HarnessEvent::ToolFinished { .. } if finished_after.is_none() => {
                    finished_after = Some(start.elapsed());
                }
                _ => {}
            }
        }

        let both_started_after =
            both_started_after.expect("expected two ToolStarted events, one per parallel call");
        let finished_after = finished_after.expect("expected at least one ToolFinished event");

        assert!(
            both_started_after < delay / 2,
            "both calls should announce themselves promptly, well under the {delay:?} it takes \
             either to finish - saw the second ToolStarted at {both_started_after:?}"
        );
        assert!(
            finished_after >= delay,
            "sanity check that the calls really did take the scripted delay to finish, saw the \
             first ToolFinished at {finished_after:?}"
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_yields_handoff_event() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new().respond_tool_use(
            "c1",
            "handoff",
            serde_json::json!({"action": "ApprovePlan"}),
        ));

        let mut turn_request = request();
        turn_request.handoff = Some(handoff_schema());

        let harness = Arc::new(Harness::new(provider, Arc::new(ToolRegistry::new())));
        let events = collect_streamed(harness, turn_request).await;

        let handoff_payload = events.iter().find_map(|event| match event {
            HarnessEvent::Handoff(payload) => Some(payload),
            _ => None,
        });
        assert_eq!(
            handoff_payload.expect("should have yielded a Handoff event")["action"],
            "ApprovePlan"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, HarnessEvent::TurnFinished { .. })),
            "a handoff ends the turn instead of a normal TurnFinished"
        );
    }

    /// Always succeeds with a fixed reply - never a real turn, no provider,
    /// no network.
    struct FakeDelegator;

    #[async_trait::async_trait]
    impl crate::tools::Delegator for FakeDelegator {
        async fn delegate(
            &self,
            _persona: &crate::persona::PersonaId,
            _task: String,
            _ctx: &ToolCtx,
        ) -> Result<String, AiError> {
            Ok("the specialist's answer".to_string())
        }
    }

    #[tokio::test]
    async fn test_a_delegate_call_emits_delegation_events_not_generic_tool_events() {
        use crate::tools::{DelegablePersona, DelegateTool, DelegatorCell};

        let delegator = std::sync::Arc::new(FakeDelegator);
        let cell = std::sync::Arc::new(DelegatorCell::new());
        cell.set(
            std::sync::Arc::downgrade(&delegator) as std::sync::Weak<dyn crate::tools::Delegator>
        );

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DelegateTool::new(
            cell,
            vec![DelegablePersona {
                id: crate::persona::PersonaId::new("researcher"),
                description: "research".to_string(),
            }],
            2,
        )));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![ContentBlock::tool_use(
                        "c1",
                        "delegate",
                        serde_json::json!({"persona": "researcher", "task": "look into tea"}),
                    )],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("the researcher found some things"),
        );

        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["delegate"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Arc::new(Harness::new(provider, Arc::new(registry)));
        let events = collect_streamed(harness, turn_request).await;

        let started = events.iter().find_map(|event| match event {
            HarnessEvent::DelegationStarted { persona, task } => {
                Some((persona.clone(), task.clone()))
            }
            _ => None,
        });
        assert_eq!(
            started,
            Some(("researcher".to_string(), "look into tea".to_string())),
            "should have yielded a DelegationStarted event carrying the persona and task"
        );

        let finished = events.iter().find_map(|event| match event {
            HarnessEvent::DelegationFinished { persona, ok } => Some((persona.clone(), *ok)),
            _ => None,
        });
        assert_eq!(
            finished,
            Some(("researcher".to_string(), true)),
            "should have yielded a DelegationFinished event"
        );

        assert!(
            !events.iter().any(|event| matches!(
                event,
                HarnessEvent::ToolStarted { .. } | HarnessEvent::ToolFinished { .. }
            )),
            "a delegate call should never also emit the generic tool events"
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_yields_failed_on_provider_error() {
        let provider: Arc<MockProvider> =
            Arc::new(MockProvider::new().respond_error(AiError::Rejected("bad key".to_string())));
        let harness = Arc::new(Harness::new(provider, Arc::new(ToolRegistry::new())));

        let events = collect_streamed(harness, request()).await;

        assert!(
            matches!(events.last(), Some(HarnessEvent::Failed(_))),
            "a provider error should surface as a Failed event, got {:?}",
            events.last()
        );
    }

    #[tokio::test]
    async fn test_streamed_turn_fails_fast_when_already_cancelled() {
        let provider: Arc<MockProvider> = Arc::new(MockProvider::new());
        let harness = Arc::new(Harness::new(
            provider.clone(),
            Arc::new(ToolRegistry::new()),
        ));

        let turn_request = request();
        turn_request.ctx.cancellation.cancel();

        let events = collect_streamed(harness, turn_request).await;

        assert!(matches!(
            events.last(),
            Some(HarnessEvent::Failed(AiError::Cancelled))
        ));
        assert_eq!(
            provider.request_count(),
            0,
            "a pre-cancelled turn should never reach the provider"
        );
    }

    /// A [`ToolAuditor`] that keeps everything it was asked to record.
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

    #[tokio::test]
    async fn test_a_successful_tool_call_is_audited() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let auditor = Arc::new(FakeToolAuditor::default());
        let harness = Harness::new(provider, Arc::new(registry)).with_auditor(auditor.clone());
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let records = auditor.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tool_name, "current_time");
        assert_eq!(records[0].status, crate::audit::ToolCallStatus::Ok);
        assert_eq!(records[0].output, "12:00");
    }

    #[tokio::test]
    async fn test_an_unknown_tool_call_is_not_audited_since_it_never_dispatches() {
        // an unknown tool name (and, identically, invalid arguments or an invalid
        // handoff payload) is rejected before dispatch_calls ever runs - there is
        // no real invocation to time or record, just a validation failure the
        // model already sees in its own tool result. Auditing only what actually
        // reached a Tool::invoke keeps ai_tool_calls a record of real calls, not
        // a duplicate of what the transcript already shows.
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "does_not_exist", serde_json::json!({}))
                .respond_text("okay, moving on"),
        );

        let auditor = Arc::new(FakeToolAuditor::default());
        let harness =
            Harness::new(provider, Arc::new(ToolRegistry::new())).with_auditor(auditor.clone());
        harness.run_turn(request()).await.expect("should succeed");

        assert!(auditor.records().is_empty());
    }

    #[tokio::test]
    async fn test_a_fatal_tool_outcome_is_still_audited() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(FatalTool));

        let provider: Arc<MockProvider> =
            Arc::new(MockProvider::new().respond_tool_use("c1", "explode", serde_json::json!({})));
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let auditor = Arc::new(FakeToolAuditor::default());
        let harness = Harness::new(provider, Arc::new(registry)).with_auditor(auditor.clone());
        let result = harness.run_turn(turn_request).await;

        assert!(result.is_err(), "the turn should still abort");
        let records = auditor.records();
        assert_eq!(
            records.len(),
            1,
            "the call should still be audited despite aborting the turn"
        );
        assert_eq!(records[0].status, crate::audit::ToolCallStatus::Fatal);
    }

    #[tokio::test]
    async fn test_every_call_in_a_multi_tool_response_is_audited() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "first",
            reply: "one",
        }));
        registry.register(Arc::new(EchoTool {
            name: "second",
            reply: "two",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond(Ok(crate::types::CompletionResponse::new(
                    vec![
                        ContentBlock::tool_use("c1", "first", serde_json::json!({})),
                        ContentBlock::tool_use("c2", "second", serde_json::json!({})),
                    ],
                    crate::types::StopReason::ToolUse,
                    Usage::default(),
                )))
                .respond_text("both done"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::named(["first", "second"]);
        turn_request.ctx = ctx(RiskTier::Safe);

        let auditor = Arc::new(FakeToolAuditor::default());
        let harness = Harness::new(provider, Arc::new(registry)).with_auditor(auditor.clone());
        harness
            .run_turn(turn_request)
            .await
            .expect("should succeed");

        let mut names: Vec<String> = auditor.records().into_iter().map(|r| r.tool_name).collect();
        names.sort();
        assert_eq!(names, vec!["first".to_string(), "second".to_string()]);
    }

    #[tokio::test]
    async fn test_streamed_turns_audit_tool_calls_too() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));

        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let auditor = Arc::new(FakeToolAuditor::default());
        let harness =
            Arc::new(Harness::new(provider, Arc::new(registry)).with_auditor(auditor.clone()));
        collect_streamed(harness, turn_request).await;

        assert_eq!(
            auditor.records().len(),
            1,
            "the streamed path shares handle_tool_calls with run_turn, so auditing must work \
             identically there"
        );
    }

    #[tokio::test]
    async fn test_without_an_auditor_wired_nothing_breaks() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool {
            name: "current_time",
            reply: "12:00",
        }));
        let provider: Arc<MockProvider> = Arc::new(
            MockProvider::new()
                .respond_tool_use("c1", "current_time", serde_json::json!({}))
                .respond_text("it is 12:00"),
        );
        let mut turn_request = request();
        turn_request.tools = ToolSelection::tier(RiskTier::Safe);
        turn_request.ctx = ctx(RiskTier::Safe);

        let harness = Harness::new(provider, Arc::new(registry));
        let outcome = harness
            .run_turn(turn_request)
            .await
            .expect("should succeed without an auditor wired");
        assert_eq!(outcome.text.as_deref(), Some("it is 12:00"));
    }
}
