# Porting a municode role prompt

`municode/docs/agent-prompts/*.md` (and the Issue Analyst sketch at `municode/docs/plan.md:889-929`)
are genuinely good prompts and are the roster milestone 3 phase 16 ports into munibot as delegable
personas. They were written for a pipeline, though, so each one mixes two things that need to be
pulled apart:

- **Role-and-standards prose** - what the role is, what good work in it looks like, what to watch
  for. Context-independent, valuable regardless of who's calling this persona or how.
- **Output-contract prose** - "return JSON shaped like...", "the builder will then...", literal
  field tables for a pipeline's own machine format.

Only the former belongs in the ported `.md` file. The latter becomes a `HandoffSchema` a future
pipeline (milestone 5) attaches to the persona in code - never text in the prompt itself. This is
what lets one prompt file serve both chat delegation (`handoff: None`, the persona just answers in
its own words) and the eventual pipeline (`handoff: Some(schema)`, structured output) with zero
duplication and no "if chat mode, do X, if pipeline mode, do Y" branching inside the prose.

## What to strip

- Any `<xml-tag>{{context}}</xml-tag>`-shaped context injection block (`<user-instructions>`,
  `<codebase-summary>`, `<reviewer-feedback>`, and municode's other pipeline-specific inputs). Chat
  delegation has no equivalent multi-field context injection - the companion's task brief arrives as
  a single, self-contained user message (see `Ai::delegate`), not a template variable.
- Every `## Output: <ActionName>` section, its JSON example, and its field-requirement table.
- Any prose that assumes a specific downstream consumer exists ("the builder agent will rely on
  this entirely", "a previous version of your plan was rejected by the Architecture Reviewer") -
  reword to describe the judgement call itself, not the pipeline stage that will act on it.

## What to keep

- The role description, adapted to munibot's own house style (`{{user_name}}`/`{{platform}}` framing,
  same as every other persona - these personas are `delegable = true`, not exclusively reachable
  through delegation, so a person can still open a direct conversation with one).
- Every genuinely reusable planning/review/judgement principle (municode's "Atomicity",
  "Completeness", "Ordering", "Consistency" sections, its "writing good instructions" guidance, and
  so on) - these are exactly the "role, standards, and judgement" the porting rule means to keep.
- An `## Instruction hierarchy` section, matching every other munibot persona.

## Precedent

`software-architect.md` (commit establishing this pattern) is the worked example - compare it
against `municode/docs/agent-prompts/software-architect.md` directly to see the transformation
applied concretely. Every prompt ported after it follows the same rule.

Tests for a ported prompt should include, alongside the usual `{{variable}}` and
render-with-a-sample-context checks, an explicit assertion that none of the source prompt's own
output-contract vocabulary survived the port (see
`persona::prompt_tests::test_software_architect_prompt_has_no_pipeline_output_contract` for the
pattern) - a real regression check that a future edit to the prompt doesn't reintroduce pipeline
prose by accident.
