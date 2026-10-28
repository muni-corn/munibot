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
#
# dioxus-cli also carries a patch (nix/patches) that teaches its fingerprint
# busting about cargo's build-dir layout v2. if a version bump makes the patch
# fail to apply, check whether upstream has fixed this and drop it if so.
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
    old:
    let
      version = crateVersion "dioxus";
      src = final.fetchCrate {
        pname = "dioxus-cli";
        inherit version;
        hash = "sha256-n1Lk9eCjr1vGqvCDbrOmbU7TTvdcGdx/chRsj4Z3a+Q=";
      };
    in
    {
      inherit src version;
      patches = (old.patches or [ ]) ++ [ ./patches/dioxus-cli-build-dir-v2.patch ];
      cargoDeps = final.rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-nqGrW0Sf7DWdrSXSKcSl+FlO+1NY4uZA7G7G4AGItuw=";
      };
    }
  );

  wasm-bindgen-cli-pinned =
    let
      version = crateVersion "wasm-bindgen";
      src = final.fetchCrate {
        pname = "wasm-bindgen-cli";
        inherit version;
        hash = "sha256-pcecKQd7E8Opw6bkFoE569epUi7gh5qpQF1e5PJY6V8=";
      };
    in
    final.buildWasmBindgenCli {
      inherit src;
      cargoDeps = prev.rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-vmUrWVU7kPJJxO5qIVeAkwQyWDELO1Z4Z5gitz2kco8=";
      };
    };
}
