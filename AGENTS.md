# munibot development guidelines

## testing

- If authentication is failing with the local MySQL (MariaDB) instance, try running `devenv tasks run devenv:mysql:configure` to configure users defined in devenv.nix.

## code style guidelines

### error handling

- Error messages use friendly, lowercase language

### imports and structure

- Always place `use` statements at top of files (or module)
- Don't use qualified import paths inline; ALWAYS `use` what you need
  - If necessary, inline paths should have only up to 3 segments (e.g. `db::models::User`)

### testing

- Test files colocated with implementation
