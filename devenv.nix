{
  config,
  pkgs,
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
in
{
  enterTest = ''
    cargo test
  '';

  env = {
    RUST_LOG = "error,munibot=debug,munibot_core=debug,munibot_discord=debug,munibot_twitch=debug";
    LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
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
      diesel-cli
      flyctl
    ]
    ++ buildInputs
    ++ nativeBuildInputs
    ++ (builtins.attrValues config.treefmt.config.build.programs);

  services.mysql = {
    enable = true;
    ensureUsers = [
      {
        name = "root";
        password = "sillylittlepassword";
      }
      {
        name = "munibot";
        password = "sillylittlepassword";
        ensurePermissions."munibot.*" = "ALL PRIVILEGES";
      }
      {
        name = "munibot_test";
        password = "sillylittlepassword";
        ensurePermissions."`munibot\\_test\\_%`.*" = "ALL PRIVILEGES";
      }
    ];
    initialDatabases = [
      { name = "munibot"; }
      { name = "munibot_test"; }
    ];
  };

  outputs.default =
    let
      # Bypass devenv's config.languages.rust.import, which assumes a single
      # root crate and fails for workspaces (cachix/devenv#2672). Instead,
      # call crate2nix directly and access the workspace member by name.
      crate2nixInput = config.lib.getInput {
        name = "crate2nix";
        url = "github:nix-community/crate2nix";
        attribute = "outputs.default";
        follows = [ "nixpkgs" ];
      };

      crate2nixTools = pkgs.callPackage "${crate2nixInput}/tools.nix" { };

      cargoNix =
        pkgs.callPackage
          (crate2nixTools.generatedCargoNix {
            name = pname;
            src = ./.;
          })
          {
            # use the same nightly toolchain configured for the dev shell
            buildRustCrateForPkgs =
              _:
              pkgs.buildRustCrate.override {
                rustc = config.languages.rust.toolchainPackage;
                cargo = config.languages.rust.toolchainPackage;
              };
          };

      args = {
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

          # customize munibot binary's build inputs
          ${pname} = _attrs: {
            inherit buildInputs nativeBuildInputs;

            # embed rpath so the installed binary finds its dynamic libraries
            runtimeDependencies = buildInputs;

            # required by bindgen (mysql, openssl build scripts)
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };

          # workspace crates that also need native libs
          munibot_core = _attrs: {
            inherit buildInputs nativeBuildInputs;
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
            # embed_migrations! resolves "../migrations" relative to the build
            # dir, but crate2nix unpacks each crate in isolation. symlink the
            # workspace migrations folder into the parent build directory so the
            # relative path works correctly.
            preBuild = ''
              ln -s ${./migrations} ../migrations
            '';
          };

          munibot_discord = _attrs: {
            inherit buildInputs nativeBuildInputs;
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };

          munibot_twitch = _attrs: {
            inherit buildInputs nativeBuildInputs;
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };
        };
      };
    in
    cargoNix.workspaceMembers.${pname}.build.override args;
}
