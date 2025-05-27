import { getClient } from './transport';

import { AuthenticationService, MetadataService, SystemService, UpgradeService } from './pb';

// Utils
export const rpc = {
    auth: getClient(AuthenticationService),
    meta: getClient(MetadataService),
    sys: getClient(SystemService),
    upgrade: getClient(UpgradeService),
};

// Export services
export const services = {
    AuthenticationService,
    MetadataService,
    SystemService,
    UpgradeService,
};
