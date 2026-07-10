# Builds munibot's fullstack bundle (server binary + prebuilt web assets)
# with `dx bundle`, replacing the previous crate2nix build of the plain bot
# binary now that munibot is a dioxus fullstack app.
#
# dx 0.7.x outputs to: target/dx/<pname>/release/web/
#   server              <- the axum server binary (bots + gui)
#   public/index.html
#   public/assets/      <- wasm, js glue, css, and static assets (all hashed)
#
# the final $out layout mirrors that so the server binary can find public/
# without any env-var override:
#   $out/bin/munibot     <- wrapped server binary
#   $out/bin/public/     <- pre-built web assets
{ config, pkgs, ... }:
let
  pname = "munibot";
  version = (fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;

  # the nightly toolchain with both linux and wasm32 targets, as configured
  # by the devenv rust language module (devenv.nix / languages.rust)
  toolchain = config.languages.rust.toolchainPackage;

  # native build inputs for the server (linux) compilation
  serverNativeBuildInputs = with pkgs; [
    autoPatchelfHook
    clang
    gcc
    libclang
    pkg-config
  ];

  # runtime/library deps linked into the server binary
  serverBuildInputs = with pkgs; [
    libressl_4_2
    libmysqlclient
  ];

  # source filter for the dx bundle build. this must include the *whole
  # workspace*, not just munibot/, since munibot path-depends on
  # munibot_core/munibot_discord/munibot_twitch. migrations/ and
  # diesel.toml live at the workspace root; munibot_core's embed_migrations!
  # resolves "../migrations" relative to its own directory, which this
  # preserves correctly (unlike crate2nix, which unpacks each crate in
  # isolation and needed a symlink workaround for this).
  bundleSrc = pkgs.lib.cleanSourceWith {
    src = ../.;
    filter =
      path: type:
      # include directories so cleanSourceWith can traverse into them
      (type == "directory")
      || (pkgs.lib.hasInfix "/assets/" path)
      || (pkgs.lib.hasInfix "/src/" path)
      || (pkgs.lib.hasInfix "/migrations/" path)
      || (pkgs.lib.hasSuffix "Cargo.toml" path)
      || (pkgs.lib.hasSuffix "Cargo.lock" path)
      || (pkgs.lib.hasSuffix "Dioxus.toml" path)
      || (pkgs.lib.hasSuffix "diesel.toml" path)
      # tailwind input file: manganis validates /assets/tailwind.css at
      # compile time, which we generate from this source before dx bundle
      || (pkgs.lib.hasSuffix "tailwind.css" path);
  };
in
pkgs.stdenvNoCC.mkDerivation {
  inherit pname version;
  src = bundleSrc;

  cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ../Cargo.lock; };

  nativeBuildInputs = [
    toolchain
    pkgs.dioxus-cli
    pkgs.wasm-bindgen-cli_0_2_122
    pkgs.binaryen
    pkgs.tailwindcss_4
    pkgs.makeWrapper
    pkgs.gcc
    pkgs.pkg-config
    pkgs.cmake
    pkgs.perl
    pkgs.rustPlatform.cargoSetupHook
  ]
  ++ serverNativeBuildInputs;

  buildInputs = serverBuildInputs;

  dontConfigure = true;

  # libmysqlclient's actual libmariadb.so.3 lives under lib/mariadb/, not
  # lib/ itself, which is one level too deep for autoPatchelfHook's default
  # search -- register it explicitly so the fixup phase can find it
  preFixup = ''
    addAutoPatchelfSearchPath ${pkgs.libmysqlclient}/lib/mariadb
  '';

  buildPhase = ''
    runHook preBuild

    # nix sandbox sets HOME to /homeless-shelter; move it somewhere writable
    export HOME=$TMPDIR
    export CARGO_HOME=$TMPDIR/cargo-home

    # required by bindgen (mysql, openssl build scripts)
    export LIBCLANG_PATH=${pkgs.libclang.lib}/lib

    export DIOXUS_APP_TITLE="munibot"
    export DIOXUS_PRODUCT_NAME="munibot"
    export DIOXUS_TELEMETRY_ENABLED=false
    export CARGO_NET_OFFLINE=true
    # tell dx to use the PATH wasm-opt instead of downloading its own copy
    export NO_DOWNLOADS=1

    # the dioxus crate lives in its own directory, not the workspace root
    cd munibot

    # manganis validates asset paths at compile time; pre-generate the
    # tailwind output so the asset!("/assets/tailwind.css") macro resolves
    mkdir -p assets
    tailwindcss -i ./tailwind.css -o assets/tailwind.css --minify

    # dx's wasm-opt invocation triggers SIGABRT under the nix sandbox because
    # binaryen's thread pool spawning is blocked by the seccomp profile.
    # intercept it with a passthrough stub so dx succeeds, then run the real
    # wasm-opt (by absolute path, since this stub stays on PATH) below with
    # threading flags omitted.
    FAKE_OPT="$TMPDIR/fake-wasm-opt/bin"
    mkdir -p "$FAKE_OPT"

    cat > "$FAKE_OPT/wasm-opt" << 'EOF'
    #!/bin/sh
    # passthrough stub: copies input to -o output so dx reports success.
    # real wasm optimization runs after dx bundle with threading flags removed.
    INPUT=""
    OUTPUT=""
    NEXT_IS_OUTPUT=0
    for arg in "$@"; do
      if [ "$NEXT_IS_OUTPUT" = "1" ]; then
        OUTPUT="$arg"
        NEXT_IS_OUTPUT=0
      elif [ "$arg" = "-o" ]; then
        NEXT_IS_OUTPUT=1
      elif [ -f "$arg" ]; then
        INPUT="$arg"
      fi
    done
    if [ -n "$INPUT" ] && [ -n "$OUTPUT" ] && [ "$INPUT" != "$OUTPUT" ]; then
      cp "$INPUT" "$OUTPUT"
    fi
    exit 0
    EOF

    chmod +x "$FAKE_OPT/wasm-opt"
    export PATH="$FAKE_OPT:$PATH"

    dx bundle --release --fullstack --locked --offline

    # cargo workspaces share one target/ at the workspace root, even though
    # dx was run from munibot/ -- return there so the remaining paths (here
    # and in installPhase) don't need a leading ../
    cd ..

    # run the real wasm-opt (absolute path -- the stub shadows the bare name
    # on PATH) on the bundled wasm without --enable-threads
    WASM=$(find "target/dx/${pname}/release/web/public/assets" \
             -name "*_bg-dxh*.wasm" | head -1)
    if [ -n "$WASM" ]; then
      WASM_TMP=$(mktemp "$TMPDIR/wasm-opt-XXXXXX.wasm")
      "${pkgs.binaryen}/bin/wasm-opt" \
        "$WASM" \
        -Oz \
        -o "$WASM_TMP" \
        --enable-reference-types \
        --enable-bulk-memory \
        --enable-mutable-globals \
        --enable-nontrapping-float-to-int \
        --strip-debug
      mv "$WASM_TMP" "$WASM"
    fi

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin

    DX_OUT="target/dx/${pname}/release/web"

    # place the server binary next to public/ so it can discover the
    # assets without any env-var override
    cp "$DX_OUT/server" $out/bin/${pname}
    cp -r "$DX_OUT/public" $out/bin/public

    wrapProgram $out/bin/${pname} \
      --set-default IP   0.0.0.0 \
      --set-default PORT 8080

    runHook postInstall
  '';

  meta = {
    description = "munibot's pre-built fullstack bundle (bots + gui)";
    mainProgram = pname;
  };
}
