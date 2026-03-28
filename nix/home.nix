{
  config,
  lib,
  pkgs,
  ...
}:

let
  revisorLib = import ./lib.nix { inherit lib pkgs; };
  cfg = config.services.revisor;
  isDarwin = pkgs.stdenv.isDarwin;

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
    if isDarwin then
      {
        launchd.agents.revisor = {
          enable = true;
          config = {
            Program = revisorScript;
            KeepAlive = true;
            RunAtLoad = true;
          };
        };
      }
    else
      {
        systemd.user.services.revisor = {
          Unit.Description = "revisor daemon supervisor";
          Service = {
            ExecStart = revisorScript;
            Restart = "on-failure";
          };
          Install.WantedBy = [ "default.target" ];
        };
      }
  );
}
