{
  description = "munibot, the cutest bot for Discord and Twitch, personality included";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crate2nix = {
      url = "github:nix-community/crate2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

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

    mk-shell-bin.url = "github:rrbutani/nix-mk-shell-bin";

    musicaloft-style = {
      url = "github:musicaloft/musicaloft-style";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    nix2container = {
      url = "github:nlewo/nix2container";
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
      ];

      perSystem =
        {
          config,
          lib,
          pkgs,
          system,
          ...
        }:
        let
          pname = "munibot";

          # runtime dependencies
          buildInputs = with pkgs; [
            libressl_4_2
            libmysqlclient
          ];

          # native build-time dependencies
          nativeBuildInputs = with pkgs; [
            clang
            glibc
            dioxus-cli
            pkg-config
          ];

          toolchain = config.devenv.shells.default.languages.rust.toolchainPackage;
        in
        {
          # unfree packages are required for surrealdb
          # ~~yet another reason to switch away to mysql/mariadb~~
          _module.args.pkgs = lib.mkForce (
            import inputs.nixpkgs {
              inherit system;
              config.allowUnfree = true;
            }
          );

          # rust setup
          devenv.shells.default = {
            enterTest = ''
              cargo test
            '';

            env = {
              RUST_LOG = "error,munibot=debug";
              LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            };

            git-hooks.hooks.clippy = {
              enable = true;
              packageOverrides = {
                cargo = toolchain;
                clippy = toolchain;
              };
            };

            languages.rust = {
              enable = true;
              channel = "nightly";
              mold.enable = true;
              # add wasm target for web gui
              targets = [ "wasm32-unknown-unknown" ];
              # embed rpath so dev binaries find dynamic libs without LD_LIBRARY_PATH
              rustflags = "-C link-args=-Wl,-rpath,${pkgs.lib.makeLibraryPath buildInputs}";
            };

            packages =
              with pkgs;
              [
                bacon
                cargo-edit
                cargo-outdated
                cargo-release
                cargo-watch
                diesel-cli
                flyctl
              ]
              ++ buildInputs
              ++ nativeBuildInputs
              ++ (builtins.attrValues config.devenv.shells.default.treefmt.config.build.programs);

            processes.surrealdb = {
              exec = "${pkgs.surrealdb}/bin/surreal start --user root --pass root --bind 0.0.0.0:8000 memory";
              process-compose.is_elevated = true;
            };

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

          packages.default =
            let
              crate2nixTools = pkgs.callPackage "${inputs.crate2nix}/tools.nix" { };
              # Use generatedCargoNix + callPackage so we can pass buildRustCrateForPkgs
              # and wire in the nightly toolchain. languages.rust.import (appliedCargoNix)
              # always defaults to pkgs.buildRustCrate; its .override only exposes
              # { features, crateOverrides } and cannot change the underlying toolchain.
              cargoNix =
                pkgs.callPackage
                  (crate2nixTools.generatedCargoNix {
                    inherit pname;
                    src = ./.;
                  })
                  {
                    buildRustCrateForPkgs =
                      _pkgs:
                      pkgs.buildRustCrate.override {
                        rustc = toolchain;
                        cargo = toolchain;
                      };
                  };
            in
            cargoNix.rootCrate.build.override {
              crateOverrides = pkgs.defaultCrateOverrides // {
                # libressl provides openssl.pc; give openssl-sys explicit access
                openssl-sys = _attrs: {
                  buildInputs = [ pkgs.libressl_4_2.dev ];
                  nativeBuildInputs = [ pkgs.pkg-config ];
                };
                # mysql client bindings need libmysqlclient via pkg-config
                mysqlclient-sys = _attrs: {
                  buildInputs = [ pkgs.libmysqlclient ];
                  nativeBuildInputs = [ pkgs.pkg-config ];
                };

                # customize munibot's build inputs
                ${pname} = attrs: {
                  # include assets and style files alongside rust sources for dioxus
                  src = pkgs.lib.cleanSourceWith {
                    src = inputs.self;
                    filter =
                      path: type:
                      (pkgs.lib.hasInfix "/assets/" path)
                      || (pkgs.lib.hasInfix "/style/" path)
                      || (pkgs.lib.hasSuffix "tailwind.config.js" path)
                      || (pkgs.lib.cleanSourceFilter path type);
                  };

                  inherit buildInputs nativeBuildInputs;

                  # embed rpath so the installed binary finds its dynamic libraries
                  runtimeDependencies = buildInputs;

                  # required by bindgen (mysql, openssl build scripts)
                  LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
                };
              };
            };

          # `nix flake check`
          checks.treefmt = config.devenv.shells.default.treefmt.config.build.check inputs.self;
        };

      flake = {
        overlays.default = final: prev: {
          munibot = inputs.self.packages.${prev.system}.default;
        };

        nixosModules.default = import ./nix/nixos.nix inputs.self;
      };
    };
}
