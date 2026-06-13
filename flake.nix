{
  description = "Task — spec-driven task management";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Dedicated, current-unstable nixpkgs used ONLY to source the
    # `dioxus-cli` (dx) binary at the version the workspace Cargo.lock
    # pins (0.7.9). The main `nixpkgs` above is locked to an older rev
    # whose `dioxus-cli` is 0.7.3 — which `dx build` rejects against a
    # 0.7.9 dioxus lock ("dx and dioxus versions are incompatible"). We
    # don't bump the main pin (it would ripple through crane's toolchain
    # + the wasm-bindgen pins); we just take dx from here.
    nixpkgs-dx.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane.url = "github:ipetkov/crane";

    # Shared Dioxus toolchain (web/desktop/mobile/native). Its default
    # dev shell is reused below as our `devShells.default` so capn
    # pre-push and any other workspace-wide build sees the full Linux
    # GTK + WebView dep list out of the box. Pointed at the local
    # checkout temporarily so dx CLI bumps land without a fork push
    # round-trip. Switch back to `github:FastTrackStudios/Dioxus-Flake`
    # once those changes are upstreamed.
    dioxus-flake = {
      url = "path:/home/cody/Development/Dioxus/dioxus-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
      inputs.crane.follows = "crane";
    };

  };

  outputs = inputs@{ flake-parts, nixpkgs, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      # ── NixOS module (for deploying task-server) ──────────────────
      flake = {
        nixosModules.default = ./nix/module.nix;
        nixosModules.task-server = ./nix/module.nix;
        nixosModules.task-email-watcher = ./nix/task-email-watcher.nix;
      };

      perSystem = { system, lib, self', ... }:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };

          # dioxus-cli (dx) 0.7.9, sourced from current-unstable nixpkgs —
          # see the nixpkgs-dx input note. Used only by the web bundle build.
          pkgsDx = import inputs.nixpkgs-dx { inherit system; };
          dioxus-cli = pkgsDx.dioxus-cli;

          # ── Rust toolchain ──────────────────────────────────────────────
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

          # ── Source layout ───────────────────────────────────────────────
          # All fork dependencies are pinned by git rev in Cargo.toml, so
          # cargo+crane handle vendoring with no in-flake rewrites.
          taskSrc = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let name = builtins.baseNameOf (toString path); in
              !(builtins.elem name [ "target" "node_modules" ".git" ".beads" "result" ]);
          };

          cargoVendorDir = craneLib.vendorCargoDeps { src = taskSrc; };
          # NOTE: `apps/web` is a member of the root workspace (members =
          # ["apps/*"]), so cargo resolves it against the *root* Cargo.lock
          # — the stale, tracked `apps/web/Cargo.lock` (ancient vox 0.3.1
          # from an unreachable bearcove/vox rev) is ignored by cargo and
          # must not drive vendoring. The web build therefore reuses the
          # root `cargoVendorDir`.

          commonArgs = {
            src = taskSrc;
            inherit cargoVendorDir;
            strictDeps = true;
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [ openssl ];
          };

          # ── wasm-bindgen-cli pinned to match Cargo.lock ────────────────
          wasm-bindgen-cli = pkgs.rustPlatform.buildRustPackage rec {
            pname = "wasm-bindgen-cli";
            version = "0.2.116";
            src = pkgs.fetchCrate {
              inherit pname version;
              hash = "sha256-Pcb5+dEaWK+SQJz73/3Si0kxEy8y8n21t+DETMdvKHc=";
            };
            cargoHash = "sha256-PztHuoWBh+wqOuSFTQxnnddKSSu0S1MBOgKy0szHQpI=";
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = lib.optionals pkgs.stdenv.isLinux [ pkgs.openssl ]
              ++ lib.optionals pkgs.stdenv.isDarwin
                (with pkgs.darwin.apple_sdk.frameworks; [ Security CoreFoundation ]);
            doCheck = false;
          };

          # wasm-bindgen-cli 0.2.122 — matches the workspace Cargo.lock's
          # wasm-bindgen lib version (dx 0.7.9 rejects a mismatch). Built
          # through `pkgsDx` (current-unstable) rather than the main pin:
          # the older pin's `fetchCargoVendor` pulls crates from
          # crates.io's `api/v1/.../download`, which now 403s without a
          # browser-y User-Agent; the newer fetcher uses static.crates.io
          # (the CDN) and works.
          wasm-bindgen-cli-web = pkgsDx.rustPlatform.buildRustPackage rec {
            pname = "wasm-bindgen-cli";
            version = "0.2.122";
            src = pkgsDx.fetchCrate {
              inherit pname version;
              hash = "sha256-vO4RSxi/sMWxmsEs3GuljdMfIRSu75A+Q+c5wgYToRU=";
            };
            cargoHash = "sha256-Inup6vvJSG5ghNyeDPyZbfZo4d0LsMG2OJfStoaeDBs=";
            nativeBuildInputs = [ pkgsDx.pkg-config ];
            buildInputs = lib.optionals pkgsDx.stdenv.isLinux [ pkgsDx.openssl ]
              ++ lib.optionals pkgsDx.stdenv.isDarwin
                (with pkgsDx.darwin.apple_sdk.frameworks; [ Security CoreFoundation ]);
            doCheck = false;
          };

          # ── Packages ────────────────────────────────────────────────────

          task-server = craneLib.buildPackage (commonArgs // {
            pname = "task-server";
            version = "0.1.0";
            cargoExtraArgs = "--package task-server";
            doCheck = false;
          });

          task-cli = craneLib.buildPackage (commonArgs // {
            pname = "task-cli";
            version = "0.1.0";
            cargoExtraArgs = "--package task-cli";
            doCheck = false;
          });

          xtask = craneLib.buildPackage (commonArgs // {
            pname = "xtask";
            cargoExtraArgs = "--package xtask";
            doCheck = false;
          });

          vault-core = craneLib.buildPackage (commonArgs // {
            pname = "vault-core";
            cargoExtraArgs = "--package vault-core";
          });

          cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
            pname = "task-deps";
            version = "0.1.0";
          });

          obsidian-wasm = craneLib.mkCargoDerivation (commonArgs // {
            pname = "obsidian-plugin-wasm";
            inherit cargoArtifacts;
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            buildPhaseCargoCommand = ''
              cargo build --release \
                --package obsidian-plugin \
                --target wasm32-unknown-unknown
            '';
            installPhase = ''
              mkdir -p $out/lib
              cp target/wasm32-unknown-unknown/release/obsidian_plugin.wasm $out/lib/
            '';
            doCheck = false;
          });

          task-webapp = craneLib.buildPackage (commonArgs // {
            pname = "task-webapp";
            version = "0.1.0";
            cargoArtifacts = null;
            cargoExtraArgs = "--manifest-path apps/web/Cargo.toml";
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
              dioxus-cli            # dx 0.7.9 (nixpkgs-dx)
              wasm-bindgen-cli-web  # 0.2.122, matches the lock
            ] ++ (with pkgs; [
              tailwindcss_4
              binaryen
            ]);
            doNotPostBuildInstallCargoBinaries = true;
            # Hermetic dx: the sandbox has no network, so dx must NOT try
            # to download its wasm-bindgen / wasm-opt toolchain. Pre-seed
            # its tool dir with the nix-built wasm-opt (binaryen) and point
            # HOME at a writable dir; the wasm-bindgen-cli on PATH (0.2.122)
            # already matches the lock so dx reuses it instead of fetching.
            buildPhaseCargoCommand = ''
              export HOME="$TMPDIR/dx-home"
              mkdir -p "$HOME/.local/share/.dx/tools/binaryen-129/bin"
              ln -sf "$(command -v wasm-opt)" \
                "$HOME/.local/share/.dx/tools/binaryen-129/bin/wasm-opt"
              tailwindcss -i apps/web/tailwind.css -o apps/web/assets/tailwind.css
              cd apps/web
              dx build --release --platform web
            '';
            # buildPhase ends in `cd apps/web`, but dx writes the bundle to
            # the *workspace-root* target dir. Anchor the copy there
            # explicitly rather than relying on the install cwd.
            installPhaseCommand = ''
              mkdir -p $out/www
              srcdir="$(pwd)"
              case "$srcdir" in */apps/web) srcdir="''${srcdir%/apps/web}";; esac
              cp -R "$srcdir/target/dx/task-app-web/release/web/public/." $out/www/
            '';
            doCheck = false;
          });

          # ── OCI container images (pure Nix, no Docker daemon) ───────────
          #
          # `dockerTools.streamLayeredImage` produces an *executable* that
          # streams a docker-archive tarball to stdout — no daemon, no
          # buildx. CI pipes that straight into skopeo:
          #
          #   $(nix build --print-out-paths .#task-server-image) \
          #     | skopeo copy docker-archive:/dev/stdin docker://<registry>/...
          #
          # Images only build on Linux (dockerTools needs a Linux store
          # path layout); guarded with lib.optionalAttrs below.

          # static-web-server config shared by the web + ui-lab images:
          # serve $root on :8080, SPA-fallback every unknown path to
          # index.html with a 200 (client-side router owns the route).
          mkStaticSite = { name, tag ? "latest", siteRoot }:
            pkgs.dockerTools.streamLayeredImage {
              inherit name tag;
              contents = [ pkgs.static-web-server pkgs.cacert ];
              config = {
                Entrypoint = [ "/bin/static-web-server" ];
                Cmd = [
                  "--host" "0.0.0.0"
                  "--port" "8080"
                  "--root" siteRoot
                  "--page-fallback" "${siteRoot}/index.html"
                  "--log-level" "info"
                ];
                ExposedPorts = { "8080/tcp" = { }; };
                Env = [ "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt" ];
              };
            };

          task-server-image = pkgs.dockerTools.streamLayeredImage {
            name = "task-server";
            tag = "latest";
            # git + curl: the snapshot engine shells out to them; cacert
            # for outbound TLS; coreutils/bash for any shell-out the
            # engine performs. /data is the TASK_DATA_ROOT volume.
            contents = with pkgs; [
              task-server
              git
              curl
              cacert
              bashInteractive
              coreutils
            ];
            extraCommands = ''
              mkdir -p data tmp
            '';
            config = {
              Entrypoint = [ "/bin/task-server" ];
              Env = [
                "TASK_DATA_ROOT=/data"
                "TASK_SERVER_BIND=0.0.0.0:8080"
                "RUST_LOG=info"
                "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                "GIT_SSL_CAINFO=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                "PATH=/bin"
              ];
              ExposedPorts = { "8080/tcp" = { }; };
              Volumes = { "/data" = { }; };
              WorkingDir = "/data";
            };
          };

          task-web-image = mkStaticSite {
            name = "task-web";
            siteRoot = "${task-webapp}/www";
          };

          # ── ui-lab (pnpm + Vite) ───────────────────────────────────────
          # Its own pnpm workspace under ui-lab/ (vendor/* holds the
          # vendored @bearcove/vox-* TS runtime as workspace:* deps). Built
          # with nixpkgs' pnpm support: pnpm.fetchDeps fetches the lockfile
          # closure into a fixed-output derivation, configHook wires the
          # offline store, then `pnpm build` (tsc -b && vite build) → dist/.
          ui-lab = pkgs.stdenv.mkDerivation (finalAttrs: {
            pname = "task-ui-lab";
            version = "0.0.0";
            src = ./ui-lab;
            nativeBuildInputs = [
              pkgs.nodejs_22
              pkgs.pnpm_9
              pkgs.pnpmConfigHook
            ];
            pnpmDeps = pkgs.fetchPnpmDeps {
              inherit (finalAttrs) pname version src;
              fetcherVersion = 2;
              hash = "sha256-JBWJhg81dixFwSc8GZg0yJcSyd38pR08VLcH81KkId4=";
            };
            buildPhase = ''
              runHook preBuild
              pnpm build
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p $out/www
              cp -R dist/* $out/www/
              runHook postInstall
            '';
          });

          task-ui-lab-image = mkStaticSite {
            name = "task-ui-lab";
            siteRoot = "${ui-lab}/www";
          };

        in
        {
          packages = {
            default = task-cli;
            inherit task-server task-cli task-webapp xtask vault-core obsidian-wasm wasm-bindgen-cli;
          } // lib.optionalAttrs pkgs.stdenv.isLinux {
            inherit task-server-image task-web-image ui-lab task-ui-lab-image;
          };

          # ── Checks ──────────────────────────────────────────────────────
          checks = {
            vault-core-tests = craneLib.cargoTest (commonArgs // {
              inherit cargoArtifacts;
              pname = "vault-core-tests";
              version = "0.1.0";
              cargoExtraArgs = "--package vault-core";
            });
            fmt = craneLib.cargoFmt { src = taskSrc; pname = "task-fmt"; version = "0.1.0"; };
          };

          # ── Dev shell ───────────────────────────────────────────────────
          #
          # One shell, everything in it. Layers on top of dioxus-flake's
          # default (full Linux GTK + WebView deps for `apps/desktop` +
          # capn pre-push) plus the wasm C toolchain required by
          # arborium-sysroot when building the editor for the browser.
          #
          # Why the unwrapped clang/llvm-bintools:
          # - `arborium-sysroot`'s build.rs compiles a tiny wasm
          #   sysroot through `cc::Build`. It needs a clang that
          #   targets `wasm32-unknown-unknown` and an `llvm-ar` that
          #   produces wasm archives.
          # - nix's `cc-wrapper` injects host-only hardening flags
          #   (notably `-fzero-call-used-regs=used-gpr`) that clang
          #   rejects for wasm. The *-unwrapped variants skip that
          #   wrapper. Same fix the editor's standalone flake used —
          #   ported verbatim per upstream's note.
          # - The CC_/AR_ env vars pin `cc::Build` to these binaries
          #   for the wasm target only; native builds still go through
          #   the wrapped gcc/clang.
          # The mobile shell stays separate because the Android NDK
          # toolchain it pulls in is heavy and only matters for that
          # surface.
          devShells.default = pkgs.mkShell {
            inputsFrom = [ inputs.dioxus-flake.devShells.${system}.default ];
            packages = with pkgs; [
              llvmPackages.clang-unwrapped
              llvmPackages.bintools-unwrapped
              # `lance-encoding` / `lance-file` (the LanceDB
              # crate stack that backs `wiki-search`'s
              # `vector` feature) invokes `prost-build` to
              # generate Rust from `.proto` schemas. Without
              # `protoc` on PATH the build fails with
              # `NotFound { kind: NotFound, error: "Could not
              # find protoc" }`.
              protobuf
            ];
            shellHook = ''
              export CC_wasm32_unknown_unknown=${pkgs.llvmPackages.clang-unwrapped}/bin/clang
              export AR_wasm32_unknown_unknown=${pkgs.llvmPackages.bintools-unwrapped}/bin/llvm-ar
              export PROTOC=${pkgs.protobuf}/bin/protoc
            '';
          };
          devShells.mobile = inputs.dioxus-flake.devShells.${system}.mobile;

          # ── Playwright shell ───────────────────────────────────────
          #
          # Browser-testing surface used by `features/editor/tests/`
          # (the editor playground spec) and any future
          # `tests/playwright/` suites. Layers nodejs + pnpm + just +
          # the Nix-managed Chromium on top of the default shell so
          # we still get cargo + dx + the wasm C toolchain for
          # `dx serve --package playground --platform web`, which is
          # what `playwright.config.js`'s `webServer` boots.
          #
          # NixOS-specific bit: the binary `npx playwright install`
          # downloads is not patchelf'd for our libs, so we point
          # Playwright at `playwright-driver.browsers` instead and
          # disable its own download path. The JS-side
          # `@playwright/test` version must roughly match the
          # `playwright-driver` rev in nixpkgs — if they drift you'll
          # see "Executable doesn't exist" / "browser version
          # mismatch" errors; bump one to match the other.
          devShells.playwright = pkgs.mkShell {
            inputsFrom = [ self'.devShells.default ];
            packages = with pkgs; [
              nodejs_22
              pnpm
              just
              playwright-driver.browsers
            ];
            shellHook = ''
              export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
              export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=true
              export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
            '';
          };
        };
    };
}
