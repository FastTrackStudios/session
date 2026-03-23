{
  description = "FastTrackStudio Session — session/project management domain";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        toolchain = pkgs.rust-bin.stable.latest.default;

        nativeBuildInputs = with pkgs; [
          pkg-config
          toolchain
          tailwindcss_4
        ];

        buildInputs = with pkgs; [
          # OpenSSL
          openssl

          # X11 / windowing
          libx11
          libxi
          libxcursor
          libxrandr
          libxcb
          libxkbcommon

          # GPU / Vulkan
          vulkan-loader
          libGL

          # GTK / GLib (Dioxus desktop / WebKitGTK)
          gtk3
          glib
          gdk-pixbuf
          pango
          cairo
          atk

          # Wayland
          wayland

          # Dioxus desktop (webkit2gtk)
          webkitgtk_4_1
          libsoup_3

          # Misc
          xdotool
        ];

        runtimeLibs = with pkgs; [
          vulkan-loader
          libGL
          wayland
          libxkbcommon
          glib
          gtk3
          webkitgtk_4_1
          libsoup_3
          cairo
          pango
          gdk-pixbuf
          atk
          xdotool
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          inherit nativeBuildInputs buildInputs;

          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;

          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";

          shellHook = ''
            echo ""
            echo "  session dev shell"
            echo "  ────────────────────────────────────────"
            echo "  just dx             — serve desktop app"
            echo "  just build          — build everything"
            echo "  just check          — type-check all"
            echo "  just test           — run tests"
            echo ""
          '';
        };
      }
    );
}
