{
  inputs = {
    nixpkgs.follows = "holonix/nixpkgs";
    holonix.url = "github:holochain/holochain";
  };

  outputs = inputs @ { holonix, ... }:
    holonix.inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      # provide a dev shell for all systems that the holonix flake supports
      systems = builtins.attrNames holonix.devShells;

      perSystem =
        { config
        , system
        , pkgs
        , inputs'
        , ...
        }: {
          devShells.default = pkgs.mkShell {
            inputsFrom = [ holonix.devShells.${system}.coreDev ];
            packages = [
              inputs'.holonix.packages.cargo-rdme
            ];
          };
        };
    };
}
