{
  description = "muni_bot";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix.url = "github:nix-community/fenix";
    utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    fenix,
    utils,
  }:
    utils.lib.eachDefaultSystem
    (system: let
      name = "muni_bot";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [fenix.overlays.default]; # for rust-analyzer-nightly
      };
      lib = pkgs.lib;

      # make rust toolchain
      toolchain = with fenix.packages.${system};
        combine [
          complete.rust-src
          complete.rustc-codegen-cranelift-preview
          default.cargo
          default.clippy
          default.rustfmt
          rust-analyzer
          targets.wasm32-unknown-unknown.latest.rust-std
        ];

      # make build library
      craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

      # build artifacts
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      # establish commonly used arguments
      commonArgs = {
        src = lib.cleanSourceWith {
          src = self;
          filter = path: type:
            (lib.hasInfix "/assets/" path)
            || (lib.hasInfix "/style/" path)
            || (lib.hasSuffix "tailwind.config.js" path)
            || (craneLib.filterCargoSources path type);
        };
        strictDeps = true;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;

        inherit nativeBuildInputs buildInputs cargoArtifacts;
      };

      nativeBuildInputs = with pkgs; [
        clang
        glibc
        leptosfmt
        pkg-config
        trunk
      ];
      buildInputs = with pkgs; [libressl];

      muni_bot = craneLib.buildPackage (commonArgs
        // {
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
        });
    in {
      # `nix build`
      packages.default = muni_bot;

      # `nix run`
      apps.default = utils.lib.mkApp {
        name = name;
        drv = self.packages."${system}".default;
        exePath = "/bin/${name}";
      };

      # `nix flake check`
      checks = {
        inherit muni_bot;
        clippy =
          craneLib.cargoClippy (commonArgs
            // {cargoClippyExtraArgs = "--all-targets --all-features";});
      };

      # `nix develop`
      devShells.default = let
        moldDevShell =
          craneLib.devShell.override
          {
            mkShell = pkgs.mkShell.override {
              stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;
            };
          };
      in
        moldDevShell {
          checks = self.checks.${system};

          packages = with pkgs; [leptosfmt cargo-watch cargo-outdated flyctl cargo-machete];

          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          RUST_LOG = "error,muni_bot=info";
          LEPTOS_TAILWIND_VERSION = "v3.4.14";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
        };
    })
    // {
      overlays.default = final: prev: {
        muni_bot = self.packages.${prev.system}.default;
      };

      nixosModules.default = import ./nix/nixos.nix self;
    };
}
