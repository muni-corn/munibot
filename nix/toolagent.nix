# Builds munibot_toolagent as a static x86_64-unknown-linux-musl binary,
# suitable for bind-mounting read-only into any sandbox container image
# regardless of what that image knows about munibot (see
# docs/plans/ai/milestone-7-projects.md, commit 204).
#
# Mirrors nix/build.nix's manual cargo-invocation pattern (a plain
# stdenvNoCC.mkDerivation driving cargo directly) rather than
# rustPlatform.buildRustPackage, for the same reason: the workspace's
# Cargo.lock lives at the repository root, not inside this one crate, and
# munibot_toolagent deliberately has zero munibot dependencies to resolve
# against it in isolation.
{
  config,
  pkgs,
  ...
}:
let
  pname = "munibot_toolagent";
  version = (fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;

  # the nightly toolchain with the musl target, as configured by the devenv
  # rust language module (devenv.nix / languages.rust.targets)
  toolchain = config.languages.rust.toolchainPackage;

  target = "x86_64-unknown-linux-musl";

  src = pkgs.lib.cleanSourceWith {
    src = ../.;
    filter =
      path: type:
      (type == "directory")
      || (pkgs.lib.hasInfix "/src/" path)
      || (pkgs.lib.hasSuffix "Cargo.toml" path)
      || (pkgs.lib.hasSuffix "Cargo.lock" path);
  };
in
pkgs.stdenvNoCC.mkDerivation {
  inherit pname version src;

  cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ../Cargo.lock; };

  nativeBuildInputs = [
    toolchain
    pkgs.rustPlatform.cargoSetupHook
  ];

  dontConfigure = true;

  buildPhase = ''
    runHook preBuild

    # nix sandbox sets HOME to /homeless-shelter; move it somewhere writable
    export HOME=$TMPDIR
    export CARGO_HOME=$TMPDIR/cargo-home
    export CARGO_NET_OFFLINE=true

    # the pinned nightly ICEs codegen'ing tokio for this target at
    # opt-level 2/3 -- opt-level 1 avoids it while staying fast enough for
    # a container's own tool agent. See
    # docs/notes/projects-preflight-findings.md.
    export RUSTFLAGS="-C opt-level=1"

    cargo build --release --target ${target} --locked --offline -p ${pname}

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    cp target/${target}/release/${pname} $out/bin/${pname}
    runHook postInstall
  '';

  # a static binary has nothing to patch: autoPatchelf would only fail
  # trying to rewrite an interpreter/rpath that a static-pie binary doesn't
  # have in the first place
  dontPatchELF = true;
  dontAutoPatchelf = true;

  meta = {
    description = "static musl munibot_toolagent, bind-mounted into sandbox containers";
    mainProgram = pname;
  };
}
