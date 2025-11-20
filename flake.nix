{
  description = "munibot, the cutest bot for Discord and Twitch, personality included";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    devenv = {
      url = "github:cachix/devenv";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };

    git-hooks-nix = {
      url = "github:cachix/git-hooks.nix";
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
        inputs.devenv.flakeModule
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

          # Leptos/Rust dependencies
          buildInputs = with pkgs; [ libressl_4_0 ];
          # Additional build inputs for Leptos
          nativeBuildInputs = with pkgs; [
            clang
            glibc
            leptosfmt
            pkg-config
            trunk
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
              channel = "stable";
              mold.enable = true;
              # Add WASM target for Leptos
              targets = [ "wasm32-unknown-unknown" ];
            };

            packages = [
              config.treefmt.build.wrapper
              pkgs.bacon
              pkgs.cargo-edit
              pkgs.cargo-outdated
              pkgs.cargo-release
              pkgs.cargo-watch
              pkgs.flyctl
            ]
            ++ buildInputs
            ++ nativeBuildInputs
            ++ (builtins.attrValues config.treefmt.build.programs);

            # git hooks
            git-hooks.hooks = {
              # commit linting
              commitlint-rs =
                let
                  config = pkgs.writers.writeYAML "commitlintrc.yml" {
                    rules = {
                      description-empty.level = "error";
                      description-format = {
                        level = "error";
                        format = "^[a-z].*$";
                      };
                      description-max-length = {
                        level = "error";
                        length = 72;
                      };
                      scope-max-length = {
                        level = "warning";
                        length = 10;
                      };
                      scope-empty.level = "warning";
                      type = {
                        level = "error";
                        options = [
                          "build"
                          "chore"
                          "ci"
                          "docs"
                          "dx"
                          "feat"
                          "fix"
                          "perf"
                          "refactor"
                          "revert"
                          "style"
                          "test"
                        ];
                      };
                    };
                  };

                in
                {
                  enable = true;
                  name = "commitlint-rs";
                  package = pkgs.commitlint-rs;
                  description = "Validate commit messages with commitlint-rs";
                  entry = "${pkgs.lib.getExe pkgs.commitlint-rs} -g ${config} -e";
                  always_run = true;
                  stages = [ "commit-msg" ];
                };

              # format on commit
              treefmt = {
                enable = true;
                packageOverrides.treefmt = config.treefmt.build.wrapper;
              };
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

          # formatting
          treefmt.programs = {
            leptosfmt.enable = true;
            nixfmt.enable = true;
            rustfmt.enable = true;
            taplo.enable = true;
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
