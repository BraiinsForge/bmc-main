import { Component, useCallback } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

// Lib
import { toast } from '@/lib/toast';
import { assertUnreachable } from '@/lib/ts';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// App
import * as pb from '@/proto';
import { getID } from '../const';

// Components
import { TextInput, NumberInput, Select, SelectItem } from '@carbon/react';
import { TrashCan as IconDelete, AddFilled as IconAdd } from '@carbon/react/icons';
import { ModalCustom, InlineNotification, Checkbox, CarbonFormField, Button } from '@/components';
import {
    WidgetSizeSelector,
    type WidgetSizeSelectorProps,
    CheckYourScreenForPreview,
    WidgetHasNoParametersToConfigure,
} from '../shared';

// styles, types
import type * as t from './types';
import cssShared from '../shared.scss';
import css from './FormWidgetRemoteWidget.scss';

export interface FormWidgetRemoteWidgetProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    // It does not make sense to have those fields as `iField`
    // purely as matter of runtime, but it makes our life easier
    // by being consistent with the rest of the widget components.
    url: iField<string>;
    name: iField<string>;
    params: iField<pb.JsonObject>;

    // Likely only usefull for storybook,
    // to provide the params statically.
    paramsSchema?: Record<string, t.Param>;

    style?: CSSProperties;
}
interface Props extends FormWidgetRemoteWidgetProps {
    intl: IntlShape;
}

interface State {
    params: Record<string, t.Param>;
    values: pb.JsonObject;
}
const getInitialState = (): State => ({
    params: {},
    values: {},
});

