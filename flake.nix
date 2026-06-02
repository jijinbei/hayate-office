{
  description = "HayateOffice dev environment (gpui Linux build/runtime deps)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # nixGL injects the host GPU driver so GPU apps run under Nix on non-NixOS.
    nixgl.url = "github:nix-community/nixGL";
    nixgl.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { self, nixpkgs, nixgl }:
    let
      # Primary test target is Linux (DESIGN: portability-first, main testing on Linux).
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAll (
        system:
        let
          pkgs = import nixpkgs { inherit system; };

          # Libraries gpui needs on Linux. Mirrors zed's nix/build.nix buildInputs subset
          # relevant to gpui (Blade/Vulkan + Wayland/X11 windowing + text).
          runtimeLibs = with pkgs; [
            wayland
            libxkbcommon
            vulkan-loader
            libglvnd
            libgbm
            libdrm
            libva
            xorg.libX11
            xorg.libxcb
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
            xorg.libXext
            xorg.libXfixes
            xorg.libXcomposite
            xorg.libXdamage
            fontconfig
            freetype
            alsa-lib
            openssl
            zlib
            zstd
          ];
        in
        {
          default = pkgs.mkShell {
            # Build tools. Rust itself comes from the user's rustup toolchain on PATH
            # (nix develop is non-pure), so we don't pin it here.
            nativeBuildInputs = with pkgs; [
              pkg-config
              cmake
              clang
              mold
              vulkan-tools # vulkaninfo, for verifying the Vulkan ICD is found
            ];
            buildInputs = runtimeLibs;

            # bindgen (used by some gpui sys-deps) needs libclang.
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

            shellHook = ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibs}:''${LD_LIBRARY_PATH:-}"
              echo "HayateOffice dev shell ready (gpui build deps)."
              echo "  Build:  cargo build -p hayate-app"
              echo "  Run (non-NixOS NVIDIA, injects GPU driver via nixGL):"
              echo "    NIXPKGS_ALLOW_UNFREE=1 nix run --impure .#vulkan-nvidia -- cargo run -p hayate-app"
            '';
          };
        }
      );

      # nixGL wrappers to run the GPU app on non-NixOS. NVIDIA's wrapper is unfree and reads
      # the host driver, so it needs `--impure` and `NIXPKGS_ALLOW_UNFREE=1`. Exposed as a
      # separate output so the default devShell stays pure and never blocks the build.
      #   NIXPKGS_ALLOW_UNFREE=1 nix run --impure .#vulkan-nvidia -- cargo run -p hayate-app
      packages = forAll (
        system:
        let
          ngl = nixgl.packages.${system};
        in
        {
          # Vulkan (Blade backend) for NVIDIA; fall back to the GL wrapper if unavailable.
          vulkan-nvidia = ngl.nixVulkanNvidia or ngl.nixGLNvidia;
          gl-nvidia = ngl.nixGLNvidia;
        }
      );
    };
}
