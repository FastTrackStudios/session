{
  description = "FastTrackStudio DAW — cross-platform DAW control framework";

  inputs = {
    # Use nixpkgs-unstable directly for Hydra binary cache hits.
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
    fts-flake.url = "github:FastTrackStudios/fts-flake";
    fts-flake.inputs.nixpkgs.follows = "nixpkgs";
    # Shared FTS repo-hygiene hub: pinned capn/tracey + the cargo xtask CI
    # battery. We pull its bearcove tool packages into the dev shell so daw
    # runs the same gate as every other repo; daw keeps its own rust-overlay
    # toolchain + REAPER/GPU/audio stack below.
    fts-repo.url = "git+https://git.starcommand.live/FastTrackStudios/fts-repo";
    fts-repo.inputs.nixpkgs.follows = "nixpkgs";
    nix2container.url = "github:nlewo/nix2container";
    nix2container.inputs.nixpkgs.follows = "nixpkgs";
    wdl = {
      url = "github:justinfrankel/WDL";
      flake = false;
    };
  };

  nixConfig = {
    extra-trusted-public-keys = [
      "fasttrackstudio.cachix.org-1:r7v7WXBeSZ7m5meL6w0wttnvsOltRvTpXeVNItcy9f4="
    ];
    extra-substituters = [
      "https://fasttrackstudio.cachix.org"
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
      fts-flake,
      fts-repo,
      nix2container,
      wdl,
    } @ inputs:
    flake-utils.lib.eachSystem
      [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ]
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
            config.allowUnfreePredicate =
              pkg:
              builtins.elem (pkgs.lib.getName pkg) [
                "reaper"
                "reaper-headless"
              ];
          };

          isLinux = pkgs.stdenv.hostPlatform.isLinux;
          rustToolchain = pkgs.rust-bin.stable.latest.default;

          # ── Crane (release packages) ──────────────────────────────
          craneLib = crane.mkLib pkgs;

          # Filter source to only Rust-relevant files (+ .cargo config)
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              (craneLib.filterCargoSources path type)
              || (builtins.match ".*\.cargo/config\.toml" path != null);
          };

          # Vendor cargo deps with WDL submodule injected into reaper-rs
          cargoVendorDir = craneLib.vendorCargoDeps {
            inherit src;
            overrideVendorGitCheckout = ps: drv:
              if pkgs.lib.any (p: pkgs.lib.hasInfix "reaper-rs" (p.source or "")) ps
              then drv.overrideAttrs (old: {
                postInstall = (old.postInstall or "") + ''
                  for d in $out/reaper-low-*/; do
                    mkdir -p "$d/lib/WDL"
                    cp -r ${wdl}/* "$d/lib/WDL/"
                  done
                '';
              })
              else drv;
          };

          # Common args shared across all derivations
          commonArgs = {
            inherit src cargoVendorDir;
            pname = "daw";
            version = "0.1.0";
            strictDeps = true;
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs =
              with pkgs;
              [ openssl ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
                pkgs.apple-sdk_15
              ];
          };

          # Build workspace deps once (cached across builds)
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          # Build the entire workspace (bins + cdylib)
          daw-workspace = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              # Don't run tests in the package build — CI handles that
              doCheck = false;
              # Install both binaries and the cdylib
              postInstall = ''
                # Copy the REAPER plugin cdylib
                mkdir -p $out/lib
                find target -name "libreaper_daw_bridge.so" -o -name "libreaper_daw_bridge.dylib" \
                  | head -1 | xargs -I{} cp {} $out/lib/ 2>/dev/null || true
              '';
            }
          );

          # ── FTS / REAPER packages (Linux only) ────────────────────────
          # These reference REAPER (unfree) and nix2container, which are
          # only available on Linux. Nix laziness means they won't be
          # evaluated on Darwin since they're only used in optionalAttrs.
          n2c = nix2container.packages.${system}.nix2container;

          # Isolated REAPER config — never touches ~/.config/REAPER
          ftsReaperConfig = "$HOME/.config/FastTrackStudio/Reaper";

          # reaper-flake (formerly fts-flake) renamed mkFtsPackages to
          # mkReaperPackages and split the GUI / headless builds in
          # https://github.com/FastTrackStudios/reaper-flake (commit
          # `aee333d`). Old consumers expected `fts-test` (FHS-sandboxed
          # headless launcher) and `fts-gui` (FHS-sandboxed GUI launcher)
          # binary names — compat shims below preserve those so the
          # existing runner.rs / devshell hooks keep working.
          mkReaper = preset: extraCfg:
            let
              base = fts-flake.lib.mkReaperPackages {
                inherit pkgs;
                cfg = preset // extraCfg;
              };
            in base // {
              # Generic FHS-env launcher: takes a binary path + args.
              # Equivalent to the old `fts-test`. Used by reaper-test's
              # spawn_reaper to run the headless REAPER under bwrap.
              fts-test = pkgs.writeShellScriptBin "fts-test" ''
                exec ${base.reaper-fhs}/bin/reaper-env "$@"
              '';
              # Visible-GUI launcher (renamed from old `fts-gui`). Wraps
              # `reaper-wrapped` so the libSwell GUI binary opens a real
              # window — needed for `just gui-test` to actually show
              # REAPER.
              fts-gui = pkgs.writeShellScriptBin "fts-gui" ''
                # GUI REAPER must use the host OpenGL stack. The dev shell
                # intentionally defaults to lavapipe for headless GPU tests,
                # but WebKitGTK embedded in REAPER needs real GL/EGL.
                unset LIBGL_ALWAYS_SOFTWARE
                unset VK_ICD_FILENAMES
                export LD_LIBRARY_PATH="/run/opengl-driver/lib:/run/opengl-driver-32/lib:''${LD_LIBRARY_PATH:-}"
                export LIBGL_DRIVERS_PATH="/run/opengl-driver/lib/dri"
                export GBM_BACKENDS_PATH="/run/opengl-driver/lib/gbm"
                export __EGL_VENDOR_LIBRARY_DIRS="/run/opengl-driver/share/glvnd/egl_vendor.d"
                exec ${base.reaper-wrapped}/bin/reaper "$@"
              '';
            };

          ftsDev = mkReaper fts-flake.presets.dev {
            reaper.configDir = ftsReaperConfig;
          };
          ftsCi = mkReaper fts-flake.presets.ci {
            reaper.configDir = ftsReaperConfig;
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
          # Build just the daw-bridge cdylib (REAPER extension plugin)
          daw-bridge = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "daw-bridge";
              cargoExtraArgs = "-p daw-bridge";
              doCheck = false;
              # cdylib-only crates don't produce binaries, so crane's default
              # install phase would fail. We just need the shared library.
              installPhaseCommand = ''
                mkdir -p $out/lib
                find target -name "libreaper_daw_bridge.so" -o -name "libreaper_daw_bridge.dylib" \
                  | head -1 | xargs -I{} cp {} $out/lib/
              '';
            }
          );

          # Build just the reaper-launcher binary
          reaper-launcher = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "reaper-launcher";
              cargoExtraArgs = "-p reaper-launcher";
              doCheck = false;
            }
          );
      in
      {
        packages =
          {
            default = daw-workspace;
            daw = daw-workspace;
            daw-bridge = daw-bridge;
            reaper-launcher = reaper-launcher;
          }
          // pkgs.lib.optionalAttrs isLinux {
            ci-image = ci-image;
          };

        devShells = pkgs.lib.optionalAttrs isLinux (
          let
            # Convert sharedScripts to writeShellScriptBin derivations
            mkScript = name: def: pkgs.writeShellScriptBin name def.exec;
            scriptPackages = pkgs.lib.mapAttrsToList mkScript sharedScripts;

            commonPackages = scriptPackages ++ [
              rustToolchain
              pkgs.cargo-nextest

              # ── Shared FTS hygiene tooling (from fts-repo) ──────────
              # Same pinned versions every FTS repo uses, so `cargo xtask ci`
              # behaves identically. daw keeps its own rust toolchain above
              # (NOT fts-repo's rustup) to avoid a PATH clash.
              fts-repo.packages.${system}.capn
              fts-repo.packages.${system}.tracey
              pkgs.git-cliff # CHANGELOG.md from conventional commits
              pkgs.just # thin task runner → cargo xtask
              pkgs.cargo-shear # unused-dependency detection (capn pre-push)

              pkgs.pkg-config
              pkgs.openssl

              # GPU / Dioxus-native rendering (daw-reaper-embed, daw-reaper-dioxus)
              pkgs.fontconfig
              pkgs.freetype

              # Dioxus desktop / Wry WebKitGTK embedding
              pkgs.gtk3
              pkgs.webkitgtk_4_1
              pkgs.webkitgtk_4_1.dev
              pkgs.libsoup_3
              pkgs.libsoup_3.dev
              pkgs.glib
              pkgs.gdk-pixbuf
              pkgs.pango
              pkgs.cairo
              pkgs.atk

              # X11 child-window embedding for Wry build_as_child on Linux
              pkgs.libx11
              pkgs.libxext
              pkgs.libxrender
              pkgs.libxfixes
              pkgs.libxcomposite
              pkgs.libxdamage
              pkgs.libxi
              pkgs.libxrandr
              pkgs.libxcursor
              pkgs.libxinerama
              pkgs.libxtst
              pkgs.libxcb
              pkgs.libxscrnsaver
              pkgs.libxkbcommon
              # libxdo — required by tray-icon / global-hotkey via wry's
              # dioxus-desktop dep tree on Linux.
              pkgs.xdotool

              # C/C++ bindgen (avahi-sys via sync, blitz transitive)
              pkgs.llvmPackages.libclang

              # Plugin-hosting runtime deps (VST3/CLAP). Also listed in
              # LD_LIBRARY_PATH below so dlopen finds them at runtime.
              pkgs.curl
              pkgs.xcbutil
              pkgs.xcbutilcursor
              pkgs.xcbutilkeysyms
              pkgs.xcbutilimage
              pkgs.xcbutilrenderutil
              pkgs.xcbutilwm

              # Misc system libs commonly required by GUI/audio crates
              pkgs.alsa-lib
              pkgs.dbus
              pkgs.zlib
              pkgs.stdenv.cc.cc.lib

              # Headless GPU (mesa software rasterizer + Vulkan loader).
              # Enables wgpu offscreen rendering on CI runners with no
              # discrete GPU. Used by daw-reaper-dioxus snapshot tests
              # when FTS_GPU_TESTS=1.
              pkgs.mesa
              pkgs.libGL
              pkgs.vulkan-loader
            ];

            commonShellEnv = {
              LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
              # Force software-rasterized OpenGL (lavapipe) so wgpu can
              # acquire a GPU adapter in headless / CI environments.
              # Tests still need to opt in via FTS_GPU_TESTS=1.
              LIBGL_ALWAYS_SOFTWARE = "1";
              # Point Vulkan-backed wgpu at lavapipe.
              VK_ICD_FILENAMES = "${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.x86_64.json";
              LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
                pkgs.gtk3
                pkgs.webkitgtk_4_1
                pkgs.libsoup_3
                pkgs.glib
                pkgs.gdk-pixbuf
                pkgs.pango
                pkgs.cairo
                pkgs.atk
                pkgs.alsa-lib
                pkgs.libx11
                pkgs.libxext
                pkgs.libxrender
                pkgs.libxfixes
                pkgs.libxcomposite
                pkgs.libxdamage
                pkgs.libxi
                pkgs.libxrandr
                pkgs.libxcursor
                pkgs.libxinerama
                pkgs.libxtst
                pkgs.libxcb
                pkgs.libxscrnsaver
                pkgs.libxkbcommon
                pkgs.mesa
                pkgs.libGL
                pkgs.vulkan-loader

                # Runtime deps for hosting third-party VST3/CLAP plugins
                # (`daw-standalone --features vst3-host,clap-host`).
                # Most commercial Linux plugin .so's link against the
                # system C++ runtime + freetype/fontconfig/curl/xcb-util
                # for their built-in GUIs. Without these on
                # LD_LIBRARY_PATH the bundle dlopens fail before
                # `GetPluginFactory` is even resolved.
                pkgs.stdenv.cc.cc.lib  # libstdc++.so.6
                pkgs.freetype
                pkgs.fontconfig.lib
                pkgs.curl.out
                pkgs.xcbutil
                pkgs.xcbutilcursor
                pkgs.xcbutilkeysyms
                pkgs.xcbutilimage
                pkgs.xcbutilrenderutil
                pkgs.xcbutilwm
              ];
            };

            commonEnv = {
              FTS_REAPER_CONFIG = ftsReaperConfig;
            };
          in
          {
            # ── Default dev shell ─────────────────────────────────
            default = pkgs.mkShell (commonEnv // commonShellEnv // {
              packages = commonPackages ++ [
                ftsDev.fts-test
                ftsDev.fts-gui
                ftsDev.reaper-fhs
              ];

              FTS_REAPER_EXECUTABLE = "${ftsDev.reaper}/bin/reaper";
              FTS_REAPER_RESOURCES = "${ftsDev.reaper}/opt/REAPER";

              shellHook = ''
                echo ""
                echo "  daw dev shell (fts-flake)"
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
                echo "  cargo xtask ci    — shared fts-repo gate (fmt/clippy/check/nextest/doctests)"
                echo "  cargo xtask check — cargo check --workspace --all-targets"
                echo "  capn / tracey     — pre-commit/pre-push gates + spec traceability"
                echo ""
                echo "  REAPER: ${ftsDev.reaper}/bin/reaper"
                echo ""
              '';
            });

            # ── CI shell — `cargo xtask ci` only ──────────────────
            # The shared fts-repo gate (fmt/clippy/check/nextest/doctests)
            # needs daw's build/system deps to COMPILE the workspace, but
            # NOT REAPER. Keeping the unfree REAPER + FHS launcher out of
            # this shell makes CI faster and avoids an unfree fetch.
            ci = pkgs.mkShell (commonEnv // commonShellEnv // {
              packages = commonPackages;
            });

            # ── REAPER-CI shell — `cargo xtask reaper-test` ───────
            # Heavier shell that adds the headless REAPER + FHS sandbox for
            # the integration-test workflow. Separate so the fast `ci` gate
            # doesn't pay for it on every push.
            reaper-ci = pkgs.mkShell (commonEnv // commonShellEnv // {
              packages = commonPackages ++ [
                ftsCi.fts-test
                ftsCi.reaper-fhs
              ];

              FTS_REAPER_EXECUTABLE = "${ftsCi.reaper}/bin/reaper";
              FTS_REAPER_RESOURCES = "${ftsCi.reaper}/opt/REAPER";
            });
          }
        );
      }
      );
}
