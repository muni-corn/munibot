self: {
  config,
  lib,
  pkgs,
  ...
}: let
  inherit (lib) mkEnableOption mkOption types;

  toml = pkgs.formats.toml {};

  defaultSettings = {
    db = {
      url = "127.0.0.1:7654";
      user = "muni_bot";
    };
  };
in {
  options.services.muni_bot = {
    enable = mkEnableOption "muni_bot";
    settings = mkOption {
      type = toml.type;
      description = "Settings for muni_bot.";
      default = defaultSettings;
    };
    environmentFile = mkOption {
      type = types.str;
      description = ''
        Path to the environment file for muni_bot containing secrets for database, Discord, and Twitch authentication.

        muni_bot requires the following variables to be set: DATABASE_PASS, DISCORD_APPLICATION_ID, DISCORD_CLIENT_SECRET, DISCORD_PUBLIC_KEY, DISCORD_TOKEN, TWITCH_CLIENT_ID, TWITCH_CLIENT_SECRET, and TWITCH_TOKEN.
      '';
    };
  };

  config = let
    cfg = config.services.muni_bot;
  in
    lib.mkIf cfg.enable {
      nixpkgs.overlays = [
        self.overlays.default
      ];

      systemd.services.muni_bot = let
        configFile = toml.generate "muni_bot.toml" (lib.recursiveUpdate defaultSettings cfg.settings);
      in {
        enable = true;

        description = "muni_bot";
        serviceConfig = {
          EnvironmentFile = cfg.environmentFile;
          ExecStart = "${pkgs.muni_bot}/bin/muni_bot --config-file ${configFile}";
          Environment = "RUST_LOG=error,muni_bot=info";
          PassEnvironment = [
            "DATABASE_PASS"
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
          User = "muni_bot";
          Group = "muni_bot";
        };
        wantedBy = ["multi-user.target"];
      };

      users = {
        groups.muni_bot = {};
        users.muni_bot = {
          isSystemUser = true;
          name = "muni_bot";
          group = "muni_bot";
        };
      };
    };
}
