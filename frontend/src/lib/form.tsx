import { type ComponentType, type DependencyList, type FormHTMLAttributes, type Ref, useMemo } from 'react';
import { blockEvent } from '@/lib/react';

export type iForm<SubmitData = void> = {
    saving?: boolean;
    disabled?: boolean;

    onSubmit: SubmitData extends void ? () => void : (data: SubmitData) => void;
    submitDisabled?: boolean;
    error?: null | string;
};

export type iField<T, ErrorType = string | ReactElement, ExtraProps extends Rec = Rec> = ExtraProps & {
    value: null | T;
    error?: null | ErrorType;
    onChange(value: T): void;
    disabled?: boolean;
};

export interface iFieldEnumOption<T> {
    value: T;
    label: string;
    icon?: ComponentType<{ className?: string }>;
    preview?: string;
}
export interface iFieldEnum<T> extends iField<T> {
    options: Array<iFieldEnumOption<T>>;
}
export interface iFieldNumber<T extends number> extends iField<T> {
    min?: T;
    max?: T;
    step?: T;
}

export function isDisabled(form: Partial<iForm>, field?: iField<any>): boolean {
    if (field?.disabled === true) return true;
    return !!form.saving || !!form.disabled;
}

export interface FormProps extends FormHTMLAttributes<HTMLFormElement> {
    $ref?: Ref<HTMLFormElement>;
    children: NonNullable<ReactNode>;
}
export function Form(props: FormProps) {
    const { $ref, ...rest } = props;
    return <form onSubmit={blockEvent} autoComplete="off" lang="g!auld" {...rest} ref={$ref} />;
}

export interface iFormErrors<FieldName extends PropertyKey = string, FieldErrorType = string> {
    global?: string[];
    fields?: Partial<Record<FieldName, null | FieldErrorType>>;
}
export function hasFormErrors<T extends iFormErrors<string, any>>(errors: Maybe<T>): boolean {
    return errors?.global?.some(Boolean) || Object.values(errors?.fields || {}).some(Boolean);
}

/**
 * Convert a collection (either interface or record)
 * of iField types to a shallow record of the value types.
 *
 * @example
 * interface FormProps {
 *     name: iField<string>;
 *     age: iField<number>;
 * }
 *
 * type FormPropsToValues = FormPropsToValuesRec<FormProps>;
 * // { name: string; age: number }
 */
export type FormPropsToValuesRec<FormProps> = {
    // We often use null to signify that a field can be explicitly omited from the form,
    // but that causes problems here and in derived types further down the road.
    // Therefore we will strip the `null` and just leave it at optional key.
    [K in keyof FormProps]?: NonNullable<FormProps[K]> extends iField<infer T> ? T : never;
};
/**
 * Convert a collection (either interface or record) of iField types
 * to a shape usable for local state of a component that communicates
 * with APIs and fills in the original form props.
 *
 * @example
 * interface FormProps {
 *     name: iField<string>;
 *     age: iField<number>;
 * }
 *
 * type FormState = FormPropsToLocalState<FormProps>;
 * // {
 * //   values: {
 * //       name: string;
 * //       age: number;
 * //   };
 * //   errors: {
 * //       global: string[];
 * //       fields: {
 * //           name: string[];
 * //           age: string[];
 * //       };
 * //   };
 * // };
 */
export type FormPropsToLocalState<FormProps> = {
    values: FormPropsToValuesRec<FormProps>;
    errors: null | iFormErrors<keyof FormPropsToValuesRec<FormProps>, string[]>;
};

type IdPath = Array<string | number>;
class GetID {
    #preffix: IdPath;

    constructor(...preffix: IdPath) {
        this.#preffix = preffix;
    }
    at = (...prefix: IdPath): GetID => {
        return new GetID(...this.#preffix, ...prefix);
    };
    get = (...suffix: IdPath): string => {
        return [...this.#preffix, ...suffix].join('-');
    };
}
export const getID = new GetID('bmc').at;
export function useID(...prefix: IdPath) {
    // biome-ignore lint/correctness/useExhaustiveDependencies: It is OK, but the check is kind of dumb
    return useMemo(() => getID(...prefix).get, prefix as DependencyList);
}
