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

    musicaloft-shell = {
      url = "github:musicaloft/musicaloft-shell/devenv";
      flake = false;
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

      imports = [ inputs.devenv.flakeModule ];

      perSystem =
        {
          config,
          ...
        }:
        {
          devenv.shells.default.imports = [
            "${inputs.musicaloft-shell}/devenv.nix"
            ./devenv.nix
          ];

          # package build
          packages = config.devenv.shells.default.outputs;
        };

      flake = {
        overlays.default = final: prev: {
          munibot = inputs.self.packages.${prev.system}.default;
        };

        nixosModules.default = import ./nix/nixos.nix inputs.self;
      };
    };
}
