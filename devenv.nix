# session's dev environment: the fleet's `rust` module (devenv.yaml) is all
# of it. The crate links nothing native and runs no processes.
{ pkgs, ... }:
{
  packages = [ pkgs.cargo-nextest ];
}
