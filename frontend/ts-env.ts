import type * as React from 'react';

declare global {
    // While these don't add any aditional type safety,
    // their name serves a semantic documentation purpose
    type Timestamp = number;
    type Float<_Low = number, _High = number> = number;
    type Integer<_Low = number, _High = number> = number;
    type StrNum = string | number;

    type IPv4 = `${number}.${number}.${number}.${number}`;
    type IPv6 = `${string}:${string}:${string}:${string}:${string}:${string}:${string}:${string}`;

    type Rec = Record<string, unknown>;

    /**
     * Create a record type from the proto enum,
     * excluding the `UNSPECIFIED` member that is,
     * by convention, assigned a `0` value.
     *
     * @example
     *  export enum WorkerMonitoringState {
     *      UNSPECIFIED = 0,
     *      OK = 1,
     *      LOW = 2,
     *      OFF = 3,
     *      DIS = 4,
     *  }
     *  const icons: ProtoEnumRecord<WorkerMonitoringState, string> = {
     *     [WorkerMonitoringState.OK]: 'check',
     *     [WorkerMonitoringState.LOW]: 'arrow-down',
     *     [WorkerMonitoringState.OFF]: 'exclamation',
     *     [WorkerMonitoringState.DIS]: 'power',
     * }
     */
    type ProtoEnumRecord<Enum extends PropertyKey, Value> = Omit<Record<Enum, Value>, 0>;
    type ProtoEnumMap<Enum extends PropertyKey, Value> = Omit<Map<Enum, Value>, 0>;
    /**
     * Create a new union type from the oneof proto union, removing the `undefined` case.
     * Usefull for usage in a component state that doesn't allow the `undefined` case.
     *
     * @example
     *  type ProtoOneOf =
     *      | { case: 'default'; value: boolean }
     *      | { case: 'changeAbsolute'; value: number }
     *      | { case: 'changeRelative'; value: number }
     *      | { case: undefined; value?: undefined };
     *  type MyOneOf = ProtoOneofStrict<ProtoOneOf>;
     *  // MyOneOf is now:
     *  //   | { case: 'default'; value: boolean }
     *  //   | { case: 'changeAbsolute'; value: number }
     *  //   | { case: 'changeRelative'; value: number }
     */
    type ProtoOneofStrict<OneOf extends { case: unknown; value?: unknown }> = Exclude<OneOf, { case: undefined }>;
    type ProtoOneofCase<OneOf extends { case: unknown; value?: unknown }> = ProtoOneofStrict<OneOf>['case'];
    type ProtoOneofValue<OneOf extends { case: unknown; value?: unknown }> = ProtoOneofStrict<OneOf>['value'];

    type Fn<R = void> = () => R;
    type AnyFunction<R = unknown> = (...args: unknown[]) => R;
    type Getter<R, Args extends unknown[] | void = void> = Args extends unknown[] ? (...args: Args) => R : () => R;

    // Maybe values
    type Maybe<T> = undefined | null | T;
    type MaybeArray<T> = T | Array<T>;
    type MaybeGetter<R, Args extends unknown[] | void = void> = R | Getter<R, Args>;
    type MaybePromise<T> = T | Promise<T>;

    // React
    type ReactNode = React.ReactNode;
    type ReactElement<
        P = unknown,
        T extends string | React.JSXElementConstructor<unknown> = string | React.JSXElementConstructor<unknown>,
    > = React.ReactElement<P, T>;
    type CSSProperties = React.CSSProperties;
}

// biome-ignore lint/complexity/noUselessEmptyExport: Needed for typescript not to trip on the module type
export {};
