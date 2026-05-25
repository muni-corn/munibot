# Changelog

## **v0.3.1**

---

## **v0.3.0**

### **Breaking changes**

- **discord:** rename MuniBotError to MunibotDiscordError
- **db:** remove surrealdb `DbItem` trait and dependencies
- replace all surrealdb calls with diesel calls

### new features

- **discord:** add fox command with randomfox.ca integration
- **discord:** add spans and structured fields to autodelete background loop
- **discord:** instrument moderation audit logging handler
- **twitch:** add spans for message dispatch and Helix API calls
- **core:** instrument config loading and database setup
- **munibot:** instrument startup and spawn discord with a root span
- **discord:** add spans for event dispatch and command handlers
- **db:** add get_quote_by_content query function
- add `Passing` trait for non-panicking error handling
- **nixos:** add conditional database url to munibot service environment
- **nixos:** update munibot service module with mysql support
- **migration:** add comprehensive data verification after migration
- **db:** add migration module for migrating surrealdb data to mysql
- **db:** add all database operations in new `operations` module
- **db:** add models module for structs from database tables
- **db:** add initial tables and migration scripts
- **db:** add `DbPool` type and `establish_pool` utility for diesel

### bug fixes

- **migration:** cast string-typed fields in SurrealQL queries
- **discord:** remove ws protocol prefix from default surreal url
- **db:** separate migration execution from pool initialization
- **db:** add IF NOT EXISTS clause to all table creation statements
- **handlers:** correct typo in discord command message
- **db:** add embedded database migrations on pool initialization
- **nixos:** remove hardcoded mysql port from database url
- **nixos:** convert mysql port to string in database url
- **migration:** handle skipped records and weak idempotency check
- **migration:** make surrealdb connection generic
- **flake:** use cargo and clippy from `rust-project` toolchain
- remove stable features

### performance

- wrap large error variants in `Box`

### tests

- **munibot_core:** move integration tests to core crate
- **munibot_twitch:** move tests to end of content_warning module
- **operations:** add integration tests for guild config operations
- **content_warning:** add unit tests for ContentWarningHandler initial state
- **passing:** add unit tests for the Passing trait
- **config:** add unit tests for config loading and default values
- **eight_ball:** add unit tests for eight ball response generation
- **autodelete:** add unit tests for AutoDeleteMode serialization round-trip
- **logging:** add unit tests for logging bool formatting helpers
- **autoban:** expand scam message detection test coverage
- **bot_affection:** add unit tests for affection response selection
- **dice:** add unit tests for dice roll logic
- **magical:** add unit tests for magical percentage handler
- **economy:** add unit tests for salary calculation
- **greeting:** add unit tests for greeting handler
- **temperature:** add unit tests for temperature conversion functions
- **migration:** add test against real SurrealDB export data
- **migration:** isolate tests with per-test databases
- **migration:** strengthen assertions and add edge case coverage
- **migration:** add stream_category and stream_title assertions
- **migration:** switch to in-memory surrealdb for painless testing
- add db migration test

### build system

- replace log and env_logger with tracing dependencies
- **deps:** upgrade rand to 0.10.1 and update RNG APIs
- **munibot_core:** add tempfile dev dependency
- **nix:** update devenv and flake for workspace
- **munibot_twitch:** create twitch crate skeleton
- **munibot_discord:** create discord crate skeleton
- **munibot_core:** create core crate skeleton

### documentation

- document tracing usage and RUST_LOG conventions
- **plans:** update modular refactor plan to use root-level crates
- update copyright year and bot description
- add modular refactor plan for workspace crates
- remove surrealdb setup from readme
- **migration:** normalize verification checklist formatting
- reflow readme paragraphs to improve readability
- add comprehensive agentic development guide with style guides

---

## **v0.2.4**

### new features

- **autodelete:** use LoggingHandler's ignore feature for less annoying logs
- **logging:** allow message ids to be ignored when logging
- add treefmt-nix for unified code formatting
- **logging:** fix some formatting for guild updates
- rename to munibot, underscore removed
- **admin:** make admin responses ephemeral
- **logging:** implement logging for guild updates
- **logging:** don't log `position` changes in role updates
- update LICENSE and README
- **simple:** make tone indicator guide ephemeral
- **bot_affection:** remove more affectionate commands
- create FUNDING.yml

### bug fixes

- use safe UTF-8 boundary truncation for Discord embed fields
- duplicate import
- **vc_greeter:** push nickname safely to prevent everyone pings
- **logging:** fix a few issues with guild updates
- **logging:** fix spacing in channel delete handler
- **autodelete:** fix text spacing in message
- **logging:** fix channel delete message typo

### performance

- box thicc `MuniBotError`s

### build system

- update crate features
- pin libressl to libressl_4_0
- update Rust edition from 2021 to 2024
- correct stdenv adapter function call syntax in flake.nix

---

## **v0.2.3**

