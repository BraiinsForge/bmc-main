// Mocks
import { mocks } from './transport';

// RPC
import * as pb from './index';

const { SystemService, AuthenticationService } = pb.services;

interface MockConf {
    verbose?: boolean;
    clearConsole?: boolean;
}
export function mockManager(c?: MockConf): Fn {
    if (c?.clearConsole) console.clear();
    mocks.config = { verbose: !!c?.verbose };

    mocks.service(SystemService, {
        getMetadata: () => pb.create(pb.MetadataSchema, { version: '1.0.0' }),
        setPassword: () => ({}),
    });

    mocks.service(AuthenticationService, {
        login: () => ({
            token: '51674a62-18df-4e0e-a1ed-1c0aa0ab12b7',
            timeoutS: 7200 * 1600,
        }),
    });

    if (c?.verbose) mocks.printMocks();
    return mocks.clear;
}
