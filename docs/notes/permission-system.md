# The permission system

Added alongside milestone 2's usage/spend panel (`get_global_usage` needed a real notion of "who is
allowed to see this", and munibot had none at all before this).

## Shape

- `munibot_core::Permission` — a plain enum, one variant per capability (`Operator` is the only one so
  far). `Display`/`FromStr` (via `strum`, `serialize_all = "snake_case"`) are the single source of
  truth for each variant's canonical string token; `Serialize`/`Deserialize` are a thin bridge onto
  those, not a second mapping.
- `user_permissions` table — one row per `(user_id, permission)` pair, not a bitmask column. Matches
  `axum_session_auth`'s own documented convention (`SELECT token FROM user_permissions WHERE user_id =
? AND token = ?`) directly.
- `munibot_api::auth::server::User.permissions: HashSet<String>` — loaded once in
  `Authentication::load_user`, checked entirely in memory for the rest of the session via
  `HasPermission::has`. Never a live query per check.
- `Config.operators: Vec<OperatorConfig>` — a top-level `[[operators]]` section (not under `[ai]`:
  operator is a service-wide role, even though the usage panel is its first real use). Each entry is
  either `{ provider, provider_user_id }` (a linked Discord/Twitch account) or `{ munibot_user_id }` (a
  raw internal id).
- `munibot::permissions::sync_operators` — resolves every configured entry to a user and grants
  `Permission::Operator`, once, at startup.

## The deliberate gap: grant-only, no revocation

Removing an entry from `[[operators]]` does **not** revoke a permission already granted. Nothing in
this system ever calls a "revoke" operation at all — there isn't one yet.

This was a conscious scope decision, not an oversight: the ask was specifically "when munibot starts
up, matching users... receive an operator permission", and building a correct revocation sync (which
has to answer "does this user's _current_ set of linked accounts still match _any_ configured entry",
not just "is this one entry still configured") is real design work that didn't have a concrete need
driving it yet.

If you need revocation:

- Add `munibot_core::db::operations::revoke_permission(pool, user_id, permission)` (mirrors
  `grant_permission`, a plain `DELETE`).
- `sync_operators` would need to additionally list every user who currently holds `Operator` (a new
  `list_users_with_permission` query) and revoke it from anyone whose linked accounts no longer match
  any configured entry - not just "wasn't in this run's list", since a user can have multiple linked
  accounts and only one needs to still match.
- Decide what "no longer matches" means for the `MunibotUser { munibot_user_id }` config variant
  specifically, since that one has no linked-account identity to fall out of sync with at all.

## Adding a new permission

Add a variant to `Permission` in `munibot_core/src/permission.rs`. Nothing else needs to change to
support checking it (`HasPermission::has` already checks against whatever string form it now has) -
you only need a new `require_*` helper (mirroring `auth::operator::require_operator`) at whichever
server function actually wants to gate on it, and a way to grant it (today, only
`sync_operators`/`[[operators]]` grants anything, and only ever `Operator`).
