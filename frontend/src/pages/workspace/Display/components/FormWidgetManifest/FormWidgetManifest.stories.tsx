import { useState } from 'react';
import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { FormWidgetManifest as Component, type FormWidgetManifestProps } from './FormWidgetManifest';

export default {
    title: 'Display/Components/FormWidgetManifest',
    component: Component,
} satisfies Meta<FormWidgetManifestProps>;

const TIMEZONES: pb.Timezone[] = [
    pb.create(pb.TimezoneSchema, { id: 'UTC', label: 'UTC', offset: '+00:00' }),
    pb.create(pb.TimezoneSchema, { id: 'Europe/Prague', label: 'Europe/Prague', offset: '+01:00' }),
    pb.create(pb.TimezoneSchema, { id: 'America/Los_Angeles', label: 'America/Los Angeles', offset: '-08:00' }),
];

function manifestWith(...params: pb.ManifestParamDefinition[]): pb.WidgetManifest {
    return pb.create(pb.WidgetManifestSchema, {
        uid: 'storybook-manifest',
        name: 'Storybook Widget',
        description: 'Manifest used to demo a single ParamType.',
        version: '0.0.0',
        supportedSizes: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE, pb.WidgetSize.FULL],
        params,
    });
}

function param(overrides: Partial<pb.ManifestParamDefinition>): pb.ManifestParamDefinition {
    return pb.create(pb.ManifestParamDefinitionSchema, {
        key: 'value',
        name: 'Value',
        paramType: pb.ManifestParamType.STRING,
        defaultValue: '""',
        enumValues: {},
        ...overrides,
    });
}

function Demo(props: {
    manifest: pb.WidgetManifest;
    initialParams?: Record<string, string>;
    error?: string | null;
    size?: pb.WidgetSize;
    sizeOptions?: FormWidgetManifestProps['sizeOptions'];
}) {
    const { manifest, initialParams, error, size, sizeOptions } = props;
    const [params, setParams] = useState<Record<string, string>>(initialParams ?? {});
    const [currentSize, setCurrentSize] = useState<pb.WidgetSize | undefined>(size);

    return (
        <Component
            isOpen
            onSave={action('onSave')}
            onCancel={action('onCancel')}
            error={error ?? null}
            manifest={manifest}
            params={params}
            onParamChange={(key, value) => {
                setParams(prev => {
                    const next = { ...prev };
                    if (value === undefined) delete next[key];
                    else next[key] = value;
                    return next;
                });
            }}
            timezones={TIMEZONES}
            size={currentSize}
            sizeOptions={sizeOptions}
            onSizeChange={setCurrentSize}
        />
    );
}

export const StringParam = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'label',
                name: 'Label',
                paramType: pb.ManifestParamType.STRING,
                description: 'Free-form text.',
                defaultValue: '"Hello"',
            }),
        )}
    />
);

export const StringEnum = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'theme',
                name: 'Theme',
                paramType: pb.ManifestParamType.STRING,
                defaultValue: '"light"',
                enumValues: { light: 'Light', dark: 'Dark', auto: 'Auto' },
            }),
        )}
    />
);

export const BooleanParam = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'enabled',
                name: 'Enabled',
                paramType: pb.ManifestParamType.BOOLEAN,
                defaultValue: 'true',
            }),
        )}
    />
);

export const NumberParam = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'refreshSeconds',
                name: 'Refresh interval (s)',
                paramType: pb.ManifestParamType.NUMBER,
                defaultValue: '30',
                min: 1,
                max: 600,
            }),
        )}
    />
);

export const NumberEnum = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'multiplier',
                name: 'Multiplier',
                paramType: pb.ManifestParamType.NUMBER,
                defaultValue: '1',
                enumValues: { '1': '1×', '2': '2×', '5': '5×', '10': '10×' },
            }),
        )}
    />
);

export const ArrayParam = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'symbols',
                name: 'Tracked symbols',
                paramType: pb.ManifestParamType.ARRAY,
                description: 'JSON array of ticker symbols.',
                defaultValue: '["BTC","ETH"]',
            }),
        )}
    />
);

export const Timezone = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'tz',
                name: 'Timezone',
                paramType: pb.ManifestParamType.TIMEZONE,
                defaultValue: 'null',
            }),
        )}
    />
);

export const InvalidAndEmpty = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'count',
                name: 'Count (was stored as JSON string)',
                paramType: pb.ManifestParamType.NUMBER,
                defaultValue: '0',
            }),
            param({
                key: 'symbols',
                name: 'Symbols (malformed JSON)',
                paramType: pb.ManifestParamType.ARRAY,
                defaultValue: '[]',
            }),
        )}
        initialParams={{
            count: '"on"',
            symbols: '[BTC,ETH',
        }}
    />
);

export const KitchenSink = () => (
    <Demo
        manifest={manifestWith(
            param({
                key: 'label',
                name: 'Label',
                paramType: pb.ManifestParamType.STRING,
                defaultValue: '"Demo"',
            }),
            param({
                key: 'theme',
                name: 'Theme',
                paramType: pb.ManifestParamType.STRING,
                defaultValue: '"light"',
                enumValues: { light: 'Light', dark: 'Dark' },
            }),
            param({
                key: 'enabled',
                name: 'Enabled',
                paramType: pb.ManifestParamType.BOOLEAN,
                defaultValue: 'true',
            }),
            param({
                key: 'refreshSeconds',
                name: 'Refresh interval (s)',
                paramType: pb.ManifestParamType.NUMBER,
                defaultValue: '30',
                min: 1,
                max: 600,
            }),
            param({
                key: 'multiplier',
                name: 'Multiplier',
                paramType: pb.ManifestParamType.NUMBER,
                defaultValue: '1',
                enumValues: { '1': '1×', '2': '2×' },
            }),
            param({
                key: 'symbols',
                name: 'Symbols',
                paramType: pb.ManifestParamType.ARRAY,
                defaultValue: '["BTC"]',
            }),
            param({
                key: 'tz',
                name: 'Timezone',
                paramType: pb.ManifestParamType.TIMEZONE,
                defaultValue: 'null',
            }),
        )}
        size={pb.WidgetSize.MEDIUM}
        sizeOptions={[pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE]}
        error="Example inline error to exercise the InlineNotification chrome."
    />
);
