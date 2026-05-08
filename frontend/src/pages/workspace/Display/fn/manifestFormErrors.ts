import * as pb from '@/proto';

export interface MappedManifestUpdateError {
    fieldErrors: Record<string, string>;
    error: string | null;
}

export function mapManifestUpdateError(rawError: unknown): MappedManifestUpdateError {
    const known = ['params'];
    const { global, fields } = pb.parseFormErrors(rawError, known);
    const paramsFieldErrors = fields.params as Maybe<Record<string, string[]>>;
    const fieldErrors: Record<string, string> = {};
    if (paramsFieldErrors) {
        for (const [rawKey, errs] of Object.entries(paramsFieldErrors)) {
            const key = rawKey.replaceAll('"', '').replaceAll("'", '');
            const msg = pb.renderFieldErrorsAsList(errs);
            if (msg) fieldErrors[key] = msg;
        }
    }
    const error = pb.renderFieldErrorsAsList(global);
    return { fieldErrors, error };
}
