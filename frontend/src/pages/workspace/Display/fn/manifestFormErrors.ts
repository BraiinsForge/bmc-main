import * as pb from '@/proto';
import type { FormifiedParams, ParamsFormErrors } from './fn';

export function mapManifestUpdateError(rawError: unknown): ParamsFormErrors {
    const known = ['params'];
    const { global, fields } = pb.parseFormErrors<{ params: FormifiedParams }>(rawError, known);
    const paramsFieldErrors = fields.params as Maybe<Record<string, string[]>>;
    const fieldErrors: pb.FieldBasedErrors<FormifiedParams> = {};
    if (paramsFieldErrors) {
        for (const [rawKey, errs] of Object.entries(paramsFieldErrors)) {
            const key = rawKey.replaceAll('"', '').replaceAll("'", '');
            (fieldErrors as Record<string, string[]>)[key] = errs;
        }
    }
    return { global, fields: fieldErrors };
}