class View extends Component<Props, State> {
    readonly state = getInitialState();
    #txt = {
        addWidget: this.props.intl.formatMessage({ defaultMessage: 'Add Widget' }),
        editWidget: this.props.intl.formatMessage({ defaultMessage: 'Edit Widget' }),
    };

    #id = (...suffix: Array<string | number>): string => {
        return getID('remote-widget-form', this.props.name.value?.toLowerCase() || '').get(...suffix);
    };

    componentDidUpdate(prevProps: Props) {
        const { url, isOpen } = this.props;
        if (prevProps.url.value !== url.value || (isOpen && !prevProps.isOpen)) this.#fetchParamsAndPopulateState();
    }
    componentDidMount() {
        const { isOpen } = this.props;
        if (isOpen) this.#fetchParamsAndPopulateState();
    }
    componentWillUnmount() {
        pb.abort.all(this);
    }

    private abortFetchParams = pb.abort.get();
    #fetchParamsAndPopulateState = async (): Promise<unknown> => {
        const { url, intl, paramsSchema, isOpen, params } = this.props;
        const { formatMessage } = intl;

        if (!isOpen || !url.value) return;

        const newState = getInitialState();

        // Static params are provided locally (storybook)
        if (paramsSchema) {
            newState.params = paramsSchema;
        }
        // We have to fetch them from the server
        else {
            try {
                const { signal } = this.abortFetchParams.replace();
                const res = await pb.rpc.scenes.getRemoteWidgetParams({ widgetUrl: url.value }, { signal });
                newState.params = res.remoteWidgetParams as unknown as State['params'];
            } catch ($) {
                if (pb.abort.is($)) return;
                let msg = pb.collectAllErrorsAsFormattedList($);
                msg ||= formatMessage({ defaultMessage: 'Failed to fetch widget parameters' });
                toast.error(msg);
            }
        }

        // Pre-fill values from parent component.
        // This is where the upstream state gets merged into local state.
        newState.values = params.value || {};

        // And set anything not already set to default value.
        // This would not matter for text fields where we render defaults
        // as placeholders, but for `array` type we have to set the value
        // to let user work from a predefined base state.
        for (const [key, val] of Object.entries(newState.params)) {
            // Abort if the value is already set
            // or the field does not define default
            if (val.default == null || newState.values[key] != null) continue;
            newState.values[key] = val.default as pb.JsonValue;
        }

        this.setState(newState);
    };

    /** Push the values from our local state into parent component */
    #valuesPush = (): void => {
        const { params } = this.props;
        const { values } = this.state;
        params.onChange?.(values);
    };

    #handleFieldChange = (key: string, value: pb.JsonValue) => {
        this.setState(s => ({ values: { ...s.values, [key]: value } }));
    };

    render() {
        const {
            // Modal & state
            isOpen,
            isEdit,
            onClose,
            error,

            // Main
            widgetSize,
            name,

            intl,
            style,
        } = this.props;
        const { params, values } = this.state;
        const { formatMessage } = intl;

        let paramsForm: ReactNode = null;
        const paramsEntries = Object.entries(params);
        if (paramsEntries.length) {
            paramsForm = paramsEntries.map(([key, x]) => (
                <Field
                    key={key}
                    id={this.#id(key)}
                    name={x.name}
                    description={x.description}
                    schema={x}
                    // @ts-expect-error: No real chance of making this type-safe statically
                    value={values[key]}
                    onChange={val => this.#handleFieldChange(key, val as pb.JsonValue)}
                    onCommit={this.#valuesPush}
                />
            ));
        } else {
            paramsForm = <WidgetHasNoParametersToConfigure />;
        }

        const verb = isEdit ? this.#txt.editWidget : this.#txt.addWidget;
        const form = (
            <Form className={cssShared.form} style={style}>
                <WidgetSizeSelector field={widgetSize} />

                <section className={css.paramsContainer} children={paramsForm} />

                <CheckYourScreenForPreview />

                {error ? (
                    <InlineNotification
                        kind="error"
                        theme="inverse"
                        stretch
                        hideCloseButton
                        title={formatMessage({ defaultMessage: 'Error' })}
                        children={error}
                    />
                ) : null}
            </Form>
        );

        return (
            <ModalCustom
                id={this.#id('dialog')}
                className={cssShared.modal}
                cancelBodyOverflowShadow
                selectorPrimaryFocus="form"
                // State
                size="sm"
                open={isOpen}
                // Heading
                title={formatMessage({ defaultMessage: 'Remote Widget: {name}' }, { name: name.value })}
                label={verb}
                // Cancel
                onClose={onClose}
                // Content
                children={form}
                footer={
                    <Button
                        id={this.#id('done')}
                        kind="primary"
                        children={formatMessage({ defaultMessage: 'Done' })}
                        onClick={onClose}
                    />
                }
            />
        );
    }
}
export function FormWidgetRemoteWidget(props: FormWidgetRemoteWidgetProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}

interface FieldProps<F extends t.SchemaAny> {
    id: string;
    name: Maybe<string>;
    description: Maybe<string>;

    schema: F;
    value: F['default'];
    // Change handler can run on every keystroke,
    // so we need to have a separate handler
    // for commit of the whole config
    onChange(value: F['default']): void;
    onCommit(): void;
}
function Field<F extends t.SchemaAny>(props: FieldProps<F>) {
    const { id, name, description, schema: x, value, onChange, onCommit } = props;
    const { formatMessage } = useIntl();

    const changeAndCommit = useCallback(
        (v: F['default']) => {
            onChange(v);
            setTimeout(onCommit, 100);
        },
        [onChange, onCommit],
    );

    const placeholder: undefined | string = x.default != null ? String(x.default) : undefined;
    switch (x.type) {
        case 'string': {
            if (x.enum) {
                let $value = typeof value === 'string' || typeof value === 'number' ? value : null;
                if (x.default != null && $value == null) $value = x.default;

                return (
                    <BoundSelect
                        id={id}
                        label={name ?? undefined}
                        helperText={description ?? undefined}
                        options={x.enum}
                        value={$value}
                        onChange={changeAndCommit}
                    />
                );
            }

            let type: string = 'string';
            switch (x.format) {
                case 'date-time':
                case 'date':
                case 'time':
                case 'email':
                    type = x.format;
            }

            return (
                <TextInput
                    id={id}
                    type={type}
                    labelText={name}
                    helperText={description}
                    placeholder={placeholder}
                    value={value != null ? String(value) : ''}
                    onChange={e => onChange(e.target.value)}
                    onBlur={onCommit}
                />
            );
        }

        case 'number':
        case 'integer':
            if (x.enum) {
                let $value = typeof value === 'string' || typeof value === 'number' ? value : null;
                if (x.default != null && $value == null) $value = x.default;

                return (
                    <BoundSelect
                        id={id}
                        label={name}
                        helperText={description}
                        options={x.enum}
                        value={$value}
                        onChange={changeAndCommit}
                    />
                );
            }

            return (
                <NumberInput
                    id={id}
                    allowEmpty
                    helperText={description}
                    type="number"
                    label={name}
                    min={x.minimum}
                    max={x.maximum}
                    step={x.multipleOf ?? (x.type === 'integer' ? 1 : undefined)}
                    value={value != null ? String(value) : ''}
                    placeholder={placeholder}
                    onChange={(_, { value }) => onChange(value)}
                    onBlur={onCommit}
                />
            );

        case 'boolean':
            return (
                <Checkbox
                    id={id}
                    label={name}
                    checked={!!(value ?? x.default)}
                    helperText={description}
                    onChange={e => changeAndCommit(e.target.checked)}
                />
            );

        case 'array': {
            const minItems = x.minItems || 0;
            const maxItems = x.maxItems || Number.MAX_VALUE;

            type T = string | number | undefined;
            const valueArray = (Array.isArray(value) ? value : []) as T[];
            if (minItems >= 1 && valueArray.length < minItems) {
                const padding: undefined[] = new Array(minItems - valueArray.length).fill(undefined);
                valueArray.push(...padding);
            }
            const handleAdd = () => changeAndCommit([...valueArray, undefined]);

            const canAdd: boolean = valueArray.length < maxItems;

            return (
                <CarbonFormField labelText={name} helperText={description} className={css.arrayContainer}>
                    {valueArray.map((val, ind) => {
                        const handleChange = ($value: T): void => {
                            onChange(valueArray.map((x, i) => (i === ind ? $value : x)));
                        };
                        const handleRemove = () => {
                            changeAndCommit(valueArray.filter((_, i) => i !== ind));
                        };
                        const itemID: string = `${id}-${ind}`;

                        return (
                            <div key={ind} className={css.arrayFieldContainer}>
                                <Field
                                    id={`${itemID}-field`}
                                    name={null}
                                    description={null}
                                    schema={x.items}
                                    value={val}
                                    onChange={handleChange}
                                    onCommit={onCommit}
                                />
                                <Button
                                    id={`${itemID}-remove`}
                                    kind="ghost"
                                    size="md"
                                    title={formatMessage({ defaultMessage: 'Remove Value' })}
                                    icon={IconDelete}
                                    hasIconOnly
                                    tooltipPosition="left"
                                    onClick={handleRemove}
                                    // Don't allow deleting of placeholders that are inserted when `minItems` exists.
                                    // Selected/filled-in values still can be removed.
                                    disabled={val == null || val === ''}
                                />
                            </div>
                        );
                    })}

                    {canAdd && (
                        <Button
                            id={`${id}-add-button`}
                            kind="tertiary"
                            size="md"
                            icon={IconAdd}
                            tooltipPosition="left"
                            children={formatMessage({ defaultMessage: 'Add Value' })}
                            className={css.addButton}
                            onClick={handleAdd}
                        />
                    )}
                </CarbonFormField>
            );
        }

        default:
            assertUnreachable(x, 'Unknown field type');
    }
}

interface BoundSelectProps {
    id: string;
    label: Maybe<string>;
    helperText: Maybe<string>;
    options: t.Enum<string | number>;
    value: Maybe<string | number>;
    onChange(value: string | number): void;
}
function BoundSelect(props: BoundSelectProps) {
    const { id, label, helperText, options, value, onChange } = props;
    const { formatMessage } = useIntl();
    type T = string | number;

    const optionsArray = Array.isArray(options)
        ? options.map(v => ({ value: v, label: String(v) }))
        : Object.entries(options).map(([value, label]) => ({ value, label }));

    return (
        <Select
            id={id}
            noLabel={!label}
            labelText={label}
            helperText={helperText}
            value={value as T}
            onChange={e => onChange(e.target.value)}
        >
            <SelectItem value="" text={formatMessage({ defaultMessage: '-- Select an option --' })} />
            {optionsArray.map((opt, i) => (
                <SelectItem key={i} value={opt.value} text={opt.label} />
            ))}
        </Select>
    );
}

export function createRemoteWidgetKind(data: FormPropsToValuesRec<FormWidgetRemoteWidgetProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'remoteWidget',
            value: pb.create(pb.RemoteWidgetSchema, {
                widgetUrl: data.url,
                name: data.name,
                params: data.params,
            }),
        },
    });
}
export function unpackRemoteWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetRemoteWidgetProps> {
    if (data.value?.case !== 'remoteWidget') throw new Error('Invalid widget kind');
    return {
        widgetSize,
        url: data.value.value.widgetUrl,
        name: data.value.value.name,
        params: data.value.value.params,
    };
}
