# munibot development commands

## build and test

- `cargo build` - Build the project
- `cargo test` - Run all tests
- `cargo test test_name` - Run specific test by name
- `cargo test handlers::autoban::tests::test_matches_scam_message` - Run single
  test
- `cargo clippy` - Run linter
- `cargo clippy --fix` - Auto-fix linting issues

## formatting

- `treefmt` - Format all code (Rust, TOML, Markdown)
- `rustfmt src/main.rs` - Format specific file
- `rustfmt --check src/main.rs` - Check formatting without changes

## code style guidelines

### error handling

- Use `MuniBotError` enum for application errors (src/lib.rs:19-50)
- Use `thiserror` for error derivation
- Error messages use friendly, lowercase language with plain-text emoticons

### imports and structure

- Group imports: std -> external crates -> internal modules
- Use `use` statements at top of files
- Module structure: `src/handlers/`, `src/discord/`, `src/twitch/`

### naming conventions

- Snake_case for functions and variables
- PascalCase for types and structs
- UPPER_SNAKE_CASE for constants
- Descriptive names: `DiscordCommandProvider`, `TwitchMessageHandler`

### async patterns

- Use `async-trait` for trait async methods
- Tokio runtime with `#[tokio::main]`
- Error handling with `Result<T, MuniBotError>`

### testing

- Tests in `#[cfg(test)]` modules
- Use `assert!` and `assert_eq!` for assertions
- Test files colocated with implementation
