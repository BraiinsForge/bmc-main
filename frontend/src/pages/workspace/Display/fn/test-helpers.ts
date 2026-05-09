import * as pb from '@/proto';
import { assertUnreachable } from '@/lib/ts';

export function paramDef(
    kindCase: pb.ManifestParamDefinition['kind']['case'],
    key = 'k',
    isOptional = false,
    overrides: Record<string, any> = {},
): pb.ManifestParamDefinition {
    let kind: pb.ManifestParamDefinition['kind'];
    switch (kindCase) {
        case 'paramString':
            kind = { case: 'paramString', value: pb.create(pb.ParamStringSchema, overrides) };
            break;
        case 'paramInteger':
            kind = { case: 'paramInteger', value: pb.create(pb.ParamIntegerSchema, overrides) };
            break;
        case 'paramDouble':
            kind = { case: 'paramDouble', value: pb.create(pb.ParamDoubleSchema, overrides) };
            break;
        case 'paramBoolean':
            kind = { case: 'paramBoolean', value: pb.create(pb.ParamBooleanSchema, overrides) };
            break;
        case 'paramTimezone':
            kind = { case: 'paramTimezone', value: pb.create(pb.ParamTimezoneSchema, overrides) };
            break;
        case undefined:
            kind = { case: undefined };
            break;
        default:
            assertUnreachable(kindCase, 'paramDef kind');
    }
    return pb.create(pb.ManifestParamDefinitionSchema, { key, name: 'K', isOptional, kind });
}
