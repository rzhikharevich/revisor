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
    defaultControlSocketPath = "/run/revisor/control.sock";
    description = "revisor daemon supervisor configuration.";
  };

  config = lib.mkIf cfg.enable {
    systemd.services.revisor = {
      description = "revisor daemon supervisor";
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        ExecStart = revisorLib.mkRevisorScript {
          inherit (cfg)
            package
            controlSocketPath
            killOnExit
            units
            ;
        };
        RuntimeDirectory = "revisor";
        Restart = "on-failure";
      };
    };
  };
}
