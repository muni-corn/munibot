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

  # treefmt formatter that combines `dx fmt` (rsx) and `rustywind`
  # (tailwind class sorting) -- see nix/dx-fmt.nix for why
  dxFmt = import ./nix/dx-fmt.nix { inherit pkgs guiRoot; };

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

  # the ai sandbox (munibot_ai::sandbox) talks to rootless podman over its
  # own socket via bollard, never the setuid docker socket. Deliberately no
  # DOCKER_HOST override here: devenv's own `env` is plain Nix, with no
  # shell expansion, so a Nix string literally containing `$XDG_RUNTIME_DIR`
  # would set DOCKER_HOST to that unexpanded text rather than the real
  # per-session path - worse than not setting it at all, since bollard's
  # Docker::connect_with_podman_defaults() checks DOCKER_HOST *first* and
  # would try to use it verbatim. Its own fallback probe of
  # `$XDG_RUNTIME_DIR/podman/podman.sock` (a real shell expansion, evaluated
  # at runtime inside bollard itself) already finds the right socket with no
  # help from this file - see docs/notes/ai-preflight-findings.md finding 11.
  #
  # One-time host setup, outside this devenv shell since it's a per-user
  # systemd unit: `systemctl --user enable --now podman.socket`. Sandbox
  # integration tests are gated behind a feature flag (see
  # munibot_ai/Cargo.toml's "sandbox-integration" feature) so `devenv test`
  # stays green on a machine where this was never set up.

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
      podman
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

  # add `dx-fmt` to the treefmt config provided by musicaloft-shell. all
  # other formatters (nixfmt, oxfmt, kdlfmt, typos, rustfmt) come from
  # there.
  treefmt.config.settings.formatter.dx-fmt = {
    command = lib.getExe dxFmt;
    # runs after rustfmt (priority 0) so rsx gets formatted against
    # already-settled rust code, rather than the other way around
    priority = 1;
    includes = [ "*.rs" ];
  };

  # dx bundle produces munibot's fullstack binary (bots + gui) directly, now
  # that munibot is a dioxus app rather than a plain bot binary -- see
  # nix/build.nix. this replaced a crate2nix-based build.
  outputs.default = import ./nix/build.nix { inherit config pkgs; };
}
