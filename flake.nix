{
  description = "FastTrackStudio DAW — cross-platform DAW control framework";

  inputs = {
    # Use nixpkgs-unstable directly for Hydra binary cache hits.
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    devenv.url = "github:cachix/devenv";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    fts-flake.url = "github:FastTrackStudios/fts-flake";
    fts-flake.inputs.nixpkgs.follows = "nixpkgs";
    nix2container.url = "github:nlewo/nix2container";
    nix2container.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = [
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
      "fasttrackstudio.cachix.org-1:r7v7WXBeSZ7m5meL6w0wttnvsOltRvTpXeVNItcy9f4="
    ];
    extra-substituters = [
      "https://devenv.cachix.org"
      "https://fasttrackstudio.cachix.org"
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      flake-utils,
      rust-overlay,
      fts-flake,
      nix2container,
    } @ inputs:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfreePredicate =
            pkg:
            builtins.elem (pkgs.lib.getName pkg) [
              "reaper"
              "reaper-headless"
            ];
        };

        n2c = nix2container.packages.${system}.nix2container;

        # Isolated REAPER config — never touches ~/.config/REAPER
        ftsReaperConfig = "$HOME/.config/FastTrackStudio/Reaper";

        # Get FTS packages for dev (with extensions + plugins) and CI (minimal)
        ftsDev = fts-flake.lib.mkFtsPackages {
          inherit pkgs;
          cfg = fts-flake.presets.dev // {
            reaper.configDir = ftsReaperConfig;
          };
        };
        ftsCi = fts-flake.lib.mkFtsPackages {
          inherit pkgs;
          cfg = fts-flake.presets.ci // {
            reaper.configDir = ftsReaperConfig;
          };
        };
        # ── Shared scripts (used in both dev and CI shells) ───────
        sharedScripts = {
          daw-build.exec = "cargo build --workspace";
          daw-build.description = "Build the entire daw workspace";

          daw-test.exec = "cargo test --workspace";
          daw-test.description = "Run all unit tests";

          daw-smoke.exec = ''
            fts-test bash -c '
              "$FTS_REAPER_EXECUTABLE" -newinst -nosplash -ignoreerrors &
              RPID=$!
              sleep 3
              if kill -0 $RPID 2>/dev/null; then
                echo "REAPER running (PID $RPID) — smoke test passed"
                kill $RPID
              else
                echo "REAPER failed to start"
                exit 1
              fi
            '
          '';
          daw-smoke.description = "Quick REAPER headless smoke test";

          daw-integration.exec = "cargo xtask reaper-test";
          daw-integration.description = "Run REAPER integration tests (headless)";

          daw-ci.exec = ''
            set -e
            echo "=== Unit tests ==="
            cargo test --workspace
            echo ""
            echo "=== Integration tests ==="
            daw-integration
            echo ""
            echo "=== All tests passed ==="
          '';
          daw-ci.description = "Run full test suite (unit + integration)";
        };

        # ── CI container image (for local testing with podman/docker) ──
        ci-image = n2c.buildImage {
          name = "daw-ci";
          tag = "latest";
          copyToRoot = pkgs.buildEnv {
            name = "daw-ci-root";
            paths = with pkgs; [
              # Core tools
              bashInteractive
              coreutils
              gnugrep
              findutils
              procps
              which
              gnused
              gawk
              git
              cacert

              # Build tools
              pkg-config
              openssl
              gcc

              # FTS packages (REAPER + headless runner + FHS sandbox)
              ftsCi.fts-test
              ftsCi.reaper-fhs

              # Rust toolchain
              cargo
              rustc
              rustfmt
            ];
            pathsToLink = [ "/bin" "/lib" "/share" "/etc" ];
          };
          config = {
            Env = [
              "FTS_REAPER_EXECUTABLE=${ftsCi.reaper}/bin/reaper"
              "FTS_REAPER_RESOURCES=${ftsCi.reaper}/opt/REAPER"
              "FTS_REAPER_CONFIG=/root/.config/FastTrackStudio/Reaper"
              "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
              "NIXPKGS_ALLOW_UNFREE=1"
            ];
            WorkingDir = "/workspace";
            Cmd = [ "${pkgs.bashInteractive}/bin/bash" ];
          };
        };
      in
      {
        packages = {
          inherit ci-image;
        };

        devShells = {
          # ── Default dev shell ─────────────────────────────────
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, config, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = [
                    ftsDev.fts-test
                    ftsDev.fts-gui
                    ftsDev.reaper-fhs
                    pkgs.pkg-config
                    pkgs.openssl
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    FTS_REAPER_EXECUTABLE = "${ftsDev.reaper}/bin/reaper";
                    FTS_REAPER_RESOURCES = "${ftsDev.reaper}/opt/REAPER";
                    FTS_REAPER_CONFIG = ftsReaperConfig;
                  };

                  scripts = sharedScripts;

                  # ── Claude Code integration ──────────────────
                  claude.code = {
                    enable = true;
                    commands = {
                      smoke = ''
                        Run the REAPER headless smoke test

                        ```bash
                        daw-smoke
                        ```
                      '';
                      integration = ''
                        Run the REAPER integration test suite

                        ```bash
                        daw-integration
                        ```
                      '';
                      build = ''
                        Build the daw workspace

                        ```bash
                        daw-build
                        ```
                      '';
                      test = ''
                        Run unit tests

                        ```bash
                        daw-test
                        ```
                      '';
                      ci = ''
                        Run the full CI test suite (unit + integration)

                        ```bash
                        daw-ci
                        ```
                      '';
                    };
                  };

                  git-hooks.hooks = {
                    rustfmt.enable = true;
                  };

                  enterShell = ''
                    echo ""
                    echo "  daw dev shell (devenv + fts-flake)"
                    echo "  ────────────────────────────────────────"
                    echo "  daw-build         — cargo build --workspace"
                    echo "  daw-test          — cargo test --workspace"
                    echo "  daw-smoke         — REAPER headless smoke test"
                    echo "  daw-integration   — REAPER integration tests"
                    echo "  daw-ci            — full test suite"
                    echo ""
                    echo "  fts-test [cmd]    — headless FHS env"
                    echo "  fts-gui           — launch REAPER with GUI"
                    echo ""
                    echo "  REAPER: ${ftsDev.reaper}/bin/reaper"
                    echo ""
                  '';
                }
              )
            ];
          };

          # ── CI shell (minimal, no GUI) ────────────────────────
          ci = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = [
                    ftsCi.fts-test
                    ftsCi.reaper-fhs
                    pkgs.pkg-config
                    pkgs.openssl
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    FTS_REAPER_EXECUTABLE = "${ftsCi.reaper}/bin/reaper";
                    FTS_REAPER_RESOURCES = "${ftsCi.reaper}/opt/REAPER";
                    FTS_REAPER_CONFIG = ftsReaperConfig;
                  };

                  scripts = sharedScripts;
                }
              )
            ];
          };
        };
      }
    );
}
