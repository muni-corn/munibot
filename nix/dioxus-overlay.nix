# pins dioxus-cli and wasm-bindgen-cli to the exact versions the `dioxus` and
# `wasm-bindgen` crates resolve to in Cargo.lock. these two must always match
# each other and the crate versions -- see devenv.nix for where this is
# applied, and Cargo.lock for the versions currently in use.
final: prev: {
  dioxus-cli = prev.dioxus-cli.overrideAttrs (
    _:
    let
      version = "0.7.9";
      src = final.fetchCrate {
        pname = "dioxus-cli";
        inherit version;
        hash = "sha256-tLMtUlohSJt3okdJh+ARweQNGmzj/vYiNl8iZhDbSAc=";
      };
    in
    {
      inherit src version;
      cargoDeps = final.rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-h5wkxHP8ehZLHqcUsro08/dpqSPnPuBbZuUGG8i4nBc=";
      };
    }
  );

  wasm-bindgen-cli-pinned =
    let
      src = final.fetchCrate {
        pname = "wasm-bindgen-cli";
        version = "0.2.122";
        hash = "sha256-vO4RSxi/sMWxmsEs3GuljdMfIRSu75A+Q+c5wgYToRU=";
      };
    in
    final.buildWasmBindgenCli {
      inherit src;
      cargoDeps = prev.rustPlatform.fetchCargoVendor {
        inherit src;
        inherit (src) pname version;
        hash = "sha256-Inup6vvJSG5ghNyeDPyZbfZo4d0LsMG2OJfStoaeDBs=";
      };
    };
}
