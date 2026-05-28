import { getClient } from './transport';

import {
    AccountManagementService,
    AlarmService,
    AuthenticationService,
    ConfigurationService,
    HardwareService,
    InitialSetupService,
    MetadataService,
    NetworkService,
    SceneManagementService,
    SystemService,
    UpgradeService,
} from './pb';

// Utils
export const rpc = {
    init: getClient(InitialSetupService),

    accounts: getClient(AccountManagementService),
    auth: getClient(AuthenticationService),
    alarm: getClient(AlarmService),
    config: getClient(ConfigurationService),
    hardware: getClient(HardwareService),
    meta: getClient(MetadataService),
    net: getClient(NetworkService),
    sys: getClient(SystemService),
    upgrade: getClient(UpgradeService),
    scenes: getClient(SceneManagementService),
};

// Export services
export const services = {
    AccountManagementService,
    AuthenticationService,
    HardwareService,
    InitialSetupService,
    MetadataService,
    SceneManagementService,
    SystemService,
    UpgradeService,
};
