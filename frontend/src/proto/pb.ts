export * from '@bufbuild/protobuf/wkt';
export type { CallOptions } from '@connectrpc/connect';

export * from './gen/web/authentication_pb';
export * from './gen/web/initial_setup_pb';
export * from './gen/web/metadata_pb';
export * from './gen/web/shared_pb';
export * from './gen/web/system_pb';
export * from './gen/web/upgrade_pb';

export enum SceneKind {
    combined = 'combined',
    clock = 'clock',
    image = 'image',
    ticker = 'ticker',
    pool = 'pool',
    manager = 'manager',
}

export enum SceneVariantClock {
    analog_rect = 'analog_rect',
    analog_round = 'analog_round',
    digital_flip = 'digital_flip',
    digital_plain = 'digital_plain',
}
export enum SceneVariantTicker {
    line = 'line',
    list = 'list',
    candle = 'candle',
}
export type SceneVariant = SceneVariantClock | SceneVariantTicker;

export interface Scene {
    id: number;
    title: string;
    description: string;
    enabled: boolean;
    durationSeconds: number;

    kind: SceneKind;
    variant?: SceneVariant;
}

export enum ClockStyle {
    analog1 = 'analog1',
    analog2 = 'analog2',
    digital1 = 'digital1',
    digital2 = 'digital2',
}
export enum FontStyle {
    light = 'light',
    medium = 'medium',
    bold = 'bold',
}
