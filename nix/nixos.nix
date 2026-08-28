self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    types
    ;

  toml = pkgs.formats.toml { };
in
{
  options.services.munibot = {
    enable = mkEnableOption "munibot";
    package = mkOption {
      type = lib.types.package;
      description = "The munibot package to use.";
      default = self.packages.${pkgs.stdenv.system}.default;
    };

    environmentFile = mkOption {
      type = types.str;
      description = ''
        Path to the environment file for munibot containing secrets for database, redis, Discord, and Twitch authentication.

        munibot requires the following variables to be set: DISCORD_APPLICATION_ID, DISCORD_CLIENT_SECRET, DISCORD_PUBLIC_KEY, DISCORD_TOKEN, TWITCH_CLIENT_ID, TWITCH_CLIENT_SECRET, and TWITCH_TOKEN. DATABASE_URL and REDIS_URL are also required unless createDatabase/createRedis (respectively) are left at their defaults.

        Note: DATABASE_URL should use unix socket authentication -- e.g. mysql://munibot@localhost/munibot -- since the munibot system user is granted passwordless access via the unix_socket plugin.
      '';
    };

    settings = mkOption {
      type = toml.type;
      description = "Settings for munibot.";
      default = { };
    };

    baseUrl = mkOption {
      type = types.str;
      description = ''
        The public base URL munibot's gui is served at, e.g. "https://munibot.example.com". Used to build OAuth2 redirect URIs -- this must match what's registered with each provider (discord, etc).
      '';
      example = "https://munibot.example.com";
    };

    createDatabase = mkOption {
      type = types.bool;
      description = "Whether to create a local MySQL/MariaDB database automatically.";
      default = true;
    };

    createRedis = mkOption {
      type = types.bool;
      description = "Whether to create a local redis instance automatically, for gui login sessions.";
      default = true;
    };

    user = mkOption {
      type = types.str;
      description = "User account under which munibot runs.";
      default = "munibot";
    };

    group = mkOption {
      type = types.str;
      description = "Group account under which munibot runs.";
      default = "munibot";
    };

    enableAiSandbox = mkOption {
      type = types.bool;
      description = ''
        Whether to enable rootless podman for the ai agent harness's container sandbox (see `docs/plans/ai/milestone-4-sandbox.md`). Required for any persona with a `sandbox` policy other than `forbidden` to actually work; harmless to enable otherwise, since nothing provisions a container unless a persona's own configuration asks for one.

        Also allocates a subuid/subgid range for the munibot user and enables systemd lingering for it, both required for rootless podman to run reliably from a system service rather than an interactive login session.

        One manual, one-time step this module does not automate: the munibot user's own `podman.socket` unit needs enabling once after first deploy, since NixOS's `virtualisation.podman` module ships the unit but does not enable it per-user on its own:
        `sudo -u munibot XDG_RUNTIME_DIR=/run/user/$(id -u munibot) systemctl --user enable --now podman.socket`
      '';
      default = true;
    };
  };

  config =
    let
      cfg = config.services.munibot;
    in
    mkIf cfg.enable {
      # primary MySQL database
      services.mysql = lib.mkIf cfg.createDatabase {
        enable = true;
        ensureDatabases = [ "munibot" ];
        ensureUsers = [
          {
            name = cfg.user;
            ensurePermissions."munibot.*" = "ALL PRIVILEGES";
          }
        ];
      };

      # backs gui login sessions
      services.redis.servers.munibot = lib.mkIf cfg.createRedis {
        enable = true;
        port = 6379;
        bind = "127.0.0.1";
      };

      # the ai sandbox's container runtime - see ai::sandbox and
      # munibot_toolagent. rootless podman needs a real subuid/subgid
      # allocation and a persistent user systemd instance (linger) to run
      # reliably from this system service rather than an interactive login
      virtualisation.podman = lib.mkIf cfg.enableAiSandbox {
        enable = true;
      };

      systemd.services.munibot =
        let
          configFile = toml.generate "munibot.toml" cfg.settings;
          mysqlName = config.systemd.services.mysql.name;
          redisName = "redis-munibot.service";
        in
        {
          enable = true;
          description = "munibot";

          after = [
            "network.target"
          ]
          ++ lib.optional cfg.createDatabase mysqlName
          ++ lib.optional cfg.createRedis redisName;
          requires = lib.optional cfg.createDatabase mysqlName ++ lib.optional cfg.createRedis redisName;

          environment = {
            RUST_LOG = "error,munibot=info";
            MUNIBOT_BASE_URL = cfg.baseUrl;
            DATABASE_URL = lib.mkIf cfg.createDatabase "mysql://${cfg.user}@localhost/munibot?socket=/run/mysqld/mysqld.sock";
            REDIS_URL = lib.mkIf cfg.createRedis "redis://127.0.0.1:6379";
          };

          serviceConfig = {
            EnvironmentFile = cfg.environmentFile;
            ExecStart = "${lib.getExe cfg.package} --config-file ${configFile}";
            PassEnvironment = [
              "DATABASE_URL"
              "REDIS_URL"
              "MUNIBOT_BASE_URL"
              "DISCORD_APPLICATION_ID"
              "DISCORD_CLIENT_SECRET"
              "DISCORD_PUBLIC_KEY"
              "DISCORD_TOKEN"
              "TWITCH_CLIENT_ID"
              "TWITCH_CLIENT_SECRET"
              "TWITCH_TOKEN"
            ];
            Restart = "always";
            RestartSec = 10;
            RestartSteps = 5;
            Type = "exec";
            User = cfg.user;
            Group = cfg.group;
          };
          wantedBy = [ "multi-user.target" ];
        };

      users = {
        groups.${cfg.group} = { };
        users.${cfg.user} = {
          isSystemUser = true;
          name = cfg.user;
          group = cfg.group;
          # rootless podman maps container uids/gids into this range - see
          # cfg.enableAiSandbox's own doc comment for why both this and
          # linger are needed for a system service rather than a login session
          subUidRanges = lib.mkIf cfg.enableAiSandbox [
            {
              count = 65536;
              startUid = 100000;
            }
          ];
          subGidRanges = lib.mkIf cfg.enableAiSandbox [
            {
              count = 65536;
              startGid = 100000;
            }
          ];
        };
      };

      # starts (and keeps running) this user's own systemd instance at boot,
      # which is what actually owns the rootless podman socket - without
      # this, podman would only work for the duration of an interactive
      # login this system service never has
      users.users.${cfg.user}.linger = lib.mkIf cfg.enableAiSandbox true;
    };
}
