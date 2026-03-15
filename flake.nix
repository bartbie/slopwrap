{
  description = "slopwrap - AI tooling isolation wrapper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-parts.url = "github:hercules-ci/flake-parts";
    systems.url = "github:nix-systems/default";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      naersk,
      ...
    }@inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import inputs.systems;
      perSystem =
        {
          system,
          pkgs,
          ...
        }:
        let
          toolchain = pkgs.rust-bin.stable.latest.default.override (p: {
            extensions = p.extensions ++ [ "rust-src" "rust-analyzer" ];
          });

          naersk' = pkgs.callPackage naersk {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        {
          _module.args.pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          packages.default = naersk'.buildPackage {
            src = ./.;
          };

          checks.unit = naersk'.buildPackage {
            src = ./.;
            doCheck = true;
            cargoTestOptions = x: x ++ [ "--" "--test-threads=1" ];
          };

          checks.property-random = naersk'.buildPackage {
            src = ./.;
            doCheck = true;
            cargoTestOptions = x: x ++ [ "prop_" "--" "--test-threads=1" ];
            ARBTEST_BUDGET_MS = "500";
            postCheck = ''
              for i in 2 3 4 5; do
                echo "=== property run $i/5 ==="
                ARBTEST_BUDGET_MS=500 cargo test prop_ -- --test-threads=1
              done
            '';
          };

          # Test binaries built by naersk, to run inside the VM
          packages.test-bins = naersk'.buildPackage {
            src = ./.;
            cargoBuildOptions = x: x ++ [ "--tests" ];
            postInstall = ''
              find target/release/deps -maxdepth 1 -type f -executable ! -name '*.d' \
                -exec cp {} $out/bin/ \;
            '';
          };

          checks.vm-test = pkgs.testers.nixosTest {
            name = "slopwrap-integration";

            nodes.machine = { pkgs, ... }: {
              environment.systemPackages = [
                self.packages.${system}.default
                self.packages.${system}.test-bins
                pkgs.bubblewrap
                pkgs.diffutils
                pkgs.curl
                pkgs.git
              ];
              # bwrap needs user namespaces
              security.unprivilegedUsernsClone = true;
              users.users.testuser = {
                isNormalUser = true;
                home = "/home/testuser";
              };
            };

            testScript = ''
              machine.wait_for_unit("multi-user.target")

              # TMPDIR must be outside /tmp — the sandbox mounts --tmpfs /tmp
              # which hides any repo path under it from the overlay mount.
              machine.succeed("su - testuser -c 'mkdir -p /home/testuser/tmp'")

              # Run the Rust integration tests (#[ignore]d ones that need bwrap).
              # Test binary names have a hash suffix from cargo.
              for test_bin in ["sandbox", "overlay", "exit_code"]:
                  machine.succeed(
                      f"su - testuser -c 'TMPDIR=/home/testuser/tmp"
                      f" $(ls /run/current-system/sw/bin/{test_bin}-* | head -1)"
                      f" --ignored --test-threads=1'",
                      timeout=60,
                  )
            '';
          };

          packages.slopwrap-fuzz = pkgs.writeShellScriptBin "slopwrap-fuzz" ''
            set -uo pipefail
            RUNS=''${1:-10}
            TIMEOUT=''${2:-60}
            FAILED=0
            FAIL_ITERS=""
            echo "Running slopwrap property tests ($RUNS iterations, 5 seeds each)..."
            for i in $(seq 1 "$RUNS"); do
              echo "=== iteration $i/$RUNS ==="
              OUTPUT=$(timeout "$TIMEOUT" nix build .#checks.${system}.property-random --rebuild 2>&1) || {
                if echo "$OUTPUT" | grep -q "not valid, so checking is not possible"; then
                  timeout "$TIMEOUT" nix build .#checks.${system}.property-random 2>&1 || {
                    FAILED=$((FAILED + 1))
                    FAIL_ITERS="$FAIL_ITERS $i"
                  }
                else
                  echo "$OUTPUT"
                  FAILED=$((FAILED + 1))
                  FAIL_ITERS="$FAIL_ITERS $i"
                fi
              }
            done
            if [ "$FAILED" -gt 0 ]; then
              echo "FAILED $FAILED/$RUNS iterations (iterations:$FAIL_ITERS)"
              echo "Reproduce with ARBTEST_SEED=<hex> cargo test <test_name>"
              exit 1
            fi
            echo "All $RUNS iterations passed ($(( RUNS * 5 )) total runs)."
          '';

          devShells.default = pkgs.mkShell {
            nativeBuildInputs = [
              pkgs.pkg-config
              toolchain
            ];
            buildInputs = [
              pkgs.bubblewrap
              pkgs.diffutils
              pkgs.util-linux
              pkgs.iproute2
              pkgs.slirp4netns
            ];
            env.RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          };
        };
    };
}
