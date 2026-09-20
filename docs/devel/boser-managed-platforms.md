# Boser-managed platform ownership

Products whose `HardwareCapabilities::boser_managed` flag is set use Boser as the owner of post-setup system
configuration and maintenance. The capability comes from the hardware profile and is fixed for the BMC process lifetime.
Self-managed products retain the normal BMC behavior.

Initial setup and native Wi-Fi reconfiguration remain BMC-owned on every product. Only BMC drives the setup access point
and captive portal. `UpgradeService` is Boser-owned on managed products: every method answers `Unimplemented`.

## gRPC boundary

`SystemService`, `NetworkService` and `UpgradeService` nest `BoserOwnershipInterceptor` inside `AuthInterceptor`, so
authentication runs before the ownership policy. On a managed product, the ownership interceptor rejects the following
exact gRPC methods with `Unimplemented` and the message `This operation is managed by Boser on this platform`:

| Boser-owned method                     | System effect                                      |
| -------------------------------------- | -------------------------------------------------- |
| `NetworkService.SetNetworkConfig`      | changes the network protocol configuration         |
| `NetworkService.SetWifi`               | changes saved Wi-Fi networks and the active uplink |
| `SystemService.CreatePassword`         | changes system authentication state                |
| `SystemService.ChangePassword`         | changes system authentication state                |
| `SystemService.RemovePassword`         | changes system authentication state                |
| `SystemService.SetTimezone`            | changes the system timezone                        |
| `SystemService.FactoryReset`           | resets system state                                |
| `SystemService.Reboot`                 | controls the system lifecycle                      |
| `UpgradeService.CheckForUpgrade`       | resolves an upgrade against the remote indexes     |
| `UpgradeService.GetInstallableWidgets` | resolves installable packages                      |
| `UpgradeService.StartUpgrade`          | starts a package or firmware upgrade               |
| `UpgradeService.SetAutoUpgrade`        | changes the automatic-upgrade preference           |
| `UpgradeService.GetAutoUpgrade`        | reads the automatic-upgrade preference             |

Rejection happens before protobuf decoding and before the handler, so the request cannot persist data or invoke a
backend. Matching is deliberately limited to the canonical service and method pair; a method with the same name on
another service is unaffected.

Read-only network and system methods remain available. The native tray `Restart` and `ReconfigureWifi` commands also
remain BMC-owned. Restart is an explicitly confirmed local action, while Wi-Fi reconfiguration starts the BMC-owned
setup access point and captive portal. The remote reboot and network-setting methods are blocked so a management client
cannot compete with Boser.

## Frontend boundary

The frontend reads the fixed `boser_managed` capability at startup. On managed products, it uses Boser's header and
navigation, hides the Network Configuration page, and redirects direct visits to that page. In System Settings, it hides
the Security and Upgrades tabs, skips the upgrade-feed request, omits the timezone editor, and replaces the local
reboot, support-archive, and factory-reset controls with a link to Boser's System page.

These visibility rules keep unavailable operations out of the normal UI. The server-side ownership interceptor remains
authoritative for direct or older clients.

## Mutation ownership audit

Every mutating web gRPC service has an explicit managed-product owner:

