{
  description = "bli: run tickets as parallel lanes of agent work";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        bli = pkgs.rustPlatform.buildRustPackage {
          pname = "bli";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # The command-layer tests run real git against a temporary repo.
          nativeCheckInputs = [ pkgs.git ];
        };
      in
      {
        packages.default = bli;
        packages.bli = bli;
        apps.default = flake-utils.lib.mkApp { drv = bli; };
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer ];
        };
      });
}
