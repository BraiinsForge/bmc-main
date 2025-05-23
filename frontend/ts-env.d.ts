import type * as React from 'react';

declare global {
    //
    // Missing Utils
    //

    type ValuesOf<T> = T[keyof T];
    type DeNullProp<O extends Dict, P extends keyof O> = O & {
        [K in P]-?: NonNullable<O[P]>;
    };

    //
    // Generics
    //

    // While these don't add any aditional type safety,
    // their name serves a semantic documentation purpose
    type Timestamp = number;
    type TimestampMs = number;
    // Date string in ISO 8601 format: https://en.wikipedia.org/wiki/ISO_8601
    type Datetime = string;

    type Seconds = number;
    type Milliseconds = number;

    type Length = number;
    type Count = number;
    type Float<Low = number, High = number> = number;
    type Delta<T extends number> = T;
    type Integer<Low = number, High = number> = number;
    type Index = number;
    type Hex = string;
    type StrNum = string | number;

    export type IPv4 = `${number}.${number}.${number}.${number}`;
    export type IPv6 = `${Hex}:${Hex}:${Hex}:${Hex}:${Hex}:${Hex}:${Hex}:${Hex}`;
    export type IP = IPv4 | IPv6;

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
    type ProtoEnumRecord<Enum extends keyof any, Value> = Omit<Record<Enum, Value>, 0>;
    type ProtoEnumMap<Enum extends keyof any, Value> = Omit<Map<Enum, Value>, 0>;
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

    type Dict<K extends keyof any = keyof any, V = unknown> = Record<K, V>;
    type Writeable<T> = { -readonly [P in keyof T]: T[P] };
    type Maybe<T> = undefined | null | T;
    type Null<T> = null | T;

    type MaybeArray<T> = T | Array<T>;
    type Getter<R, Args extends unknown[] | void = void> = Args extends unknown[] ? (...args: Args) => R : () => R;
    type MaybeGetter<R, Args extends unknown[] | void = void> = R | Getter<R, Args>;
    type MaybePromise<T> = T | Promise<T>;

    type Fn<R = void> = () => R;
    type AnyFunction<R = unknown> = (...args: unknown[]) => R;
    type AttributeUsageConst = 'NONE' | 'SOME' | 'ALL';

    //
    // React
    //

    type ReactState<State extends Record<string, unknown>> = Pick<State, keyof State> | State;
    type ReactNode = React.ReactNode;
    type ReactElement<
        P = unknown,
        T extends string | React.JSXElementConstructor<unknown> = string | React.JSXElementConstructor<unknown>,
    > = React.ReactElement<P, T>;
    type CSSProperties = React.CSSProperties;
    type HTMLAttributes<T = unknown> = React.HTMLAttributes<T>;
}
