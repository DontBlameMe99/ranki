self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.ranki;
  tomlFormat = pkgs.formats.toml {};
  hasConfig = cfg.collection != null || cfg.mediaDir != null || cfg.settings != {};

  icon = pkgs.writeTextDir "ranki.svg" ''
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
      <rect width="64" height="64" rx="12" fill="#5b21b6"/>
      <path d="M14 16h36v8H14zm0 12h22v8H14zm0 12h30v8H14z" fill="#fff"/>
    </svg>
  '';

  validOrNull = predicate: value: value == null || predicate value;
  validSettings = settings: let
    section = name:
      if builtins.isAttrs (settings.${name} or null)
      then settings.${name}
      else {};
    general = section "general";
    ui = section "ui";
    scheduler = section "scheduler";
  in
    validOrNull builtins.isString (general.collection or null)
    && validOrNull builtins.isString (general.media_dir or null)
    && validOrNull (value: builtins.elem value ["dark" "light" "system"]) (
      ui.theme or null
    )
    && validOrNull (value: builtins.isFloat value && value > 0) (ui.font_size or null)
    && validOrNull (value: builtins.isInt value && value >= 1 && value <= 1000) (
      ui.review_batch_size or null
    )
    && validOrNull builtins.isBool (ui.show_keyboard_hints or null)
    && validOrNull (value: builtins.elem value ["fsrs" "sm2"]) (
      scheduler.algorithm or null
    )
    && validOrNull (value: builtins.isFloat value && value > 0 && value < 1) (
      scheduler.desired_retention or null
    );
in {
  options.programs.ranki = {
    enable = lib.mkEnableOption "Ranki";

    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "ranki.packages.<system>.default";
      description = "Ranki package. Null uses an externally installed binary.";
    };

    collection = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/home/user/.local/share/Anki2/main/collection.anki2";
      description = "Collection path written to general.collection.";
    };

    mediaDir = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/home/user/.local/share/Anki2/main/collection.media";
      description = "Media directory written to general.media_dir.";
    };

    settings = lib.mkOption {
      type = lib.types.addCheck tomlFormat.type validSettings;
      default = {};
      example = {
        ui.theme = "system";
        ui.font_size = 16.0;
        ui.review_batch_size = 20;
        ui.show_keyboard_hints = true;
        scheduler.algorithm = "fsrs";
        scheduler.desired_retention = 0.9;
      };
      description = ''
        Freeform TOML attribute set written to ~/.config/ranki/config.toml.
        Recognized sections are general, ui, and scheduler. Values in settings
        override collection and mediaDir when the same key is present.
      '';
    };

    desktopEntry = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Install a freedesktop desktop entry for Ranki.";
    };

    forceConfig = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Replace an existing unmanaged config.toml at activation.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = lib.mkIf (cfg.package != null) [cfg.package];

    xdg.configFile."ranki/config.toml" = lib.mkIf hasConfig {
      source = tomlFormat.generate "ranki-config.toml" (
        lib.filterAttrsRecursive (_: v: v != null) (
          lib.recursiveUpdate {
            general.collection = cfg.collection;
            general.media_dir = cfg.mediaDir;
          }
          cfg.settings
        )
      );
      force = cfg.forceConfig;
    };

    xdg.desktopEntries.ranki = lib.mkIf cfg.desktopEntry {
      name = "Ranki";
      exec =
        if cfg.package == null
        then "ranki gui"
        else "${cfg.package}/bin/ranki gui";
      comment = "Anki-compatible flashcards";
      terminal = false;
      categories = ["Education"];
      icon = "${icon}/ranki.svg";
    };
  };
}
