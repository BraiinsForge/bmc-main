import { getClient } from './transport';

// Export root-level PBs
export * from './gen/authentication_pb';
export * from './gen/metadata_pb';
export * from './gen/system_pb';
export * from './gen/upgrade_pb';

// Services
import { AuthenticationService } from './gen/authentication_pb';
import { MetadataService } from './gen/metadata_pb';
import { SystemService } from './gen/system_pb';
import { UpgradeService } from './gen/upgrade_pb';

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
