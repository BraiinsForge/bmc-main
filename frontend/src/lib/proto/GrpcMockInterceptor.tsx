// Generics
import { delay } from '@/lib/async';
import { Aborter } from '@/lib/dom';
import { cloneDeep } from 'es-toolkit';
import { abortableAsyncIterable } from '@/lib/iter';
import { pipeAsync, map, wait } from 'iter-ops';

// RPC libs
import type { DescMethod, DescMethodStreaming, Message } from '@bufbuild/protobuf';
import type { Interceptor, UnaryResponse, StreamResponse } from '@connectrpc/connect';
import type { GenService, GenServiceMethods, GenMessage } from '@bufbuild/protobuf/codegenv2';

type GetRuntimeShape<T extends GenMessage<any>> = T extends GenMessage<infer M> ? M : never;
type PlainMessage<T extends Message> = Omit<T, '$typeName' | '$unknown'>;

// A json object respresentation for the message type that the method returns
type MockFunConf = {
    // - Static value
    // - Getter called for each response message (in server streaming methods).
    //   Can be used to simulate more realistic reponse staggering.
    delay?: number | ((counter: number) => number);
};
export type GrpcConfSetter = (configuration: MockFunConf) => void;

interface MockFnInput<RequestMessage extends Message> {
    req: RequestMessage;
    ctx: MockState;
    conf: GrpcConfSetter;

    // Aborting
    signal: AbortSignal;
    abort(): void;
}
type MockFn<Method extends DescMethod, Input extends Message, Output extends Message> = (
    d: MockFnInput<Input>,
) => Method['methodKind'] extends 'server_streaming'
    ? // List of response for server streaming methods
      PlainMessage<Output>[] | AsyncIterable<PlainMessage<Output>>
    : // Unary response
      PlainMessage<Output>;

/** Describes a mapping of service method names to their mockers */
type ServiceMocks<Service extends GenService<any>> = {
    [Key in keyof Service['method']]: MockFn<
        Service['method'][Key],
        GetRuntimeShape<Service['method'][Key]['input']>,
        GetRuntimeShape<Service['method'][Key]['output']>
    >;
};

type MockState = {
    $$state: Record<string, number>;
    service: string;
    method: string;
    n(key?: string): number;
};

const defaultState: MockState = {
    $$state: {},
    service: '',
    method: '',
    n(this: MockState, key = `${this.service}/${this.method}`) {
        this.$$state[key] = (this.$$state[key] ?? 0) + 1;
        return this.$$state[key];
    },
};
export interface GrpcMockInterceptorConfig {
    verbose?: boolean;
}

export class GrpcMockInterceptor {
    constructor(conf?: GrpcMockInterceptorConfig) {
        if (conf) this.config = conf;
    }

