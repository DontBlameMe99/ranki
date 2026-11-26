self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.ranki;
  tomlFormat = pkgs.formats.toml {};
  hasConfig = cfg.settings != {};

  validOrNull = predicate: value: value == null || predicate value;
  # Ranges mirror the Rust config validator (crates/bin/src/config.rs), the source of truth.
  validSettings = settings: let
    section = name:
      if builtins.isAttrs (settings.${name} or null)
      then settings.${name}
      else {};
    ui = section "ui";
    scheduler = section "scheduler";
  in
    validOrNull (value: builtins.elem value ["dark" "light" "system"]) (
      ui.theme or null
    )
    && validOrNull (value: builtins.isFloat value && value >= 8.0 && value <= 40.0) (
      ui.font_size or null
    )
    && validOrNull (value: builtins.isInt value && value >= 1 && value <= 500) (
      ui.review_batch_size or null
    )
    && validOrNull builtins.isBool (ui.show_keyboard_hints or null)
    && validOrNull (value: builtins.isFloat value && value >= 0.5 && value <= 0.999) (
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

    settings = lib.mkOption {
      type = lib.types.addCheck tomlFormat.type validSettings;
      default = {};
      example = {
        ui.theme = "system";
        ui.font_size = 16.0;
        ui.review_batch_size = 20;
        ui.show_keyboard_hints = true;
        scheduler.desired_retention = 0.9;
      };
      description = ''
        Freeform TOML attribute set written to ~/.config/ranki/config.toml.
        Recognized sections are ui, scheduler, and sync.
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
        lib.filterAttrsRecursive (_: v: v != null) cfg.settings
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
      icon = "ranki";
    };
  };
}
