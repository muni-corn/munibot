{
  config,
  lib,
  pkgs,
  ...
}:
let
  # tailwind scans munibot_gui's components and outputs into its assets/,
  # since that's the crate whose asset!() reference resolves the css
  guiRoot = "${config.git.root}/munibot_gui";

  # runtime dependencies
  buildInputs = with pkgs; [
    atk
    glib
    gtk3
    libmysqlclient
    libressl_4_2
    libsoup_3
    webkitgtk_4_1
    xdotool
  ];

  # native build-time dependencies
  nativeBuildInputs = with pkgs; [
    binaryen
    clang
    glibc
    dioxus-cli
    pkg-config
    wrapGAppsHook4
  ];
in
{
  # pins dioxus-cli and wasm-bindgen-cli to match the versions the `dioxus`
  # and `wasm-bindgen` crates resolve to in Cargo.lock
  overlays = [ (import ./nix/dioxus-overlay.nix) ];

  enterTest = ''
    cargo test
  '';

  env = {
    RUST_LOG = "error,munibot=debug,munibot_api=debug,munibot_core=debug,munibot_discord=debug,munibot_gui=debug,munibot_twitch=debug";
    LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
    # matches the redirect URI registered with discord for local development
    PORT = 8080;
  };

  # setup helix to format dioxus rust and scss with treefmt
  files.".helix/languages.toml".toml.language = [
    {
      name = "rust";
      formatter = {
        command = "treefmt";
        args = [
          "--stdin"
          "foo.rs"
        ];
      };
      language-servers = [
        {
          name = "rust-analyzer";
          except-features = [ "format" ];
        }
        "tailwind"
      ];
    }
    {
      name = "scss";
      formatter = {
        command = "treefmt";
        args = [
          "--stdin"
          "foo.scss"
        ];
      };
      language-servers = [
        {
          name = "vscode-css-language-server";
          except-features = [ "format" ];
        }
        "tailwind"
      ];
    }
  ];

  languages = {
    javascript = {
      enable = true;
      directory = "./munibot_gui";
      lsp.enable = false;
      pnpm = {
        enable = true;
        install.enable = true;
      };
    };
    rust = {
      enable = true;
      channel = "nightly";
      # mold.enable is off: it sets RUSTFLAGS=-C link-arg=-fuse-ld=mold
      # globally, which breaks the wasm32-unknown-unknown build (rust-lld
      # doesn't understand -fuse-ld, since wasm has no cc-style linker driver)
      targets = [ "wasm32-unknown-unknown" ];
    };
  };

  packages =
    with pkgs;
    [
      diesel-cli
      flyctl
      tailwindcss_4
      wasm-bindgen-cli-pinned
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

  # backs gui login sessions (see munibot_api/src/auth)
  services.redis.enable = true;

  processes = {
    tailwind = {
      exec = "${lib.getExe pkgs.tailwindcss_4} -i ./tailwind.css -o ./assets/tailwind.css";
      cwd = guiRoot;
      watch = {
        # watch the whole gui crate so tailwind rebuilds on any component change
        paths = [ guiRoot ];
        extensions = [
          "css"
          "rs"
          "toml"
        ];
        ignore = [ "target" ];
      };
    };

    dx-serve = {
      exec = "secretspec run -- ${lib.getExe pkgs.dioxus-cli} serve --package munibot";
      after = [
        "devenv:processes:mysql"
        "devenv:processes:redis"
      ];
      ready.http.get.port = 8080;
    };
  };

  # dx bundle produces munibot's fullstack binary (bots + gui) directly, now
  # that munibot is a dioxus app rather than a plain bot binary -- see
  # nix/build.nix. this replaced a crate2nix-based build.
  outputs.default = import ./nix/build.nix { inherit config pkgs; };
}
