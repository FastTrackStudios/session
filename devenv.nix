# session's dev environment: the fleet's `rust` module (devenv.yaml) is all
# of it. The crate links nothing native and runs no processes.
{ pkgs, lib, ... }:
{
  packages = [ pkgs.cargo-nextest ];

  # CI (.github/workflows/ci.yml sets FLEET_CI): skip building cargo-rail
  # from source on every uncached run; this repo keeps no change files.
  fleet.rust.cargoRail.enable = builtins.getEnv "FLEET_CI" != "1";
}
