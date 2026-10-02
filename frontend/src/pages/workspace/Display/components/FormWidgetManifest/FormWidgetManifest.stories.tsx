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
import { ParamField, parseFormifiedValue, type RowError } from '@/components/ParamField';
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
import cn from 'clsx';

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

function param(
    key: string,
    name: string,
    kind: pb.ManifestParamDefinition['kind'],
    description?: string,
): pb.ManifestParamDefinition {
    return create(pb.ManifestParamDefinitionSchema, { key, name, kind, description });
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
    create(pb.ManifestParamDefinitionSchema, {
        key: 'city',
        name: 'City',
        description: 'Where the forecast is for.',
        isOptional: true,
        kind: { case: 'paramString', value: create(pb.ParamStringSchema, { placeholder: 'e.g. Prague' }) },
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
    param('period', 'Period', {
        case: 'paramString',
        value: create(pb.ParamStringSchema, {
            defaultValue: '7d',
            enumControl: pb.EnumControl.RADIO,
            enumValues: [
                create(pb.StringOptionSchema, { value: '1d', label: '1 Day' }),
                create(pb.StringOptionSchema, { value: '7d', label: '7 Days' }),
                create(pb.StringOptionSchema, { value: '1mo', label: '1 Month' }),
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
    param('columns', 'Columns', {
        case: 'paramInteger',
        value: create(pb.ParamIntegerSchema, {
            defaultValue: 2,
            enumControl: pb.EnumControl.RADIO,
            enumValues: [
                create(pb.IntegerOptionSchema, { value: 1, label: 'One' }),
                create(pb.IntegerOptionSchema, { value: 2, label: 'Two' }),
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
    param('tz', 'Timezone', {
        case: 'paramTimezone',
        value: create(pb.ParamTimezoneSchema, { placeholder: 'e.g. Europe/Prague' }),
    }),
];

function list(
    key: string,
    name: string,
    items: pb.ArrayItemKind['kind'],
    options: {
        minItems?: number;
        maxItems: number;
        uniqueItems?: pb.ParamArray['uniqueItems'];
        description?: string;
    },
    defaultValue: Array<pb.FieldValue['kind']>,
): pb.ManifestParamDefinition {
    const { description, ...bounds } = options;
    return param(
        key,
        name,
        {
            case: 'paramArray',
            value: create(pb.ParamArraySchema, {
                items: { kind: items },
                ...bounds,
                defaultValue: defaultValue.map(kind => create(pb.FieldValueSchema, { kind })),
            }),
        },
        description,
    );
}

const SYMBOLS = list(
    'symbols',
    'Ticker symbols',
    { case: 'paramString', value: create(pb.ParamStringSchema, { placeholder: 'e.g. BTC or AAPL' }) },
    { minItems: 1, maxItems: 8, description: 'Shown one after another, in this order.' },
    [
        { case: 'stringValue', value: 'NVDA' },
        { case: 'stringValue', value: 'AAPL' },
    ],
);

const LINK_ROW: pb.ArrayItemKind['kind'] = {
    case: 'paramObject',
    value: create(pb.ParamObjectSchema, {
        fields: [
            {
                key: 'label',
                name: 'Label',
                description: 'The text the widget shows for the link.',
                kind: { case: 'paramString', value: { placeholder: 'e.g. Braiins' } },
            },
            {
                key: 'url',
                name: 'URL',
                description: 'Leave empty to show the label as plain text.',
                isOptional: true,
                kind: { case: 'paramString', value: { format: pb.StringFormat.URI, placeholder: 'https://…' } },
            },
        ],
    }),
};

function link(label: string, url: string): pb.FieldValue['kind'] {
    return {
        case: 'structValue',
        value: create(pb.FieldValuesSchema, {
            fields: {
                label: create(pb.FieldValueSchema, { kind: { case: 'stringValue', value: label } }),
                url: create(pb.FieldValueSchema, { kind: { case: 'stringValue', value: url } }),
            },
        }),
    };
}

const BRAIINS_LINK = link('Braiins', 'https://braiins.com');

const LINKS = list('links', 'Objects', LINK_ROW, { maxItems: 4, description: 'Shown as a list of links.' }, [
    BRAIINS_LINK,
]);

const UNIQUE_LINKS = list(
    'linksUnique',
    'Objects, unique',
    LINK_ROW,
    {
        maxItems: 4,
        uniqueItems: { case: 'whole', value: create(pb.EmptySchema) },
        description: 'No two rows alike.',
    },
    [BRAIINS_LINK, link('Pool', 'https://pool.braiins.com')],
);

function toggled(value: boolean): pb.FieldValue['kind'] {
    return { case: 'booleanValue', value };
}

const UNIQUE_TOGGLES = list(
    'togglesUnique',
    'Toggles, unique',
    { case: 'paramBoolean', value: create(pb.ParamBooleanSchema) },
    { maxItems: 2, uniqueItems: { case: 'whole', value: create(pb.EmptySchema) }, description: 'No two alike.' },
    [toggled(true), toggled(false)],
);

function shownRow(label: string, shown: boolean): pb.FieldValue['kind'] {
    return {
        case: 'structValue',
        value: create(pb.FieldValuesSchema, {
            fields: {
                label: create(pb.FieldValueSchema, { kind: { case: 'stringValue', value: label } }),
                shown: create(pb.FieldValueSchema, { kind: toggled(shown) }),
            },
        }),
    };
}

const UNIQUE_BY_TOGGLE = list(
    'shownUnique',
    'Objects, unique by toggle',
    {
        case: 'paramObject',
        value: create(pb.ParamObjectSchema, {
            fields: [
                { key: 'label', name: 'Label', kind: { case: 'paramString', value: {} } },
                { key: 'shown', name: 'Shown', kind: { case: 'paramBoolean', value: {} } },
            ],
        }),
    },
    {
        maxItems: 2,
        uniqueItems: { case: 'by', value: create(pb.UniqueKeysSchema, { keys: ['shown'] }) },
        description: 'No two rows alike in Shown.',
    },
    [shownRow('Braiins', true), shownRow('Deck', false)],
);

const ITEM_KIND_LISTS: pb.ManifestParamDefinition[] = [
    list(
        'thresholds',
        'Whole numbers',
        { case: 'paramInteger', value: create(pb.ParamIntegerSchema, { min: 0, max: 100, placeholder: 'e.g. 42' }) },
        { maxItems: 5, description: 'Each between 0 and 100.' },
        [
            { case: 'integerValue', value: 10 },
            { case: 'integerValue', value: 50 },
        ],
    ),
    list(
        'weights',
        'Decimals',
        { case: 'paramDouble', value: create(pb.ParamDoubleSchema, { defaultValue: 1.0, step: 0.1 }) },
        { minItems: 1, maxItems: 4, description: 'At least one, in steps of 0.1.' },
        [{ case: 'doubleValue', value: 0.5 }],
    ),
    list(
        'flags',
        'Toggles',
        { case: 'paramBoolean', value: create(pb.ParamBooleanSchema) },
        { maxItems: 3, description: 'Up to three switches.' },
        [
            { case: 'booleanValue', value: true },
            { case: 'booleanValue', value: false },
        ],
    ),
    list(
        'zones',
        'Timezones',
        { case: 'paramTimezone', value: create(pb.ParamTimezoneSchema, { defaultValue: 'UTC' }) },
        { maxItems: 3, description: 'Clocks shown next to the local one.' },
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
        { minItems: 1, maxItems: 3, description: 'Pick at least one side.' },
        [
            { case: 'stringValue', value: 'left' },
            { case: 'stringValue', value: 'right' },
        ],
    ),
    LINKS,
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
        items: { [SYMBOLS.key]: [undefined, { errors: ['Unknown symbol.'] }] },
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

function invalidRow(definition: pb.ManifestParamDefinition): RowError {
    const items = definition.kind.case === 'paramArray' ? definition.kind.value.items?.kind : undefined;
    const firstField = items?.case === 'paramObject' ? items.value.fields[0]?.key : undefined;
    return firstField ? { fields: { [firstField]: 'Not acceptable.' } } : { error: 'Not acceptable.' };
}

function ListCell({ definition, invalid }: { definition: pb.ManifestParamDefinition; invalid: boolean }) {
    const [value, setValue] = useState<FormifiedValue>(() => defaultFormifiedValue(definition));
    const isObjectList =
        definition.kind.case === 'paramArray' && definition.kind.value.items?.kind.case === 'paramObject';
    const parsed = parseFormifiedValue(definition, value);
    const live = parsed.ok ? undefined : parsed;
    return (
        <div className={cn(css.cell, isObjectList && css.wide)}>
            <ParamField
                id={`story-${definition.key}`}
                definition={definition}
                value={value}
                error={invalid ? `${definition.name} is not acceptable.` : live?.error}
                itemErrors={invalid ? [invalidRow(definition), invalidRow(definition)] : live?.items}
                onChange={(_key, next) => setValue(next)}
                timezones={TIMEZONES}
            />
        </div>
    );
}

function withErrors(definition: pb.ManifestParamDefinition): pb.ManifestParamDefinition {
    return pb.create(pb.ManifestParamDefinitionSchema, {
        ...definition,
        key: `${definition.key}Invalid`,
        name: `${definition.name}, with errors`,
    });
}

const ERROR_DEMOS = [SYMBOLS, LINKS].map(withErrors);

export function ListField({ invalid }: Args) {
    return (
        <div className={css.grid}>
            {[SYMBOLS, ...ITEM_KIND_LISTS, UNIQUE_LINKS, UNIQUE_TOGGLES, UNIQUE_BY_TOGGLE].map(definition => (
                <ListCell key={definition.key} definition={definition} invalid={invalid} />
            ))}
            {ERROR_DEMOS.map(definition => (
                <ListCell key={definition.key} definition={definition} invalid />
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
