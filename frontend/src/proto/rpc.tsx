import { getClient } from './transport';

// Export root-level PBs
export * from './gen/web/authentication_pb.ts';
export * from './gen/web/metadata_pb.ts';
export * from './gen/web/system_pb.ts';
export * from './gen/web/upgrade_pb.ts';

// Services
import { AuthenticationService } from './gen/web/authentication_pb.ts';
import { MetadataService } from './gen/web/metadata_pb.ts';
import { SystemService } from './gen/web/system_pb.ts';
import { UpgradeService } from './gen/web/upgrade_pb.ts';

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
