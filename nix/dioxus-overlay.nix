# pins dioxus-cli and wasm-bindgen-cli-pinned to the exact versions the
# `dioxus` and `wasm-bindgen` crates resolve to in Cargo.lock -- see
# devenv.nix for where this is applied.
#
# the *version* of each cli below is read straight out of Cargo.lock, so it
# can never drift out of sync with the crates again. the fetch hashes can't
# be derived the same way -- Cargo.lock only records checksums for the
# library crates, not for these cli tools or their vendored dependencies --
# so they still need a manual bump whenever the version changes. nix will
# fail the build and print the correct hash to paste in when that happens.
final: prev:
let
  inherit (final) lib;

  cargoLock = fromTOML (builtins.readFile ../Cargo.lock);

  # looks up the version of `name` resolved in Cargo.lock
  crateVersion =
    name:
    (lib.findFirst (
      pkg: pkg.name == name
    ) (throw "'${name}' not found in Cargo.lock") cargoLock.package).version;
in
{
  dioxus-cli = prev.dioxus-cli.overrideAttrs (
    _:
    let
      version = crateVersion "dioxus";
      src = final.fetchCrate {
        pname = "dioxus-cli";
        inherit version;
        hash = "sha256-4x9xTc9FW03ohEhDOe+wJ0EJ4yR8HWFmiEA+hvlLF7Q=";
      };
    in
    {
      inherit src version;
      cargoDeps = final.rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-eGGdmI5dvNav2fJmDv/GD7Anfd0lRModfgfEg+Jg3CQ=";
      };
    }
  );

  wasm-bindgen-cli-pinned =
    let
      version = crateVersion "wasm-bindgen";
      src = final.fetchCrate {
        pname = "wasm-bindgen-cli";
        inherit version;
        hash = "sha256-H6Is3fiZVxZCfOMWK5dWMSrtn50VGv0sfdnsT+cTtyk=";
      };
    in
    final.buildWasmBindgenCli {
      inherit src;
      cargoDeps = prev.rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-VucqkXbCi4qtQzY/HrXiDnbSURsagPsdNVMn1Tw3UiY=";
      };
    };
}
