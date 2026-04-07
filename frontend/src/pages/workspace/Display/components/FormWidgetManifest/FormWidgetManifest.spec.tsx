import { describe, expect, test } from '@rstest/core';

import { encodeNumberEnumParamValue, encodeNumberParamValue, getNumberInputValue } from './FormWidgetManifest';

describe('getNumberInputValue', () => {
    test('returns empty value for non-numeric defaults', () => {
        expect(getNumberInputValue('null')).toEqual('');
        expect(getNumberInputValue('""')).toEqual('');
    });
});

describe('encodeNumberParamValue', () => {
    test('returns json number for numeric input', () => {
        expect(encodeNumberParamValue('42')).toEqual('42');
    });

    test('returns explicit json null for empty input', () => {
        expect(encodeNumberParamValue('')).toBe('null');
        expect(encodeNumberParamValue(null)).toBe('null');
    });
});

describe('encodeNumberEnumParamValue', () => {
    test('returns explicit json null for non-numeric enum key', () => {
        expect(encodeNumberEnumParamValue('on')).toBe('null');
    });
});
