{
  description = "slopwrap - AI tooling isolation wrapper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs:
    inputs.flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux"];

      perSystem = {pkgs, ...}: {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            bubblewrap
            diffutils
            util-linux # for unshare, nsenter
            iproute2 # for ip netns
            slirp4netns # userspace networking for selective net access
            shellcheck
          ];
        };
      };
    };
}
