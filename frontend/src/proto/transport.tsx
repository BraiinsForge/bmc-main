import type { DescService } from '@bufbuild/protobuf';
import { createClient, type Client } from '@connectrpc/connect';
import { createGrpcWebTransport } from '@connectrpc/connect-web';

import { GrpcMockInterceptor } from '@/lib/proto';
import { store } from '@/store';

export const mocks = new GrpcMockInterceptor();

// Custom fetch that injects the Authorization header
function fetchWithAuthorizationHeader(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
    const headers = new Headers(init?.headers);
    headers.set('Authorization', `Bearer ${store.token}`);

    const requestInit: RequestInit = { ...init, headers };
    return window.fetch(input, requestInit);
}

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
    fetch: fetchWithAuthorizationHeader,
});

export function getClient<T extends DescService>(service: T): Client<T> {
    return createClient(service, transport);
}
