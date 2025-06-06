import { getClient } from './transport';

import { AuthenticationService, InitialSetupService, MetadataService, SystemService, UpgradeService } from './pb';

// Utils
export const rpc = {
    init: getClient(InitialSetupService),

    auth: getClient(AuthenticationService),
    meta: getClient(MetadataService),
    sys: getClient(SystemService),
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
