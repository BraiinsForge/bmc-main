import type { DescService } from '@bufbuild/protobuf';
import { createClient, type Client } from '@connectrpc/connect';
import { createGrpcWebTransport } from '@connectrpc/connect-web';

import { GrpcMockInterceptor } from '@/lib/proto';

export const mocks = new GrpcMockInterceptor();

export const transport = createGrpcWebTransport({
    baseUrl: '/',
    useBinaryFormat: true,
    interceptors: [mocks.interceptor],
    binaryOptions: {
        readUnknownFields: true,
        writeUnknownFields: true,
    },
    jsonOptions: {
        enumAsInteger: false,
        alwaysEmitImplicit: true,
        useProtoFieldName: true,
        ignoreUnknownFields: true,
    },
});

export function getClient<T extends DescService>(service: T): Client<T> {
    return createClient(service, transport);
}
