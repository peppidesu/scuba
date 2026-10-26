{
  description = "Scuba";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, rust-overlay }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSystem = nixpkgs.lib.genAttrs systems;

      # Cranelift is only shipped as a nightly rustup component:
      #   rustup component add rustc-codegen-cranelift-preview --toolchain nightly
      # `selectLatestNightlyWith` picks the newest nightly that actually has
      # every requested component, so a broken nightly won't break your flake.
      craneliftRust =
        pkgs:
        pkgs.rust-bin.selectLatestNightlyWith (
          toolchain:
          toolchain.default.override {
            extensions = [
              "rustc-codegen-cranelift-preview"
              "rust-src"
            ];
          }
        );

      llvmRust = pkgs: pkgs.rust-bin.selectLatestNightlyWith (t: t.default);

      rustPlatformFor =
        pkgs: rust:
        pkgs.makeRustPlatform {
          cargo = rust;
          rustc = rust;
        };
    in
    {
      packages = forEachSystem (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
        in
        {
          # Normal LLVM release build (the default artifact).
          default = (rustPlatformFor pkgs (llvmRust pkgs)).buildRustPackage {
            pname = "my-crate";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
          };

          # Debug build through Cranelift — fast compiles, slower runtime code.
          cranelift = (rustPlatformFor pkgs (craneliftRust pkgs)).buildRustPackage {
            pname = "my-crate";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            buildType = "debug";
            # buildRustPackage honours this attribute and exports it as RUSTFLAGS.
            RUSTFLAGS = "-Zcodegen-backend=cranelift";
          };
        }
      );

      devShells = forEachSystem (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          rust = craneliftRust pkgs;
        in
        {
          default = pkgs.mkShell {
            packages = [
              rust
              pkgs.pkg-config
              pkgs.clang
              pkgs.mold
              pkgs.cargo-watch
            ];

            # Only needed if you don't use the .cargo/config.toml below.
            # RUSTFLAGS = "-Zcodegen-backend=cranelift";
          };
        }
      );

      formatter = forEachSystem (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
        in
        pkgs.nixfmt-rfc-style
      );
    };
}
