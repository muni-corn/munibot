{
  description = "munibot, the cutest bot for Discord and Twitch, personality included";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    devenv = {
      url = "github:cachix/devenv";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix2container.url = "github:nlewo/nix2container";
    nix2container.inputs = {
      nixpkgs.follows = "nixpkgs";
    };

    mk-shell-bin.url = "github:rrbutani/nix-mk-shell-bin";

    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };

    git-hooks-nix = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    musicaloft-style = {
      url = "github:musicaloft/musicaloft-style";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    rust-flake = {
      url = "github:juspay/rust-flake";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    devenv-root = {
      url = "file+file:///dev/null";
      flake = false;
    };
  };

  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      imports = [
        inputs.git-hooks-nix.flakeModule
        inputs.musicaloft-style.flakeModule
        inputs.rust-flake.flakeModules.default
        inputs.rust-flake.flakeModules.nixpkgs
        inputs.treefmt-nix.flakeModule
      ];

      perSystem =
        {
          config,
          pkgs,
          ...
        }:
        let
          name = "munibot";

          # runtime dependencies
          buildInputs = with pkgs; [ libressl_4_2 ];

          # native build-time dependencies
          nativeBuildInputs = with pkgs; [
            clang
            glibc
            dioxus-cli
            pkg-config
          ];
        in
        {
          # rust setup
          devenv.shells.default = {
            env = {
              RUST_LOG = "error,munibot=debug";
              LEPTOS_TAILWIND_VERSION = "v3.4.14";
              LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            };

            languages.rust = {
              enable = true;
              channel = "nightly";
              mold.enable = true;
              # add wasm target for web gui
              targets = [ "wasm32-unknown-unknown" ];
            };

            packages =
              with pkgs;
              [
                config.treefmt.build.wrapper
                bacon
                cargo-edit
                cargo-outdated
                cargo-release
                cargo-watch
                flyctl
              ]
              ++ buildInputs
              ++ nativeBuildInputs
              ++ (builtins.attrValues config.treefmt.build.programs);

            services.mysql = {
              enable = true;
              ensureUsers = [
                {
                  name = "munibot";
                  password = "sillylittlepassword";
                  ensurePermissions."munibot.*" = "ALL PRIVILEGES";
                }
              ];
              initialDatabases = [
                { name = "munibot"; }
              ];
            };
          };

          # setup rust packages
          rust-project = {
            # ensure assets and style files are included with build
            src = pkgs.lib.cleanSourceWith {
              src = inputs.self;
              filter =
                path: type:
                (pkgs.lib.hasInfix "/assets/" path)
                || (pkgs.lib.hasInfix "/style/" path)
                || (pkgs.lib.hasSuffix "tailwind.config.js" path)
                || (config.rust-project.crane-lib.filterCargoSources path type);
            };

            # use the same rust toolchain from the dev shell for consistency
            toolchain = config.devenv.shells.default.languages.rust.toolchainPackage;

            # specify dependencies
            defaults.perCrate.crane.args = {
              inherit nativeBuildInputs buildInputs;
              # Additional environment variables for Leptos
              LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
              LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
            };
          };

          packages.default = config.rust-project.crates.${name}.crane.outputs.packages.${name};

          # `nix flake check`
          checks = {
            clippy = config.rust-project.crates.${name}.crane.outputs.clippy;
            formatting = config.treefmt.build.check inputs.self;
          };
        };

      flake = {
        overlays.default = final: prev: {
          munibot = inputs.self.packages.${prev.system}.default;
        };

        nixosModules.default = import ./nix/nixos.nix inputs.self;
      };
    };
}
