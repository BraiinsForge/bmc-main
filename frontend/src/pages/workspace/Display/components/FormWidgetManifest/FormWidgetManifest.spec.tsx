import { describe, expect, test } from '@rstest/core';

import * as pb from '@/proto';
import { widgetDataValueFromRaw } from './FormWidgetManifest';

describe('widgetDataValueFromRaw', () => {
    test('decodes a JSON string to stringValue', () => {
        const kind: pb.ManifestParamDefinition['kind'] = {
            case: 'paramString',
            value: pb.create(pb.ParamStringSchema),
        };
        const v = widgetDataValueFromRaw('"hello"', kind);
        expect(v.kind).toEqual({ case: 'stringValue', value: 'hello' });
    });

    test('decodes a JSON number to integerValue', () => {
        const kind: pb.ManifestParamDefinition['kind'] = {
            case: 'paramInteger',
            value: pb.create(pb.ParamIntegerSchema),
        };
        const v = widgetDataValueFromRaw('42', kind);
        expect(v.kind).toEqual({ case: 'integerValue', value: 42 });
    });

    test('decodes null / empty number to nullValue', () => {
        const kind: pb.ManifestParamDefinition['kind'] = {
            case: 'paramInteger',
            value: pb.create(pb.ParamIntegerSchema),
        };
        expect(widgetDataValueFromRaw('null', kind).kind.case).toBe('nullValue');
        expect(widgetDataValueFromRaw('', kind).kind.case).toBe('nullValue');
    });

    test('decodes boolean string to booleanValue', () => {
        const kind: pb.ManifestParamDefinition['kind'] = {
            case: 'paramBoolean',
            value: pb.create(pb.ParamBooleanSchema),
        };
        expect(widgetDataValueFromRaw('true', kind).kind).toEqual({ case: 'booleanValue', value: true });
        expect(widgetDataValueFromRaw('false', kind).kind).toEqual({ case: 'booleanValue', value: false });
    });
});
