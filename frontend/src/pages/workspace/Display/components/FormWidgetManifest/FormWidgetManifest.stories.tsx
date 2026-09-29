// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { useState } from 'react';
import { action } from 'storybook/actions';

import * as pb from '@/proto';
import { create } from '@/proto';
import { ParamField } from '@/components/ParamField';
import {
    defaultFormifiedValue,
    widgetParamsToFormifiedState,
    type FormifiedParams,
    type FormifiedValue,
    type ParamsFormErrors,
} from '../../fn';
import { FormWidgetManifest, WidgetManifestForm, type WidgetManifestFormProps } from './FormWidgetManifest';

// Styles
import css from './FormWidgetManifest.stories.scss';

interface Args {
    invalid: boolean;
}

export default {
    title: 'Display/Components/FormWidgetManifest',
    component: WidgetManifestForm,
    args: {
        invalid: false,
    },
    argTypes: {
        invalid: { control: { type: 'boolean' } },
    },
};

const TIMEZONES: pb.Timezone[] = [
    pb.create(pb.TimezoneSchema, { id: 'UTC', label: 'UTC', offset: '+00:00' }),
    pb.create(pb.TimezoneSchema, { id: 'Europe/Prague', label: 'Europe/Prague', offset: '+01:00' }),
    pb.create(pb.TimezoneSchema, { id: 'America/Los_Angeles', label: 'America/Los Angeles', offset: '-08:00' }),
];

const ACCOUNTS: pb.Account[] = [
    pb.create(pb.AccountSchema, {
        id: 'acct-pool-1',
        name: 'Main Pool',
        typeId: 'braiins-pool',
        createdAt: pb.create(pb.TimestampSchema, { seconds: 1_700_000_000n }),
    }),
    pb.create(pb.AccountSchema, {
        id: 'acct-pool-2',
        name: 'Backup Pool',
        typeId: 'braiins-pool',
        createdAt: pb.create(pb.TimestampSchema, { seconds: 1_710_000_000n }),
    }),
    pb.create(pb.AccountSchema, {
        id: 'acct-token',
        name: 'Weather API',
        typeId: 'generic-token',
        createdAt: pb.create(pb.TimestampSchema, { seconds: 1_720_000_000n }),
    }),
];

function param(key: string, name: string, kind: pb.ManifestParamDefinition['kind']): pb.ManifestParamDefinition {
    return create(pb.ManifestParamDefinitionSchema, { key, name, kind });
}

function slot(key: string, typeId: string, label: string, required: boolean): pb.CredentialSlotDefinition {
    return create(pb.CredentialSlotDefinitionSchema, {
        key,
        typeId,
        label,
        required,
        description: `Bound account supplies {{ credential.${key}.* }} at egress.`,
    });
}

const SCALAR_PARAMS: pb.ManifestParamDefinition[] = [
    param('label', 'Label', {
        case: 'paramString',
        value: create(pb.ParamStringSchema, { defaultValue: 'Demo' }),
    }),
    param('theme', 'Theme', {
        case: 'paramString',
        value: create(pb.ParamStringSchema, {
            defaultValue: 'light',
            enumValues: [
                create(pb.StringOptionSchema, { value: 'light', label: 'Light' }),
                create(pb.StringOptionSchema, { value: 'dark', label: 'Dark' }),
                create(pb.StringOptionSchema, { value: 'auto', label: 'Auto' }),
            ],
        }),
    }),
    param('enabled', 'Enabled', {
        case: 'paramBoolean',
        value: create(pb.ParamBooleanSchema, { defaultValue: true }),
    }),
    param('refreshSeconds', 'Refresh interval (s)', {
        case: 'paramInteger',
        value: create(pb.ParamIntegerSchema, { defaultValue: 30, min: 1, max: 600 }),
    }),
    param('multiplier', 'Multiplier', {
        case: 'paramInteger',
        value: create(pb.ParamIntegerSchema, {
            defaultValue: 2,
            enumValues: [
                create(pb.IntegerOptionSchema, { value: 1, label: '1×' }),
                create(pb.IntegerOptionSchema, { value: 2, label: '2×' }),
                create(pb.IntegerOptionSchema, { value: 4, label: '4×' }),
            ],
        }),
    }),
    param('scale', 'Scale factor', {
        case: 'paramDouble',
        value: create(pb.ParamDoubleSchema, { defaultValue: 1.0, min: 0.1, max: 10.0, step: 0.1 }),
    }),
    param('gain', 'Gain', {
        case: 'paramDouble',
        value: create(pb.ParamDoubleSchema, {
            defaultValue: 1.0,
            enumValues: [
                create(pb.DoubleOptionSchema, { value: 0.5, label: '0.5×' }),
                create(pb.DoubleOptionSchema, { value: 1.0, label: '1.0×' }),
                create(pb.DoubleOptionSchema, { value: 2.0, label: '2.0×' }),
            ],
        }),
    }),
    param('tz', 'Timezone', { case: 'paramTimezone', value: create(pb.ParamTimezoneSchema) }),
];

