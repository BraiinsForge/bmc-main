import { getClient } from './transport';

// Services
import { SystemService } from './gen/web/system_pb.ts';
import { AuthenticationService } from './gen/web/authentication_pb.ts';

// Utils
export const rpc = {
    auth: getClient(AuthenticationService),
    sys: getClient(SystemService),
};

// Export services
export const services = {
    SystemService,
    AuthenticationService,
};

// Export root-level PBs
export * from './gen/web/system_pb.ts';
export * from './gen/web/authentication_pb.ts';
