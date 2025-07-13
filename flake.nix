{
  description = "munibot, the cutest bot for Discord and Twitch, personality included";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      fenix,
      treefmt-nix,
      utils,
    }:
    utils.lib.eachDefaultSystem (
      system:
      let
        name = "munibot";
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;

        # treefmt formatting
        treefmtEval = treefmt-nix.lib.evalModule pkgs ./treefmt.nix;

        # make rust toolchain
        toolchain =
          with fenix.packages.${system};
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
            filter =
              path: type:
              (lib.hasInfix "/assets/" path)
              || (lib.hasInfix "/style/" path)
              || (lib.hasSuffix "tailwind.config.js" path)
              || (craneLib.filterCargoSources path type);
          };
          strictDeps = true;
          stdenv = p: p.stdenvAdapters.useMoldLinker p.stdenv;

          inherit nativeBuildInputs buildInputs cargoArtifacts;
        };

        nativeBuildInputs = with pkgs; [
          clang
          glibc
          leptosfmt
          pkg-config
          trunk
        ];
        buildInputs = with pkgs; [ libressl_4_0 ];

        munibot = craneLib.buildPackage (
          commonArgs
          // {
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
          }
        );
      in
      {
        # `nix build`
        packages.default = munibot;

        # `nix run`
        apps.default = utils.lib.mkApp {
          name = name;
          drv = self.packages."${system}".default;
          exePath = "/bin/${name}";
        };

        # `nix flake check`
        checks = {
          inherit munibot;
          clippy = craneLib.cargoClippy (
            commonArgs // { cargoClippyExtraArgs = "--all-targets --all-features"; }
          );
          formatting = treefmtEval.config.build.check self;
        };

        # `nix develop`
        devShells.default =
          let
            moldDevShell = craneLib.devShell.override {
              mkShell = pkgs.mkShell.override {
                stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;
              };
            };
          in
          moldDevShell {
            checks = self.checks.${system};

            packages =
              buildInputs
              ++ (with pkgs; [
                leptosfmt
                cargo-release
                cargo-watch
                cargo-outdated
                flyctl
                cargo-machete
              ]);

            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            RUST_LOG = "error,munibot=debug";
            LEPTOS_TAILWIND_VERSION = "v3.4.14";
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
          };

        # `nix fmt`
        formatter = treefmtEval.config.build.wrapper;
      }
    )
    // {
      overlays.default = final: prev: {
        munibot = self.packages.${prev.system}.default;
      };

      nixosModules.default = import ./nix/nixos.nix self;
    };
}