function list(
    key: string,
    name: string,
    items: pb.ArrayItemKind['kind'],
    bounds: { minItems?: number; maxItems: number },
    defaultValue: Array<pb.FieldValue['kind']>,
): pb.ManifestParamDefinition {
    return param(key, name, {
        case: 'paramArray',
        value: create(pb.ParamArraySchema, {
            items: { kind: items },
            ...bounds,
            defaultValue: defaultValue.map(kind => create(pb.FieldValueSchema, { kind })),
        }),
    });
}

const SYMBOLS = list(
    'symbols',
    'Ticker symbols',
    { case: 'paramString', value: create(pb.ParamStringSchema) },
    { minItems: 1, maxItems: 8 },
    [
        { case: 'stringValue', value: 'NVDA' },
        { case: 'stringValue', value: 'AAPL' },
    ],
);

const ITEM_KIND_LISTS: pb.ManifestParamDefinition[] = [
    list(
        'thresholds',
        'Whole numbers',
        { case: 'paramInteger', value: create(pb.ParamIntegerSchema, { min: 0, max: 100 }) },
        { maxItems: 5 },
        [
            { case: 'integerValue', value: 10 },
            { case: 'integerValue', value: 50 },
        ],
    ),
    list(
        'weights',
        'Decimals',
        { case: 'paramDouble', value: create(pb.ParamDoubleSchema, { defaultValue: 1.0, step: 0.1 }) },
        { minItems: 1, maxItems: 4 },
        [{ case: 'doubleValue', value: 0.5 }],
    ),
    list('flags', 'Toggles', { case: 'paramBoolean', value: create(pb.ParamBooleanSchema) }, { maxItems: 3 }, [
        { case: 'booleanValue', value: true },
        { case: 'booleanValue', value: false },
    ]),
    list(
        'zones',
        'Timezones',
        { case: 'paramTimezone', value: create(pb.ParamTimezoneSchema, { defaultValue: 'UTC' }) },
        { maxItems: 3 },
        [{ case: 'stringValue', value: 'Europe/Prague' }],
    ),
    list(
        'sides',
        'Choices',
        {
            case: 'paramString',
            value: create(pb.ParamStringSchema, {
                defaultValue: 'left',
                enumValues: [
                    create(pb.StringOptionSchema, { value: 'left', label: 'Left' }),
                    create(pb.StringOptionSchema, { value: 'center', label: 'Center' }),
                    create(pb.StringOptionSchema, { value: 'right', label: 'Right' }),
                ],
            }),
        },
        { minItems: 1, maxItems: 3 },
        [
            { case: 'stringValue', value: 'left' },
            { case: 'stringValue', value: 'right' },
        ],
    ),
];

const SLOTS: pb.CredentialSlotDefinition[] = [
    slot('pool', 'braiins-pool', 'Pool Account', true),
    slot('backup', 'braiins-pool', 'Backup Pool Account', true),
    slot('api', 'generic-token', 'Weather Service', false),
    slot('stale', 'generic-token', 'Retired Service', false),
];

function storyManifest(
    subname: string,
    fields: { params?: pb.ManifestParamDefinition[]; credentials?: pb.CredentialSlotDefinition[] },
): pb.WidgetManifest {
    return pb.create(pb.WidgetManifestSchema, {
        uid: `storybook-${subname}`,
        name: 'Storybook Widget',
        subname,
        version: '0.0.0',
        supportedSizes: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE, pb.WidgetSize.FULL],
        ...fields,
    });
}

