import { describe, expect, test } from '@rstest/core';

import * as pb from '@/proto';
import { widgetDataValueFromRaw } from './FormWidgetManifest';

function paramDef(
    kindCase: pb.ManifestParamDefinition['kind']['case'],
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
        key: 'k',
        name: 'K',
        isOptional,
        kind,
    });
}

describe('widgetDataValueFromRaw', () => {
    test('decodes a JSON string to stringValue', () => {
        const v = widgetDataValueFromRaw('"hello"', paramDef('paramString'));
        expect(v.kind).toEqual({ case: 'stringValue', value: 'hello' });
    });

    test('decodes a JSON number to integerValue', () => {
        const v = widgetDataValueFromRaw('42', paramDef('paramInteger'));
        expect(v.kind).toEqual({ case: 'integerValue', value: 42 });
    });

    test('decodes null / empty number to nullValue', () => {
        const def = paramDef('paramInteger');
        expect(widgetDataValueFromRaw('null', def).kind.case).toBe('nullValue');
        expect(widgetDataValueFromRaw('', def).kind.case).toBe('nullValue');
    });

    test('decodes boolean string to booleanValue', () => {
        const def = paramDef('paramBoolean');
        expect(widgetDataValueFromRaw('true', def).kind).toEqual({ case: 'booleanValue', value: true });
        expect(widgetDataValueFromRaw('false', def).kind).toEqual({ case: 'booleanValue', value: false });
    });

    test('optional string with empty raw round-trips as nullValue', () => {
        expect(widgetDataValueFromRaw('', paramDef('paramString', true)).kind.case).toBe('nullValue');
        expect(widgetDataValueFromRaw('null', paramDef('paramString', true)).kind.case).toBe('nullValue');
    });

    test('required string with empty raw stays as empty stringValue', () => {
        const v = widgetDataValueFromRaw('""', paramDef('paramString', false));
        expect(v.kind).toEqual({ case: 'stringValue', value: '' });
    });

    test('optional timezone with empty raw round-trips as nullValue', () => {
        expect(widgetDataValueFromRaw('', paramDef('paramTimezone', true)).kind.case).toBe('nullValue');
    });
});
