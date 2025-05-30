import type { ComponentType, FormHTMLAttributes, Ref } from 'react';
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

export interface iFormErrors<FieldName extends Key = string, FieldErrorType = string> {
    global?: string[];
    fields?: Partial<Record<FieldName, null | FieldErrorType>>;
}
