{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      mkModuleWrapper =
        path:
        { pkgs, lib, ... }:
        {
          imports = [ path ];
          services.revisor.package = lib.mkDefault self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "revisor";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
          };
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
          };
        }
      );

      nixosModules.default = mkModuleWrapper ./nix/nixos.nix;
      darwinModules.default = mkModuleWrapper ./nix/darwin.nix;
      homeManagerModules.default = mkModuleWrapper ./nix/home.nix;
    };
}
