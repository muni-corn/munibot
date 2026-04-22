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
in
{
  dotenv.enable = true;

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
    config.languages.rust.import ./. args;
}
