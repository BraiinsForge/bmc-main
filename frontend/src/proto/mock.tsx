// Mocks
import { mocks } from './transport';

// RPC
// import * as pb from './index';
// const { AuthenticationService, MetadataService, SystemService } = pb.services;

interface MockConf {
    verbose?: boolean;
    clearConsole?: boolean;
}
export function mockManager(c?: MockConf): Fn {
    if (c?.clearConsole) console.clear();
    mocks.config = { verbose: !!c?.verbose };

    if (c?.verbose) mocks.printMocks();
    return mocks.clear;
}
