# munibot development guidelines

## testing

- If authentication is failing with the local MySQL (MariaDB) instance, try running `devenv tasks run devenv:mysql:configure` to configure users defined in devenv.nix.

## code style guidelines

### error handling

- Error messages use friendly, lowercase language with plain-text emoticons

### imports and structure

- Always place `use` statements at top of files (or module)

### testing

- Test files colocated with implementation