const SCALARS_MANIFEST = storyManifest('scalar params', { params: SCALAR_PARAMS });
const CREDENTIALS_MANIFEST = storyManifest('credential slots', { credentials: SLOTS });
const FULL_MANIFEST = storyManifest('every field kind', { params: [...SCALAR_PARAMS, SYMBOLS], credentials: SLOTS });

// `backup` is left unbound and `stale` holds an account of the wrong type,
// so the required-slot warning and the misbound-slot error are both on screen.
// A binding whose account is *gone* would render the same, but never arrives:
// the server drops it, where a type mismatch survives to reach the editor.
const INITIAL_BINDINGS: Record<string, string> = {
    pool: 'acct-pool-1',
    api: 'acct-token',
    stale: 'acct-pool-1',
};

function invalidErrors(manifest: pb.WidgetManifest): ParamsFormErrors {
    return {
        global: ['The widget could not be saved.'],
        fields: Object.fromEntries(manifest.params.map(p => [p.key, [`${p.name} is not acceptable.`]])),
        items: { [SYMBOLS.key]: [undefined, ['Unknown symbol.']] },
        credentials: Object.fromEntries(manifest.credentials.map(s => [s.key, ['Account not found']])),
    };
}

function useDemoProps(manifest: pb.WidgetManifest, invalid: boolean): WidgetManifestFormProps {
    const [params, setParams] = useState<FormifiedParams>(() => widgetParamsToFormifiedState(manifest, undefined));
    const [bindings, setBindings] = useState<Record<string, string>>(INITIAL_BINDINGS);

    return {
        manifest,
        params,
        errors: invalid ? invalidErrors(manifest) : null,
        onParamChange: (key, value) => setParams(prev => ({ ...prev, [key]: value })),
        timezones: TIMEZONES,
        accounts: ACCOUNTS,
        credentialBindings: bindings,
        onCredentialBindingChange: (slotKey, accountId) => {
            action('onCredentialBindingChange')(slotKey, accountId);
            setBindings(prev => {
                const next = { ...prev };
                if (accountId) next[slotKey] = accountId;
                else delete next[slotKey];
                return next;
            });
        },
    };
}

function Boxed(props: WidgetManifestFormProps) {
    return (
        <div className="ui-box" style={{ maxWidth: 560 }}>
            <WidgetManifestForm {...props} />
        </div>
    );
}

export function ScalarFields({ invalid }: Args) {
    return <Boxed {...useDemoProps(SCALARS_MANIFEST, invalid)} />;
}

function ListCell({ definition, invalid }: { definition: pb.ManifestParamDefinition; invalid: boolean }) {
    const [value, setValue] = useState<FormifiedValue>(() => defaultFormifiedValue(definition));
    return (
        <div className={css.cell}>
            <ParamField
                id={`story-${definition.key}`}
                definition={definition}
                value={value}
                error={invalid ? `${definition.name} is not acceptable.` : undefined}
                itemErrors={invalid ? [undefined, 'Not acceptable.'] : undefined}
                onChange={(_key, next) => setValue(next)}
                timezones={TIMEZONES}
            />
        </div>
    );
}

export function ListField({ invalid }: Args) {
    return (
        <div className={css.grid}>
            {[SYMBOLS, ...ITEM_KIND_LISTS].map(definition => (
                <ListCell key={definition.key} definition={definition} invalid={invalid} />
            ))}
        </div>
    );
}

export function CredentialSlots({ invalid }: Args) {
    return <Boxed {...useDemoProps(CREDENTIALS_MANIFEST, invalid)} />;
}

export function InDialog({ invalid }: Args) {
    const props = useDemoProps(FULL_MANIFEST, invalid);
    const [size, setSize] = useState<pb.WidgetSize>(pb.WidgetSize.MEDIUM);
    return (
        <FormWidgetManifest
            {...props}
            size={size}
            sizeOptions={[pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE]}
            onSizeChange={setSize}
            isOpen
            onSave={action('onSave')}
            onCancel={action('onCancel')}
        />
    );
}
