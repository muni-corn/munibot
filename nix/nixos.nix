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

  defaultSettings = {
    db = {
      url = "127.0.0.1:7654";
      user = "muni_bot";
    };
  };
in
{
  options.services.munibot = {
    enable = mkEnableOption "munibot";
    package = mkOption {
      type = lib.types.package;
      description = "The munibot package to use.";
      default = self.packages.${pkgs.stdenv.system}.default;
    };
    settings = mkOption {
      type = toml.type;
      description = "Settings for munibot.";
      default = defaultSettings;
    };
    environmentFile = mkOption {
      type = types.str;
      description = ''
        Path to the environment file for munibot containing secrets for database, Discord, and Twitch authentication.

        munibot requires the following variables to be set: DATABASE_URL, DATABASE_PASS, DISCORD_APPLICATION_ID, DISCORD_CLIENT_SECRET, DISCORD_PUBLIC_KEY, DISCORD_TOKEN, TWITCH_CLIENT_ID, TWITCH_CLIENT_SECRET, and TWITCH_TOKEN.

        During the SurrealDB-to-MySQL migration period, SURREAL_URL, SURREAL_USER, and SURREAL_PASS may also be set to override the defaults (ws://localhost:8000, root, root).

        Note: when using the MariaDB service enabled by this module, DATABASE_URL should use unix socket authentication — e.g. mysql://munibot@localhost/munibot — since the munibot system user is granted passwordless access via the unix_socket plugin.
      '';
    };
    createDatabase = mkOption {
      type = types.bool;
      description = "Whether to create a local MySQL/MariaDB database automatically.";
      default = true;
    };
    surrealdb = {
      enable =
        mkEnableOption "SurrealDB during munibot's migration stages. munibot has moved to MySQL/MariaDB, but can automatically migrate data over from a legacy SurrealDB database automatically if specified"
        // {
          # keep enabled until the SurrealDB-to-MySQL migration is complete
          default = true;
        };
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
            ensurePermissions = {
              "munibot.*" = "ALL PRIVILEGES";
            };
          }
        ];
      };

      # SurrealDB migration source; remove once migration to MySQL is complete
      services.surrealdb.enable = mkIf cfg.surrealdb.enable true;

      systemd.services.munibot =
        let
          configFile = toml.generate "munibot.toml" (lib.recursiveUpdate defaultSettings cfg.settings);

          mysqlCfg = config.services.mysql;
          mysqlHost = mysqlCfg.settings.mysqld.bind-address or "localhost";
          mysqlName = config.systemd.services.mysql.name;
          surrealName = config.systemd.services.surrealdb.name;

          surrealdbDeps = lib.optional cfg.surrealdb.enable surrealName;
        in
        {
          enable = true;
          description = "munibot";

          after = [
            "network.target"
            mysqlName
          ]
          ++ surrealdbDeps;
          requires = [ mysqlName ] ++ surrealdbDeps;

          environment = {
            RUST_LOG = "error,munibot=info";
            DATABASE_URL = "mysql://${cfg.user}@${mysqlHost}";
          };

          serviceConfig = {
            EnvironmentFile = cfg.environmentFile;
            ExecStart = "${lib.getExe cfg.package} --config-file ${configFile}";
            PassEnvironment = [
              "DATABASE_URL"
              "DATABASE_PASS"
              "DISCORD_APPLICATION_ID"
              "DISCORD_CLIENT_SECRET"
              "DISCORD_PUBLIC_KEY"
              "DISCORD_TOKEN"
              "TWITCH_CLIENT_ID"
              "TWITCH_CLIENT_SECRET"
              "TWITCH_TOKEN"
              "SURREAL_URL"
              "SURREAL_USER"
              "SURREAL_PASS"
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
        };
      };
    };
}
