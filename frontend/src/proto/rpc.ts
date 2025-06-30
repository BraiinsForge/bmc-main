import { getClient } from './transport';

import {
    AuthenticationService,
    ConfigurationService,
    InitialSetupService,
    MetadataService,
    SystemService,
    UpgradeService,
} from './pb';

// Utils
export const rpc = {
    init: getClient(InitialSetupService),

    auth: getClient(AuthenticationService),
    meta: getClient(MetadataService),
    sys: getClient(SystemService),
    config: getClient(ConfigurationService),
    upgrade: getClient(UpgradeService),
};

// Export services
export const services = {
    AuthenticationService,
    InitialSetupService,
    MetadataService,
    SystemService,
    UpgradeService,
};
