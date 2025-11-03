/**
 * @see https://gitlab.ii.zone/braiins-forge/deck/deckfeeder/-/blob/master/assets/sdk/v1/schema.json?ref_type=heads#L61
 * @see https://transform.tools/json-schema-to-typescript
 */

export type Param<T extends SchemaAny = SchemaAny> = T & {
    name: string;
    description?: string;
};

export interface SchemaString {
    type: 'string';
    default?: string;
    enum?: string[];
    const?: string;
    format?: 'date-time' | 'date' | 'time' | 'email' | 'hostname' | 'ipv4' | 'ipv6' | 'uri' | 'uuid';
    pattern?: string;
    minLength?: number;
    maxLength?: number;
}
export interface SchemaNumber {
    type: 'number';
    default?: number;
    enum?: [number, ...number[]];
    const?: number;
    minimum?: number;
    maximum?: number;
    exclusiveMinimum?: number;
    exclusiveMaximum?: number;
    multipleOf?: number;
}
export interface SchemaInteger {
    type: 'integer';
    default?: number;
    enum?: [number, ...number[]];
    const?: number;
    minimum?: number;
    maximum?: number;
    exclusiveMinimum?: number;
    exclusiveMaximum?: number;
    multipleOf?: number;
}
export interface SchemaBoolean {
    type: 'boolean';
    default?: boolean;
    const?: boolean;
}
export interface SchemaArray {
    type: 'array';
    items: SchemaPrimitive;
    default?: unknown[];
    minItems?: number;
    maxItems?: number;
    uniqueItems?: boolean;
}

export type SchemaPrimitive = SchemaString | SchemaNumber | SchemaInteger | SchemaBoolean;
export type SchemaAny = SchemaPrimitive | SchemaArray;
