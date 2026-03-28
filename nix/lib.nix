{
  lib,
  pkgs,
}:

rec {
  unitSubmodule = lib.types.submodule {
    options.script = lib.mkOption {
      type = lib.types.lines;
      description = "Shell script body for the unit's run file.";
    };
  };

  validUnitName = name: name != "" && !lib.hasInfix "/" name && !lib.hasInfix "." name;

  mkUnitsDir =
    units:
    assert
      lib.all validUnitName (lib.attrNames units)
      || throw "Invalid revisor unit name(s). Names must not be empty or contain '/' or '.'.";
    pkgs.linkFarm "revisor-units" (
      lib.mapAttrsToList (name: unitCfg: {
        name = "${name}/run";
        path = pkgs.writeShellScript "revisor-run-${name}" unitCfg.script;
      }) units
    );

  mkRevisorArgs =
    {
      package,
      controlSocketPath,
      killOnExit,
      units,
    }:
    [
      "${package}/bin/revisor"
      "--control-socket"
      controlSocketPath
    ]
    ++ lib.optional killOnExit "--kill-on-exit"
    ++ [ "${mkUnitsDir units}" ];

  mkRevisorScript =
    args:
    pkgs.writeShellScript "revisor-start" ''
      mkdir -p "$(dirname ${lib.escapeShellArg args.controlSocketPath})"
      exec ${lib.escapeShellArgs (mkRevisorArgs args)}
    '';

  instanceModule = {
    options = {
      enable = lib.mkEnableOption "revisor";

      package = lib.mkPackageOption pkgs "revisor" { };

      killOnExit = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Kill all managed units when this instance exits.";
      };

      controlSocketPath = lib.mkOption {
        type = lib.types.str;
        description = "Path to the control socket.";
      };

      units = lib.mkOption {
        type = lib.types.attrsOf unitSubmodule;
        default = { };
        description = "Attribute set of units to manage.";
      };
    };
  };

  mkServiceOption =
    {
      defaultControlSocketPath,
      description,
    }:
    lib.mkOption {
      type = lib.types.submodule (
        { ... }:
        {
          imports = [ instanceModule ];
          config.controlSocketPath = lib.mkDefault defaultControlSocketPath;
        }
      );
      default = { };
      inherit description;
    };
}
