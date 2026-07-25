# Implementation notes: gui configuration provider system

Deviations and clarifications discovered while implementing
`docs/plans/gui-configuration.md`, recorded here so later commits (and
other agents) don't have to rediscover them. The plan doc itself is not
edited in place; it stays the historical high-level design.

## `FieldKind::Float` added (phase 1, commit 5)

The plan's `ConfigValue` (section 5.2) already had a `Float(f64)` variant,
but `FieldKind` (section 5.3) had no matching variant, which would have
made `Float` unreachable from any real schema. Added `FieldKind::Float {
min, max, step, unit }`, mirroring `Int`. No v1 provider uses it yet, but
`ConfigField for f64` needs a real `FieldKind` to report.
