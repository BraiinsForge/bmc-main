import { getClient } from './transport';

import {
    AlarmService,
    AuthenticationService,
    ConfigurationService,
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

    auth: getClient(AuthenticationService),
    alarm: getClient(AlarmService),
    config: getClient(ConfigurationService),
    meta: getClient(MetadataService),
    net: getClient(NetworkService),
    sys: getClient(SystemService),
    upgrade: getClient(UpgradeService),
    scenes: getClient(SceneManagementService),
};

// Export services
export const services = {
    AuthenticationService,
    InitialSetupService,
    MetadataService,
    SceneManagementService,
    SystemService,
    UpgradeService,
};
