# Ranki Nix and Home Manager research

Research only. No source or flake files were changed. Verified with Nix 2.35.6 on this system.

## System snapshot

```text
nix --version                         -> nix (Nix) 2.35.6
nix config show experimental-features -> fetch-tree flakes nix-command
command -v home-manager               -> not found
~/.config/home-manager, ~/.config/nix -> absent
nix eval --raw nixpkgs#home-manager.version -> 0-unstable-2026-08-06
nix flake metadata                    -> rev 3fa5092..., locked nixpkgs d357a2b...
```

`~/.local/state/home-manager/gcroots/current-home` is an old generation symlink. Leave it alone.
The registry flake `flake:home-manager` resolves to rev `03885bcd0b477bdecebebc57b213b0f8f0826d51`,
with `release.json` set to `26.11`. A test evaluation returned `config.home.version.full = "26.11"`.
That flake has no `lib.version` and no `homeManagerModules` output. The nixpkgs package version
and the registry flake are different snapshots. Make Home Manager's nixpkgs follow the consumer's.

Ranki inputs are `nixpkgs` (nixpkgs-unstable), `crane`, `flake-utils`, `rust-overlay`, and
`git-hooks`. The last two already set `inputs.nixpkgs.follows = "nixpkgs"`. There is no Home
Manager input. Commit `75e3302 fix(nix): make Wayland GUI launch from dev shell and package` is
in HEAD. It defines `guiLibs = pkgs.lib.makeLibraryPath [ pkgs.wayland pkgs.libxkbcommon pkgs.libglvnd ]`,
wraps `$out/bin/ranki` with `--prefix LD_LIBRARY_PATH : "${guiLibs}"`, and exports the same path
from the devShell `shellHook`. Install the wrapped package from `self.packages.<system>.default`.

## App facts that constrain the schema

There is no config file reader in the Rust code. `Cli` has only global `--collection` and
`--media_dir`, both `Option<PathBuf>`. `ranki_ui::Options` has `light_theme: bool`, so
`theme = "system"` needs Rust work. `DUE_BATCH` is 20, the hint bar is always drawn, and the
UI starts on FSRS with `desired_retention = 0.9`. The user can toggle FSRS and SM-2 with `T`.
Cargo has `serde` and `serde_json`, but no direct TOML crate. A reader needs `toml` with serde.

## Proposed config schema

```toml
[general]
collection = "/home/user/.local/share/Anki2/main/collection.anki2"
media_dir = "/home/user/.local/share/Anki2/main/collection.media"

[ui]
theme = "system"
font_size = 16.0
review_batch_size = 20
show_keyboard_hints = true

[scheduler]
algorithm = "fsrs"
desired_retention = 0.9
```

The schema matches current defaults. Keep paths as strings, since a Nix path literal in a
generated config can be copied into the store. Validate `theme` as `dark | light | system`,
`algorithm` as `fsrs | sm2`, `font_size` as a positive float, `review_batch_size` as an
integer from 1 to 1000, `desired_retention` as a float in `(0, 1)`, and
`show_keyboard_hints` as a bool. Scheduler values are initial values, since the user can
change the scheduler during a session.

## Flake patch sketch

```nix
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # crane, flake-utils, rust-overlay, git-hooks stay as they are
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    let
      rankiHomeModule = import ./modules/ranki.nix;
    in
    flake-utils.lib.eachDefaultSystem (
      system: let
        # existing let bindings, including the wrapped package
      in {
        # existing outputs
      }
    )
    // {
      homeManagerModules.default = rankiHomeModule;
      homeModules = rec {
        default = rankiHomeModule;
        ranki = default;
      };
    };
```

A module export is a plain Nix module function, so both paths work. The `home-manager` input
is needed only if Ranki also exposes a test `homeConfigurations` output. Consumers import
`inputs.ranki.homeModules.ranki` and pass `extraSpecialArgs = { ranki = inputs.ranki; }`.

## Home Manager module sketch

Save as `modules/ranki.nix`. This exact module built an activation package on this machine.
The `ranki` argument must come from `extraSpecialArgs`, not `_module.args`.

```nix
{ config, lib, pkgs, ranki, ... }:
let
  cfg = config.programs.ranki;
  tomlFormat = pkgs.formats.toml { };
  hasConfig =
    cfg.collection != null || cfg.mediaDir != null || cfg.settings != { };
in {
  options.programs.ranki = {
    enable = lib.mkEnableOption "Ranki";
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = ranki.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "ranki.packages.<system>.default";
      description = "Ranki package. Null uses an externally installed binary.";
    };
    collection = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/home/user/.local/share/Anki2/main/collection.anki2";
    };
    mediaDir = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/home/user/.local/share/Anki2/main/collection.media";
    };
    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        ui.theme = "system";
        ui.font_size = 16.0;
        ui.review_batch_size = 20;
        ui.show_keyboard_hints = true;
        scheduler.algorithm = "fsrs";
        scheduler.desired_retention = 0.9;
      };
    };
    desktopEntry = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Install an XDG desktop entry.";
    };
    forceConfig = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Replace an existing unmanaged config.toml at activation.";
    };
  };
  config = lib.mkIf cfg.enable {
    home.packages = lib.mkIf (cfg.package != null) [ cfg.package ];
    xdg.configFile."ranki/config.toml" = lib.mkIf hasConfig {
      source = tomlFormat.generate "ranki-config.toml" (
        lib.filterAttrsRecursive (_: v: v != null) (
          lib.recursiveUpdate {
            general.collection = cfg.collection;
            general.media_dir = cfg.mediaDir;
          } cfg.settings
        )
      );
      force = cfg.forceConfig;
    };
    xdg.desktopEntries.ranki = lib.mkIf cfg.desktopEntry {
      name = "Ranki";
      exec = if cfg.package == null then "ranki gui" else "${lib.getExe cfg.package} gui";
      comment = "Anki-compatible flashcards";
      terminal = false;
      categories = [ "Education" ];
    };
  };
}
```