    #config: GrpcMockInterceptorConfig = {};
    public set config(conf: GrpcMockInterceptorConfig) {
        Object.assign(this.#config, conf);
    }
    public get config(): GrpcMockInterceptorConfig {
        return Object.freeze({ ...this.#config });
    }

    /** Global state shared accross all mockers usefull for faking a server state */
    #state: MockState = cloneDeep(defaultState);

    /** Map of mocked service methods */
    #mocks: Partial<Record<string, MockFn<DescMethod, any, any>>> = {};
    public clear = (): this => {
        this.#mocks = {};
        this.#state = cloneDeep(defaultState);
        return this;
    };

    /** Type-aware safeguard for mocking all service methods */
    public service<Service extends GenService<GenServiceMethods>>(
        service: Service,
        methods: ServiceMocks<Service>,
    ): this {
        Object.entries(methods).forEach(([key, value]) => {
            const methodInfo = service.method[key];
            const serverPath = `${service.typeName}/${methodInfo.name}`;
            this.#mocks[serverPath] = value;
        });
        return this;
    }

    /**
     * Interceptor is something colloquially known as "middlerware".
     * Usefull for:
     *  - mocking unimplemented / unavailable server methods
     *  - injection of authentication data
     *
     * @see https://connect.build/docs/web/interceptors
     */
    public readonly interceptor: Interceptor = next => {
        return async request => {
            // The whole mocking routine should never get into production,
            // so we'll safeguard it by build mode of the whole application
            if (process.env.NODE_ENV !== 'development') {
                const { verbose } = this.config;

                // Path is used as a lookup key from the mockers map
                const path = `${request.service.typeName}/${request.method.name}`;
                const mocker = this.#mocks[path];
                if (typeof mocker === 'function') {
                    // Resolve the input message
                    let inputMessage: Message;
                    if (Symbol.asyncIterator in request.message) {
                        // Streaming methods have the message field wrapped
                        // in an "AsyncIterable" object, so we need to unwrap it
                        // to be able to read it easily in the mocker.
                        inputMessage = (await request.message[Symbol.asyncIterator]().next()).value;
                    } else {
                        inputMessage = request.message as Message;
                    }

                    // Prepare the mocker context
                    const mockFnConf: MockFunConf = { delay: 0 };
                    const aborter = new Aborter().attach(request.signal);
                    const mockerInput = {
                        req: inputMessage,
                        ctx: Object.assign({}, this.#state, {
                            service: request.service.typeName,
                            method: request.method.name,
                        }),
                        conf(conf: MockFunConf) {
                            Object.assign(mockFnConf, conf);
                        },
                        // Aborting
                        signal: request.signal,
                        abort: () => aborter.abort(),
                    } satisfies MockFnInput<any>;
                    const getResponseDelay = (iterationIndex?: number): number => {
                        const d = mockFnConf.delay;
                        if (typeof d === 'function') return d(iterationIndex ?? 0);
                        if (typeof d === 'number') return d;
                        return 0;
                    };

                    // Prepare the response base object and logger function
                    const logMockerResponse = (responseData: unknown): void => {
                        if (!verbose) return;

                        console.groupCollapsed(`gRPC: %c${path}`, 'color: gold;, font-weight: bold;');
                        console.log('input', inputMessage);
                        console.log('header', request.header);
                        console.log('output', responseData);
                        console.log('call config', mockFnConf);
                        console.groupEnd();
                    };
                    const responseBase = {
                        stream: request.stream,
                        header: request.header,
                        method: request.method,
                        service: request.service,
                        trailer: request.header,
                    } as Omit<UnaryResponse, 'message'>;

                    return new Promise((resolveRequest, rejectRequest) => {
                        const method = request.method;
                        switch (method.methodKind) {
                            case 'unary': {
                                const responseData = mocker(mockerInput);
                                logMockerResponse(responseData);

                                const response = {
                                    ...responseBase,
                                    message: responseData,
                                } as UnaryResponse;
                                setTimeout(() => resolveRequest(response), getResponseDelay());
                                return;
                            }

                            case 'bidi_streaming':
                            case 'client_streaming':
                            case 'server_streaming': {
                                // Type casting here gives us strict types of possible output values
                                const responseData = mocker(mockerInput) as Iterable<any>;
                                logMockerResponse(responseData);
                                let responseIterable: AsyncIterable<Message>;

                                // If we get an async iterator, we'll use it as is and exit early.
                                // This gives full control over the response stream to the mocker.
                                if (Symbol.asyncIterator in responseData) {
                                    // We have to map the iterator since the mock function
                                    // is only requested to produce a plain variant for each message,
                                    // but the RPC library requires a full message class instance.
                                    responseIterable = pipeAsync(
                                        responseData,
                                        // map(msg => new method.O(msg)),
                                    );
                                }

                                // Otherwise we'll wrap the response array in an async iterator
                                // of our own, providing an abstraction over network latency.
                                else {
                                    let iterationIndex = -1;
                                    let lastIteratorRead: number = Date.now();

                                    responseIterable = pipeAsync(
                                        responseData,
                                        // Simulate network latency by delaying each message
                                        // by the configured amount of time (can be dynamic).
                                        map(async msg => {
                                            iterationIndex += 1;
                                            const responseDelay = getResponseDelay(iterationIndex);

                                            const now = Date.now();
                                            const timeDelta: number = Math.floor((now - lastIteratorRead) / 1e3);
                                            lastIteratorRead = now;

                                            if (timeDelta < responseDelay) await delay(responseDelay - timeDelta);
                                            // return new method.O(msg);
                                            return msg;
                                        }),
                                        // Since the map function is async, we need
                                        // to await the messages before returning them.
                                        wait(),
                                    );
                                }

                                // We want to delay the first response
                                // the same as any following ones.
                                setTimeout(() => {
                                    resolveRequest({
                                        ...responseBase,
                                        stream: true,
                                        message: abortableAsyncIterable(responseIterable, aborter.signal),
                                        method: method satisfies DescMethodStreaming<any, any>,
                                    } satisfies StreamResponse);
                                }, getResponseDelay());
                                return;
                            }

                            default:
                                rejectRequest(new Error(`Unsupported method kind "${method}"`));
                        }
                    });
                }
            }

            return next(request);
        };
    };

    public printMocks(): void {
        console.groupCollapsed('gRPC: %cMocked methods', 'color: violet; font-weight: bold;');
        Object.entries(this.#mocks).forEach(([path, mocker]) => {
            console.log(path, mocker);
        });
        console.groupEnd();
    }
}
