{ pkgs, ... }:
{
  projectRootFile = "flake.nix";
  programs = {
    dprint = {
      enable = true;
      includes = [
        "*.toml"
        "*.md"
        "*.scss"
        "*.html"
      ];
      excludes = [ "Cargo.lock" ];
      settings = {
        plugins =
          with pkgs;
          dprint-plugins.getPluginList (
            plugins: with dprint-plugins; [
              dprint-plugin-toml
              dprint-plugin-markdown
              g-plane-malva
              g-plane-markup_fmt
            ]
          );

        indentWidth = 2;
        useTabs = false;

        markdown = {
          textWrap = "always";
          emphasisKind = "asterisks";
        };

        malva = {
          hexColorLength = "short";
          quotes = "preferDouble";
          formatComments = true;
          declarationOrder = "smacss";
          singleLineBlockThreshold = 1;
          keyframeSelectorNotation = "keyword";
          preferSingleLine = true;
        };
      };
    };
    leptosfmt.enable = true;
    nixfmt.enable = true;
    rustfmt.enable = true;
  };

  settings.formatter.leptosfmt.options = [ "--experimental-tailwind" ];
}