Why these choices:

- `lib.mkPackageOption pkgs "ranki"` throws because Ranki is not in nixpkgs. Use a nullable
  `types.package` with Ranki's own package.
- `pkgs.formats.toml` cannot serialize `null`. `filterAttrsRecursive` prevents the observed
  `unsupported unit type` build failure.
- `recursiveUpdate` lets `settings` override `collection` and `mediaDir`. Use
  `types.ints.between 1 1000` and `types.addCheck types.float (v: v > 0.0 && v < 1.0)` here.
- No config file is generated for empty settings, following Home Manager guidelines.
- Unmanaged files are not replaced during activation unless `force = true`, which deletes them.
- Home Manager 26.11 makes `xdg.desktopEntries.*` a package from `pkgs.makeDesktopItem`.
  Older versions wrote files under XDG data directories.

## Config and CLI merge rules

Load order: built-in defaults, then `~/.config/ranki/config.toml`, then CLI flags.
Use `$XDG_CONFIG_HOME/ranki/config.toml` when that variable is set. A missing file is normal.
Malformed TOML should produce a clear error.

Keep `--collection` and `--media_dir` global as `Option<PathBuf>`. Only override the loaded
config when clap returns `Some(path)`. Do not add `default_value_t` to new UI and scheduler
flags, since that masks config values. Add `Option<T>` flags such as `--theme`, `--font-size`,
`--review-batch-size`, `--show-keyboard-hints`, `--algorithm`, and `--desired-retention`, and
apply them after loading config. The runtime `T` toggle still wins inside a session.

## Verification commands

Nix only sees files tracked by Git inside a repository. Use intent-to-add for the scratch
flake. Never run the activation script.

```sh
cd "$(git rev-parse --show-toplevel)"
mkdir -p .tmp/hm-test
# Copy the module sketch above to .tmp/hm-test/ranki.nix.
# Write .tmp/hm-test/flake.nix from the next block.
git add -N .tmp/hm-test/flake.nix .tmp/hm-test/ranki.nix
cd .tmp/hm-test
nix build --no-link --print-build-logs \
  .#homeConfigurations.ranki-test.activationPackage
TOML=$(nix eval --raw \
  '.#homeConfigurations.ranki-test.config.xdg.configFile."ranki/config.toml".source')
cat "$TOML"
nix eval --raw '.#homeConfigurations.ranki-test.config.home.version.full'
cd ../..
git reset -q -- .tmp/hm-test
rm -rf .tmp/hm-test
test ! -e ~/.config/ranki && echo "user config untouched"
```

The scratch flake uses `pkgs.hello` so the check is fast. Replace it with
`inputs.ranki.packages.x86_64-linux.default` once the module is in the repo.

```nix
{
  inputs = {
    nixpkgs.url = "flake:nixpkgs";
    home-manager = {
      url = "flake:home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = { nixpkgs, home-manager, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      ranki = { packages.${system}.default = pkgs.hello; };
    in {
      homeConfigurations.ranki-test = home-manager.lib.homeManagerConfiguration {
        inherit pkgs;
        extraSpecialArgs = { inherit ranki; };
        modules = [
          {
            home.username = "user";
            home.homeDirectory = "/home/user";
            home.stateVersion = "26.11";
            xdg.enable = true;
            programs.ranki = {
              enable = true;
              collection = "/home/user/.local/share/Anki2/main/collection.anki2";
              mediaDir = "/home/user/.local/share/Anki2/main/collection.media";
              settings = {
                ui = {
                  theme = "system";
                  font_size = 16.0;
                  review_batch_size = 20;
                  show_keyboard_hints = true;
                };
                scheduler = { algorithm = "fsrs"; desired_retention = 0.9; };
              };
            };
          }
          ./ranki.nix
        ];
      };
    };
}
```

Observed result: the activation package built, the generated TOML contained every proposed
key and value, and `~/.config/ranki` stayed absent. Nothing was activated. The `home-manager`
command is absent, so `home-manager switch` is unavailable until it is installed.

## Version and API differences

- The flake-parts convention changed from `homeModules` to `homeManagerModules` in
  February 2025, then reverted to `homeModules` in May 2025. Export both.
- The registry flake has no `homeManagerModules` output. Its flake-parts module defines
  `flake.homeModules` and `flake.homeConfigurations`.
- `homeManagerConfiguration` still takes `pkgs`, `modules`, and optional `extraSpecialArgs`.
  `home.stateVersion = "26.11"` worked in the test.
- Home Manager 26.11 has no `lib.version` flake output. Use `release.json`, flake metadata,
  or `config.home.version.full`.
- `nixpkgs#home-manager.version` is a different snapshot than the registry flake. Keep
  `inputs.nixpkgs.follows = "nixpkgs"` aligned to avoid two module sets.

## Sources

- https://home-manager.dev/manual/unstable/nix-flakes/standalone.html
- https://home-manager.dev/manual/unstable/nix-flakes/flake-parts.html
- https://home-manager.dev/manual/unstable/contributing/guidelines.html
- https://github.com/nix-community/home-manager/pull/6406
- https://nix-community.github.io/home-manager/release-notes/rl-2611.html
