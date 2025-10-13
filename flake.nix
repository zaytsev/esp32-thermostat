{
  description = "My Rust Project";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # devenv.url = "github:cachix/devenv";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    # devenv,
    flake-utils,
    rust-overlay,
    ...
  }: let
    packageName = "esp32-thermostat";
    perSystemOutputs = flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        crateManifest = builtins.fromTOML (builtins.readFile ./crates/server/Cargo.toml);

        rustPlatform = pkgs.makeRustPlatform {
          cargo = pkgs.rust-bin.stable.latest.minimal;
          rustc = pkgs.rust-bin.stable.latest.minimal;
        };
      in {
        packages = {
          ${packageName} = rustPlatform.buildRustPackage {
            pname = packageName;
            version = crateManifest.package.version;

            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            postInstall = ''
              mv $out/bin/${crateManifest.package.name} $out/bin/${packageName}
            '';

            cargoBuildFlags = [
              "--package"
              "${crateManifest.package.name}"
            ];

            # Build dependencies
            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs = with pkgs; [
              openssl
              duckdb
            ];

            # doCheck = false;
          };

          default = self.packages.${system}.${packageName};
        };
      }
    );
  in
    perSystemOutputs
    // {
      nixosModules = rec {
        esp32-thermostat = {
          config,
          lib,
          pkgs,
          ...
        }:
          with lib; let
            cfg = config.services.esp32-thermostat;
          in {
            options.services.esp32-thermostat = {
              enable = mkEnableOption "ESP32 Thermostat Service";

              package = mkOption {
                type = types.package;
                default = self.packages.${pkgs.system}.${packageName};
                description = "The package to use for esp32-thermostat-service";
              };

              # Secret configuration
              secretFile = mkOption {
                type = types.nullOr types.path;
                default = null;
                description = ''
                  Path to a file containing the SECRET environment variable.
                  This should contain the HMAC key (hex or plain string).
                  If not 32 bytes it will be hashed with SHA-256.
                '';
                example = "/run/secrets/esp32-thermostat-psk";
              };

              secret = mkOption {
                type = types.nullOr types.str;
                default = null;
                description = ''
                  HMAC secret key (hex or plain string).

                  WARNING: This will be stored in the Nix store and is readable by all users.
                  Use secretFile option instead for production deployments.
                '';
              };

              telemetry = {
                host = mkOption {
                  type = types.str;
                  default = "0.0.0.0";
                  description = "Address to bind telemetry endpoint to";
                };

                port = mkOption {
                  type = types.port;
                  default = 40333;
                  description = "Port for telemetry endpoint";
                };

                openFirewall = mkOption {
                  type = types.bool;
                  default = false;
                  description = "Whether to open the firewall for the telemetry port";
                };
              };

              # Web interface configuration
              web = {
                host = mkOption {
                  type = types.str;
                  default = "127.0.0.1";
                  description = "Address to bind web interface to";
                };

                port = mkOption {
                  type = types.port;
                  default = 8080;
                  description = "Port for web interface";
                };

                openFirewall = mkOption {
                  type = types.bool;
                  default = false;
                  description = "Whether to open the firewall for the web port";
                };
              };

              # Database configuration
              database = {
                path = mkOption {
                  type = types.str;
                  default = "/var/lib/esp32-thermostat/metrics.db";
                  description = "Path to the DuckDB database file";
                };
              };

              # State directory
              stateDir = mkOption {
                type = types.str;
                default = "/var/lib/esp32-thermostat";
                description = "Directory to store service data";
              };

              # User/Group configuration
              user = mkOption {
                type = types.str;
                default = "esp32therm";
                description = "User account under which the service runs";
              };

              group = mkOption {
                type = types.str;
                default = "esp32therm";
                description = "Group under which the service runs";
              };

              extraGroups = mkOption {
                type = types.listOf types.str;
                default = [];
                description = "Additional groups the service user should be a member of";
              };

              # Extra environment variables
              extraEnvironment = mkOption {
                type = types.attrsOf types.str;
                default = {};
                description = "Extra environment variables";
                example = {
                  RUST_LOG = "info";
                };
              };
            };

            config = mkIf cfg.enable {
              # Validation
              assertions = [
                {
                  assertion = cfg.secretFile != null || cfg.secret != null;
                  message = "services.esp32-thermostat: either secretFile or secret must be set";
                }
                {
                  assertion = cfg.secret == null || cfg.secretFile == null;
                  message = "services.esp32-thermostat: only one of secretFile or secret should be set";
                }
              ];

              warnings = optional (cfg.secret != null) ''
                services.esp32-thermostat: Using 'secret' option stores the secret in the Nix store.
                Consider using 'secretFile' with sops-nix for production deployments.
              '';

              # Create systemd service
              systemd.services.telemetry-service = {
                description = "ESP32 Thermostat Service";
                wantedBy = ["multi-user.target"];
                after = ["network.target"];

                environment =
                  {
                    TELEMETRY_HOST = cfg.telemetry.host;
                    TELEMETRY_PORT = toString cfg.telemetry.port;
                    WEB_HOST = cfg.web.host;
                    WEB_PORT = toString cfg.web.port;
                    DB_PATH = cfg.database.path;
                  }
                  // (optionalAttrs (cfg.secret != null) {
                    SECRET = cfg.secret;
                  })
                  // cfg.extraEnvironment;

                serviceConfig = {
                  ExecStart = let
                    startScript = pkgs.writeShellScript "${packageName}-start" ''
                      export SECRET="$(< "$CREDENTIALS_DIRECTORY/SECRET")"
                      exec ${cfg.package}/bin/${packageName}
                    '';
                  in
                    if cfg.secretFile != null
                    then "${startScript}"
                    else "${cfg.package}/bin/${packageName}";

                  Restart = "always";
                  RestartSec = "10s";
                  User = cfg.user;
                  Group = cfg.group;
                  WorkingDirectory = cfg.stateDir;
                  StateDirectory = baseNameOf cfg.stateDir;

                  # Load secrets from file if provided
                  LoadCredential =
                    lib.mkIf (cfg.secretFile != null)
                    "SECRET:${cfg.secretFile}";

                  # Security hardening
                  NoNewPrivileges = true;
                  PrivateTmp = true;
                  ProtectSystem = "strict";
                  ProtectHome = true;
                  ReadWritePaths = [cfg.stateDir];
                  ProtectKernelTunables = true;
                  ProtectKernelModules = true;
                  ProtectControlGroups = true;
                  RestrictAddressFamilies = ["AF_UNIX" "AF_INET" "AF_INET6"];
                  RestrictNamespaces = true;
                  LockPersonality = true;
                  RestrictRealtime = true;
                  RestrictSUIDSGID = true;
                  PrivateMounts = true;
                  SystemCallArchitectures = "native";

                  # Capabilities
                  CapabilityBoundingSet = "";
                  AmbientCapabilities = "";

                  # Additional hardening
                  ProtectKernelLogs = true;
                  ProtectClock = true;
                  SystemCallFilter = ["@system-service" "~@privileged" "~@resources"];
                };
              };

              users.users.${cfg.user} = {
                isSystemUser = true;
                group = cfg.group;
                home = cfg.stateDir;
                extraGroups = cfg.extraGroups;
                description = "ESP32 Thermostat Service User";
              };

              users.groups.${cfg.group} = {};

              networking.firewall = mkMerge [
                (mkIf cfg.telemetry.openFirewall {
                  allowedUDPPorts = [cfg.telemetry.port];
                })
                (mkIf cfg.web.openFirewall {
                  allowedTCPPorts = [cfg.web.port];
                })
              ];
            };
          };
        default = esp32-thermostat;
      };
    };
  # }

  # Keep your devenv shell
  # devShells.default = devenv.lib.mkShell {
  #   inherit pkgs;
  #   modules = [
  #     ./devenv.nix
  #   ];
  # };
  # }
}