### new features

- **discord:** add tone indicator guide command

---

## **v0.2.2**

### new features

- **autodelete:** don't delete pinned messages

---

## **v0.2.1**

### performance

- **autodelete:** replace futures block_on with tokio's

---

## **v0.2.0**

### **Breaking changes**

- feat(bot_affection): fallback to a default string if empty response

### new features

- add autodelete feature
- **logging:** add pausing and arbitrary logs
- **admin:** make admin commands ephemeral
- introduce GlobalAccess type
- **discord:** add LoggingHandler in DiscordState
- **discord:** improve some error messages
- **admin:** allow setting logging channel to current by ommission
- add basic vc greeter functionality
- allow for missing log channels without errors
- skip launching twitch integration if misconfigured
- **logging:** add display name to member joining message
- upgrade surrealdb to `2.0`
- **logging:** handle role updates with more detail
- remove `TopicChangeProvider`
- **logging:** improve logging for reaction emoji removals
- **logging:** add message link for single reaction removals
- **logging:** add better logging for member updates
- **logging:** add message links to reaction removal events
- **logging:** improve new member logging with account created timestamp
- **discord:** register with `GUILD_MEMBERS` intent
- **autoban:** use homoglyph detection on messages
- **logging:** improve message delete message
- **logging:** better formatting for `max_uses`
- **logging:** make useless update messages less of an eyesore
- **economy:** swap checks for zero and self-transfer
- **logging:** add more metadata to message update logs
- **twitch:** logs errors instead of bailing
- re-introduce twitch features for my channel only
- **autoban:** adjust autoban rules
- **agent:** add logging for bans
- **twitch:** better handling of NOTICE messages
- revoke EightBallProvider privileges
- **config:** make intro banners single-line
- remove `RaidMsgHandler`
- **twitch:** add `initial_channels` option
- add ModeratorManageBannedUsers scope
- adjust logging levels
- **twitch:** send request to get membership capability
- **autoban:** ban scambots based on message content
- throw message when `TWITCH_CLIENT_ID` isn't found
- **twitch:** remove all handlers except auto-ban
- **twitch:** pass HelixClient to twitch handlers
- **twitch:** add autoban handler
- **twitch:** add convenience functions to `TwitchAgent`
- **autoban:** ban based on privmsg too as a fallback
- add autoban handler to twitch bot
- **twitch:** update twitch auth and agent
- **config:** add `twitch_user` config item
- replace println with logging macros
- **twitch:** change up logs a little bit
- **twitch:** use more efficient `clone_into`
- add `Default` impls for `ContentWarningHandler`, `LiftHandler`
- **economy:** disallow sending zero coins
- **logging:** make message update logs less aggressive
- **discord:** enable caching
- **logging:** remove comma in reaction removal embeds
- **logging:** make vc status change embed more helpful
- **logging:** make reaction removal embeds more useful
- **logging:** make MessageUpdate embed more helpful and less annoying
- **logging:** make MessageDelete handler better
- **discord:** add admin commands
- **discord:** add logging module!!
- **db:** add `as_thing`
- add db module for easier database access
- **discord:** add `DiscordHandlerError::from_display`
- **discord:** add `DiscordEventListener` trait
- **nixos:** add option for generating config file
- use config file to set ventriloquists
- **config:** make config writing errors non-fatal and more helpful
- support specifying config file from command line
- add support for config file
- add nixos module
- **quotes:** add addquote command
- **temperature:** limit decimal places in conversions
- **affection:** add boop!
- **bot_affection:** add lick command. thanks, mew.
- add temperature conversion provider
- **discord:** fallback to users' global names before usernames
- **twitch:** handle errors with starting twitch bot
- enable never type feature
- **magical:** make suffix just a period with magicalness < 75%
- **magical:** be less rude about empty magic
- **topic_change:** add question to message and make both responses ChannelMessageWithSource
- **handlers:** add topic change command provider for discord
- **twitch:** add magical handler
- tweak SerenityError variant message
- **economy:** add transfer command
- **economy:** add command descriptions
- **economy:** add impls to convert errors to MuniBotError
- **economy:** add payouts and claim command
- tweak ready messages
- **bot_affection:** tweak rare smooch message
- **bot_affection:** add bite command
- add basic affection handler for twitch
- **twitch:** print to logs when joined
- **twitch:** remove joined notification
- **economy:** curb bigger salaries with sigmoid function
- **economy:** format wallet numbers
- **agent:** remove dbg statement
- **economy:** implement money earning
- **economy:** add wallet command
- **discord:** add util functions for getting display names
- **discord:** add serenity and framework context in handler trait
- **bot_affection:** tweak kiss responses
- **dice:** tweak some critical success messages
- **handlers:** add ventriloquize command provider
- **eight_ball:** tweak eight ball message
- **dice:** add `track_edits` to dice roll command
- add shoutout handler and commands
- add !liftmuni command
- **bot_affection:** change default kiss reaction
- **bot_affection:** add pats and hugs :3
- **discord:** add type aliases and make state mutable
- **magical:** hash a tuple instead of formatted string
- **discord:** add prefixes for discord commands
- enable async_fn_in_trait feature, because why not
- **eight_ball:** have muni_bot shake an actual eight ball
- **greeting:** update greetings
- **eight_ball:** answer with certainty more often than not
- **eight_ball:** add more responses
- add eight ball provider
- **quotes:** use twitch agent to get channel info for quotes
- add `TwitchAgentError`
- add twitch agent to `TwitchMessageHandler`
- **twitch:** add agent to twitch bot
- add `TwitchAgent`
- add quotes functionality
- **main:** print messages when bots exit cleanly
- replace postgres with surrealdb
- remove `rocket` and `open` libs
- use basic token for twitch auth
- replace `lazy_static` with `once_cell`
- **bot_affection:** decrease rarity of smooch responses
- **bot_affection:** make kiss prefixes non-always
- **bot_affection:** fallback to a default string if empty response
- **bot_affection:** add kissies
- **bot_affection:** make prefixes a `ResponseSelection`
- **bot_affection:** add `ActionResponse` for defining rare actions
- **bot_affection:** add boop!
- **bot_affection:** add tilde
- add niceness
- **magical:** use `xxh3_128` instead of 64-bit
- **magical:** call users by their display name
- add `TWITCH_TOKEN` env var to example
- **dice:** add `purpose` parameter to `roll`
- **greeting:** respond to 'henlo' too
- **greeting:** respond to 'hewo' and 'hewwo' too
- **dice:** modify critical success suffixes
- **auth_server:** remove discord oauth routes for now
- **greeting:** use user's display name in a server if available
- add MagicalHandler
- add nuzzle handler
- add dice command provider for discord
- **discord:** add discord command providers
- call dotenv at beginning of main function
- **discord:** get discord integration working
- add `bot` module
- **handlers:** add `TwitchHandlerCollection` and `DiscordHandlerCollection`
- **discord:** add discord and discord/handler modules
- use auth server in main code
- add auth_server module

