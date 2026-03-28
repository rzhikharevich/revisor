{
  config,
  lib,
  pkgs,
  ...
}:

let
  revisorLib = import ./lib.nix { inherit lib pkgs; };
  cfg = config.services.revisor;
in
{
  options.services.revisor = revisorLib.mkServiceOption {
    defaultControlSocketPath = "/var/run/revisor/control.sock";
    description = "System-level revisor daemon configuration.";
  };

  config = lib.mkIf cfg.enable {
    launchd.daemons.revisor = {
      serviceConfig = {
        Program = revisorLib.mkRevisorScript {
          inherit (cfg)
            package
            controlSocketPath
            killOnExit
            units
            ;
        };
        KeepAlive = true;
        RunAtLoad = true;
      };
    };
  };
}
