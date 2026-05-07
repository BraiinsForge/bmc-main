import { useState } from 'react';
import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { create } from '@/proto';
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
            create(pb.ManifestParamDefinitionSchema, {
                key: 'label',
                name: 'Label',
                description: 'Free-form text.',
                kind: { case: 'paramString', value: create(pb.ParamStringSchema, { defaultValue: 'Hello' }) },
            }),
        )}
    />
);

export const StringEnum = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'theme',
                name: 'Theme',
                kind: {
                    case: 'paramString',
                    value: create(pb.ParamStringSchema, {
                        defaultValue: 'light',
                        enumValues: [
                            create(pb.StringOptionSchema, { value: 'light', label: 'Light' }),
                            create(pb.StringOptionSchema, { value: 'dark', label: 'Dark' }),
                            create(pb.StringOptionSchema, { value: 'auto', label: 'Auto' }),
                        ],
                    }),
                },
            }),
        )}
    />
);

export const BooleanParam = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'enabled',
                name: 'Enabled',
                kind: { case: 'paramBoolean', value: create(pb.ParamBooleanSchema, { defaultValue: true }) },
            }),
        )}
    />
);

export const IntegerParam = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'refreshSeconds',
                name: 'Refresh interval (s)',
                kind: {
                    case: 'paramInteger',
                    value: create(pb.ParamIntegerSchema, { defaultValue: 30, min: 1, max: 600 }),
                },
            }),
        )}
    />
);

export const IntegerEnum = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'multiplier',
                name: 'Multiplier',
                kind: {
                    case: 'paramInteger',
                    value: create(pb.ParamIntegerSchema, {
                        defaultValue: 1,
                        enumValues: [
                            create(pb.IntegerOptionSchema, { value: 1, label: '1×' }),
                            create(pb.IntegerOptionSchema, { value: 2, label: '2×' }),
                            create(pb.IntegerOptionSchema, { value: 5, label: '5×' }),
                            create(pb.IntegerOptionSchema, { value: 10, label: '10×' }),
                        ],
                    }),
                },
            }),
        )}
    />
);

export const DoubleParam = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'scale',
                name: 'Scale factor',
                kind: {
                    case: 'paramDouble',
                    value: create(pb.ParamDoubleSchema, { defaultValue: 1.0, min: 0.1, max: 10.0, step: 0.1 }),
                },
            }),
        )}
    />
);

export const Timezone = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'tz',
                name: 'Timezone',
                kind: { case: 'paramTimezone', value: create(pb.ParamTimezoneSchema) },
            }),
        )}
    />
);

export const KitchenSink = () => (
    <Demo
        manifest={manifestWith(
            create(pb.ManifestParamDefinitionSchema, {
                key: 'label',
                name: 'Label',
                kind: { case: 'paramString', value: create(pb.ParamStringSchema, { defaultValue: 'Demo' }) },
            }),
            create(pb.ManifestParamDefinitionSchema, {
                key: 'theme',
                name: 'Theme',
                kind: {
                    case: 'paramString',
                    value: create(pb.ParamStringSchema, {
                        defaultValue: 'light',
                        enumValues: [
                            create(pb.StringOptionSchema, { value: 'light', label: 'Light' }),
                            create(pb.StringOptionSchema, { value: 'dark', label: 'Dark' }),
                        ],
                    }),
                },
            }),
            create(pb.ManifestParamDefinitionSchema, {
                key: 'enabled',
                name: 'Enabled',
                kind: { case: 'paramBoolean', value: create(pb.ParamBooleanSchema, { defaultValue: true }) },
            }),
            create(pb.ManifestParamDefinitionSchema, {
                key: 'refreshSeconds',
                name: 'Refresh interval (s)',
                kind: {
                    case: 'paramInteger',
                    value: create(pb.ParamIntegerSchema, { defaultValue: 30, min: 1, max: 600 }),
                },
            }),
            create(pb.ManifestParamDefinitionSchema, {
                key: 'scale',
                name: 'Scale',
                kind: {
                    case: 'paramDouble',
                    value: create(pb.ParamDoubleSchema, { defaultValue: 1.0, min: 0.1, max: 10.0 }),
                },
            }),
            create(pb.ManifestParamDefinitionSchema, {
                key: 'tz',
                name: 'Timezone',
                kind: { case: 'paramTimezone', value: create(pb.ParamTimezoneSchema) },
            }),
        )}
        size={pb.WidgetSize.MEDIUM}
        sizeOptions={[pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE]}
        error="Example inline error to exercise the InlineNotification chrome."
    />
);
