import { describe, expect, test } from '@rstest/core';

import * as pb from '@/proto';
import { formStateToWidgetDataStruct } from '../../fn';

function paramDef(
    kindCase: pb.ManifestParamDefinition['kind']['case'],
    key = 'k',
    isOptional = false,
): pb.ManifestParamDefinition {
    let kind: pb.ManifestParamDefinition['kind'];
    switch (kindCase) {
        case 'paramString':
            kind = { case: 'paramString', value: pb.create(pb.ParamStringSchema) };
            break;
        case 'paramInteger':
            kind = { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema) };
            break;
        case 'paramDouble':
            kind = { case: 'paramDouble', value: pb.create(pb.ParamDoubleSchema) };
            break;
        case 'paramBoolean':
            kind = { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema) };
            break;
        case 'paramTimezone':
            kind = { case: 'paramTimezone', value: pb.create(pb.ParamTimezoneSchema) };
            break;
        default:
            kind = { case: 'paramString', value: pb.create(pb.ParamStringSchema) };
    }
    return pb.create(pb.ManifestParamDefinitionSchema, {
        key,
        name: 'K',
        isOptional,
        kind,
    });
}

describe('formStateToWidgetDataStruct', () => {
    const manifest = pb.create(pb.WidgetManifestSchema, {
        uid: 'widget-1',
        name: 'Widget',
        description: '',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [
            paramDef('paramString', 'name'),
            paramDef('paramInteger', 'count'),
            paramDef('paramBoolean', 'enabled'),
            pb.create(pb.ManifestParamDefinitionSchema, {
                key: 'tz',
                name: 'Timezone',
                isOptional: true,
                kind: { case: 'paramTimezone', value: pb.create(pb.ParamTimezoneSchema) },
            }),
        ],
    });

    test('keeps typed wire values as-is', () => {
        const params: Record<string, pb.WidgetDataValue> = {
            enabled: pb.create(pb.WidgetDataValueSchema, { kind: { case: 'booleanValue', value: true } }),
            tz: pb.create(pb.WidgetDataValueSchema, {
                kind: { case: 'nullValue', value: pb.create(pb.WidgetDataValue_NullSchema) },
            }),
        };
        const result = formStateToWidgetDataStruct(manifest, params);
        expect(result.fields.enabled.kind).toEqual({ case: 'booleanValue', value: true });
        expect(result.fields.tz.kind.case).toBe('nullValue');
    });

    test('drops unknown keys from form state', () => {
        const params: Record<string, pb.WidgetDataValue> = {
            unknown: pb.create(pb.WidgetDataValueSchema, { kind: { case: 'stringValue', value: 'x' } }),
        };
        const result = formStateToWidgetDataStruct(manifest, params);
        expect(result.fields.unknown).toBeUndefined();
        expect(result.fields.name.kind).toEqual({ case: 'stringValue', value: '' });
    });

    test('fills missing manifest keys with defaults/nulls', () => {
        const result = formStateToWidgetDataStruct(manifest, {});
        expect(result.fields.name.kind).toEqual({ case: 'stringValue', value: '' });
        expect(result.fields.count.kind.case).toBe('nullValue');
        expect(result.fields.enabled.kind).toEqual({ case: 'booleanValue', value: false });
        expect(result.fields.tz.kind.case).toBe('nullValue');
    });
});
