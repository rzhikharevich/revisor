{
  config,
  lib,
  options,
  pkgs,
  ...
}:

let
  revisorLib = import ./lib.nix { inherit lib pkgs; };
  cfg = config.services.revisor;

  revisorScript = revisorLib.mkRevisorScript {
    inherit (cfg)
      package
      controlSocketPath
      killOnExit
      units
      ;
  };
in
{
  options.services.revisor = revisorLib.mkServiceOption {
    defaultControlSocketPath = "${config.xdg.stateHome}/revisor/control.sock";
    description = "revisor user service configuration.";
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      (lib.mkIf (options ? launchd) {
        launchd.agents.revisor = {
          enable = true;
          config = {
            Program = revisorScript;
            KeepAlive = true;
            RunAtLoad = true;
          };
        };
      })
      (lib.mkIf (options ? systemd) {
        systemd.user.services.revisor = {
          Unit.Description = "revisor daemon supervisor";
          Service = {
            ExecStart = revisorScript;
            Restart = "on-failure";
          };
          Install.WantedBy = [ "default.target" ];
        };
      })
    ]
  );
}
