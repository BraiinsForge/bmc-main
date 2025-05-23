import { getClient } from './transport';

// Export root-level PBs
export * from './gen/web/authentication_pb';
export * from './gen/web/metadata_pb';
export * from './gen/web/system_pb';
export * from './gen/web/upgrade_pb';

// Services
import { AuthenticationService } from './gen/web/authentication_pb';
import { MetadataService } from './gen/web/metadata_pb';
import { SystemService } from './gen/web/system_pb';
import { UpgradeService } from './gen/web/upgrade_pb';

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
