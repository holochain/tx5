{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-parts.url = "flake-parts";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs: inputs.flake-parts.lib.mkFlake { inherit inputs; } {
    systems = [ "aarch64-darwin" "x86_64-linux" "x86_64-darwin" "aarch64-linux" ];

    perSystem = { pkgs, system, ... }: {
      _module.args.pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [ inputs.rust-overlay.overlays.default ];
      };

      formatter = pkgs.nixpkgs-fmt;

      devShells.default = with pkgs; mkShell {
        packages = [
          pkg-config
          git
          (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
          perl
          go
          cmake
          openssl
          clang
          llvmPackages.libclang
        ];

        LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
      };
    };
  };
}
