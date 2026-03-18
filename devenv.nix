{
  config,
  inputs,
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

  toolchain = config.languages.rust.toolchainPackage;
in
{
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
    ++ (builtins.attrValues config.treefmt.config.build.programs);

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

  outputs.default =
    let
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
    in
    config.languages.rust.import ./. args;
}