| Service                    | Owned methods                                                                                                                                                                     | Owner on managed products | Handling                                                                     |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- | ---------------------------------------------------------------------------- |
| `AuthenticationService`    | `Login`, `Logout`                                                                                                                                                                 | BMC                       | operate BMC web sessions                                                     |
| `AccountManagementService` | `UpsertAccount`, `RemoveAccount`                                                                                                                                                  | BMC                       | manage widget account bindings and secrets                                   |
| `AlarmService`             | `AddAlarm`, `SetAlarm`, `DeleteAlarm`, `SetAlarmEnabled`                                                                                                                          | BMC                       | manage alarm state                                                           |
| `ConfigurationService`     | all `Set*`, `ShowSecondsInStatusBar`, `PlaySound`                                                                                                                                 | BMC                       | manage display, sound, localization, telemetry, and presentation preferences |
| `InitialSetupService`      | `SetWifi`, `SkipWifi`, `SetupDevice`                                                                                                                                              | BMC setup and recovery    | retain provisioning checks                                                   |
| `LedTestService`           | `SetEffect`, `SetBrightness`, `Disable`, `Enable`                                                                                                                                 | BMC                       | control presentation hardware                                                |
| `NetworkService`           | `SetNetworkConfig`, `SetWifi`                                                                                                                                                     | Boser                     | reject in `BoserOwnershipInterceptor` after authentication                   |
| `SceneManagementService`   | `AddFullscreenScene`, `AddCombinedScene`, `UpdateScene`, `MoveScene`, `CloneScene`, `RemoveScene`, `PreviewScene`, `AddWidget`, `UpdateWidget`, `RemoveWidget`, `SetSceneCycling` | BMC                       | manage scenes, widgets, and presentation state                               |
| `SystemService`            | `CreatePassword`, `ChangePassword`, `RemovePassword`, `SetTimezone`, `FactoryReset`, `Reboot`                                                                                     | Boser                     | reject in `BoserOwnershipInterceptor` after authentication                   |
| `UpgradeService`           | `CheckForUpgrade`, `GetInstallableWidgets`, `StartUpgrade`, `SetAutoUpgrade`, `GetAutoUpgrade`                                                                                    | Boser                     | reject in `BoserOwnershipInterceptor` after authentication                   |

`CredentialManagementService`, `MetadataService`, and the remaining methods on the listed services are read-only.

## Initial setup and Wi-Fi recovery

`InitialSetupService` is structurally outside both `AuthInterceptor` and `BoserOwnershipInterceptor`. `SetupDevice`
remains restricted to `SetupPending` and can set network, timezone, password, and BMC presentation preferences directly.
On mining products, it also writes the pool seed Boser consumes on first boot, applies the hostname with the network
settings, advances provisioning, and starts `boser` and `bosminer`. Factory-default Wi-Fi setup remains available as
well.

The service also accepts Wi-Fi changes in a persisted `WifiReconfiguration` state. The native settings command is the
fresh transition into that state outside failure recovery and remains BMC-owned on every platform because only BMC can
start the setup access point and captive portal. It may enter Wi-Fi reconfiguration from `Operational` and is
deliberately independent of `HardwareCapabilities::boser_managed`.

This recovery operation is distinct from `NetworkService.SetWifi`: the native command starts BMC's provisioning flow,
while the gRPC method changes saved networks and the active uplink and remains Boser-owned after setup.

## Upgrades and local maintenance

Every `UpgradeService` method nests `BoserOwnershipInterceptor` inside `AuthInterceptor` and answers `Unimplemented` on
managed products, the read-only `GetAutoUpgrade` included: Boser owns package and firmware upgrades and their
automatic-upgrade configuration there. The frontend hides the Upgrades tab and skips its upgrade-feed request on managed
products.

`SystemUpgradeService` nevertheless prevents managed products from running competing local maintenance:

- effective local automatic-upgrade enablement is always false;
- startup registers no automatic-upgrade job and spawns no trigger listener;
- immediate automatic checks are ignored;
- startup registers no periodic Nix garbage-collection job.

Applying the managed automatic-upgrade setting still cancels any existing scheduler source before returning. The
persisted preference is retained for backward compatibility but has no local runtime effect. Manual upgrade preflight
remains non-collecting; Boser is responsible for managed store reclamation.

## Timezone visibility

BMC seeds its timezone watch from the operating system at process start. On managed products, Boser will provide a
streaming API for timezone changes. The Boser-backed integration will subscribe to that API and publish changes through
BMC's existing timezone watch. This will let the compositor and widgets receive updates without restarting BMC.

## Extending the API

Adding any `SystemService`, `NetworkService` or `UpgradeService` method requires assigning its exact path a
`ManagedRpcOwner` value. `every_ownership_intercepted_service_method_has_the_expected_owner` compares those decisions
with the generated descriptor set, so an unclassified addition fails the test suite. A Boser-owned mutation on another
service must also nest `BoserOwnershipInterceptor` inside `AuthInterceptor` and extend the completeness test. Adding a
BMC-owned mutation requires an explicit decision in the audit above. Keep the corresponding frontend capability
predicates aligned with that decision while retaining server-side enforcement for direct and older clients.
