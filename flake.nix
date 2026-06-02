{
  description = "HayateOffice dev environment (gpui Linux build/runtime deps)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
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
              echo "HayateOffice dev shell ready (gpui Vulkan/Wayland/X11 deps)."
              echo "  cargo run -p hayate-app --features gpui_platform/wayland,gpui_platform/x11"
              echo "  Verify Vulkan:  vulkaninfo --summary"
              echo "  Non-NixOS GPU note: if no ICD is found, set e.g."
              echo "    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/nvidia_icd.json"
            '';
          };
        }
      );
    };
}