### bug fixes

- add name to vcgreeter handler
- **logging:** fix database query issues
- store `id`s as `RecordId` instead of `String`
- **logging:** escape formatting in user names
- **logging:** correct link for message update logs
- **bot_affection:** escape backslash
- **logging:** maybe fix "nothing to nothing" error in channel statuses
- **twitch:** try matching by channel_login instead
- **twitch:** use configured bot name in twitch credentials
- **flake:** fix naersk overrides
- **twitch:** reintroduce TwitchAgentError
- **logging:** fix formatting of new invite message
- **logging:** don't announce vs status changes if they're actually the same
- properly use config for database info
- **temperature:** disallow commas in temperature parsing
- **bot_affection:** fix a comment typo (thanks blaze <3)
- **greeting:** don't greet self on discord
- **flake:** update flake to use fenix
- **wallet:** fix transfers not being deducted properly
- **discord:** fix ready message
- **discord:** make discord state immutable to fix deadlocks
- **discord:** fix useless `impl From<DiscordCommandError>`
- **ventriloquize:** fix typing simulation
- **magical:** fix impossibility of 100
- **agent:** fix parsing of twitch api response
- **quotes:** fix issues with quote db queries
- **quotes:** fix use of improper db connection
- **twitch:** fix REDIRECT_URI
- **magical:** remove xxhash and fix magical algorithm
- **bot_affection:** fix suffixes to disappear without an action
- **dice:** change a prefix message
- **bonk:** use StdRng for `Send`-safe rng
- **greeting:** make greeting matching more strict
- **twitch:** use `Arc<Mutex<TwitchAuthState>>` in rocket handler
- add missing imports
- **auth_server:** remove broken code
- **greeting:** make linokii match case insensitive
- **greeting:** fix trait method name
- **handlers/greeting:** fix missed `send_twitch_message` rename

### performance

- use tokio sleep instead of std sleep
- **logging:** remove redundant borrow

### tests

- **autoban:** add test for `matches_scam_message`

### build system

- customize rust toolchain and add wasm32 target
- add `log` and `env_logger` deps
- upgrade libressl
- update dependencies
- enable native-tls on tokio-tungstenite
- add tokio-tungstenite
- update cargo dependencies
- add anyhow dependency
- update cargo dependencies
- update cargo dependencies
- update cargo deps
- **flake:** remove unqlite
- add `url` crate
- update dependencies
- add xxhash-rust dependency
- add .env.example
- **cargo:** update lock file
- **flake:** Upgrade libressl

### documentation

- **topic_change:** add comments
- **wallet:** add function docs
- **bot_affection:** add docs for `get_str_or_empty`
- **eight_ball:** add docs for eight ball discord command
- **bot_affection:** add some comments

---
