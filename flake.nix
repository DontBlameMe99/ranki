{
  description = "Rust development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    flake-utils,
    rust-overlay,
    git-hooks,
    ...
  }: let
    rankiHomeModule = import ./nix/ranki.nix self;
  in
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [(import rust-overlay)];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.minimal.override {
          extensions = [
            "rust-src"
            "rustfmt"
            "clippy"
            "rust-analyzer"
          ];
          targets = [
            "aarch64-linux-android"
            "aarch64-apple-ios"
          ];
        };

        craneLib =
          (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # taffy includes its README in the crate docs, so keep README files in the
        # otherwise cargo-only source filter.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (craneLib.filterCargoSources path type) || (pkgs.lib.hasSuffix "README.md" path);
        };

        guiLibs = pkgs.lib.makeLibraryPath [
          pkgs.wayland
          pkgs.libxkbcommon
          pkgs.vulkan-loader
          pkgs.fontconfig
          pkgs.libglvnd
        ];

        commonArgs = {
          inherit src;
          strictDeps = true;
          pname = binName;
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.libxkbcommon.dev
            pkgs.vulkan-loader
            pkgs.fontconfig
            pkgs.libxcb
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly (
          (builtins.removeAttrs commonArgs ["src"])
          // {
            dummySrc = src;
          }
        );

        package = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [pkgs.makeWrapper];
            postFixup = ''
              install -Dm644 ${./assets/rai.svg} \
                $out/share/icons/hicolor/scalable/apps/ranki.svg
              wrapProgram $out/bin/${binName} \
                --prefix LD_LIBRARY_PATH : "${guiLibs}"
            '';
          }
        );

        mkHook = name: entry: {
          enable = true;
          inherit name entry;
          language = "system";
          pass_filenames = false;
          stages = ["pre-push"];
        };

        hooks = git-hooks.lib.${system}.run {
          src = ./.;

          hooks = {
            alejandra.enable = true;
            deadnix.enable = true;
            flake-checker.enable = true;
            gitlint.enable = true;
            check-merge-conflicts.enable = true;
            forbid-new-submodules.enable = true;
            check-json.enable = true;
            lychee = {
              enable = true;
              stages = ["pre-push"];
            };
            comrak = {
              enable = true;
              entry = "${pkgs.comrak}/bin/comrak --inplace --extension tasklist";
            };
            ripsecrets.enable = true;
            typos = {
              enable = true;
              excludes = ["^vendor/taffy/"];
              settings.config.default.extend-words.PN = "PN";
            };
            check-toml.enable = true;
            check-yaml.enable = true;
            check-executables-have-shebangs.enable = true;
            check-added-large-files.enable = true;
            check-symlinks.enable = true;
            trim-trailing-whitespace.enable = true;
            shellcheck.enable = true;

            woodpecker-cli-lint = {
              enable = true;
              files = "\\.woodpecker/";
            };

            rustfmt = mkHook "rustfmt" "nix build .#checks.${system}.fmt --no-link --print-build-logs";
            clippy = mkHook "clippy" "nix build .#checks.${system}.clippy --no-link --print-build-logs";
            audit = mkHook "audit" "${pkgs.cargo-audit}/bin/cargo-audit audit";
            deny = mkHook "deny" "${pkgs.writeShellScript "cargo-deny-check" ''
              export PATH="${rustToolchain}/bin:${pkgs.cargo-deny}/bin:$PATH"
              exec ${pkgs.cargo-deny}/bin/cargo-deny check
            ''}";
            tarpaulin = mkHook "tarpaulin" "nix build .#checks.${system}.tarpaulin --no-link --print-build-logs";

            cargo-nextest =
              mkHook
              "cargo nextest"
              "nix build .#checks.${system}.nextest --no-link --print-build-logs";
          };
        };
        binCargoPath = ./crates/bin/Cargo.toml;
        cargoToml = builtins.fromTOML (builtins.readFile binCargoPath);
        binName = cargoToml.package.name;
      in {
        formatter = pkgs.alejandra;
        packages.default = package;

        apps.default = {
          type = "app";
          program = "${package}/bin/${binName}";
        };

        checks = {
          pre-commit = hooks;

          build = package;

          clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-features --all-targets -- -D warnings";
            }
          );

          nextest = craneLib.cargoNextest (
            commonArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
            }
          );

          tarpaulin = craneLib.mkCargoDerivation (
            commonArgs
            // {
              inherit src cargoArtifacts;
              pname = "${binName}-tarpaulin";
              buildPhaseCargoCommand = "cargo tarpaulin --exclude-files 'crates/bench/src/*'";
              installPhase = "mkdir -p $out";
              nativeBuildInputs =
                commonArgs.nativeBuildInputs ++ [pkgs.cargo-tarpaulin];
            }
          );

          fmt = craneLib.cargoFmt {
            inherit src;
          };
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};
          inputsFrom = [package];

          packages = with pkgs;
            [
              rustToolchain
              pkg-config
              libxkbcommon.dev
              vulkan-loader
              fontconfig
              clang
              cargo-nextest
              cargo-watch
              cargo-audit
              cargo-deny
              cargo-tarpaulin
              bacon
            ]
            ++ hooks.enabledPackages;

          RUST_BACKTRACE = "1";

          shellHook = ''
            ${hooks.shellHook}
            export LD_LIBRARY_PATH="${guiLibs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          '';
        };
      }
    )
    // {
      homeManagerModules.default = rankiHomeModule;
      homeModules = rec {
        default = rankiHomeModule;
        ranki = default;
      };
    };
}
