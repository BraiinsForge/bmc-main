import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

import * as pb from '@/proto';
import { getID } from '../const';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// Components
import {
    WidgetSizeSelector,
    type WidgetSizeSelectorProps,
    BoundRadioGroup,
    BoundCheckbox,
    BoundComboBox,
    type OptionItem,
    CheckYourScreenForPreview,
} from '../shared';
import { ModalCustom, InlineNotification } from '@/components';
import { CheckboxGroup } from '@carbon/react';
import { Earth as IconEarth } from '@carbon/react/icons';

// styles
import css from '../shared.scss';

const $ = getID('clock-form').get;

export interface FormWidgetClockProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    clockStyle: iField<pb.ClockWidget_ClockStyle>;
    fontStyle: iField<pb.FontStyle>;

    showDate: iField<boolean>;
    showSeconds: iField<boolean>;

    showTimezone: iField<boolean>;
    timezone: iField<string> & { options: pb.Timezone[] };

    // showWeather: iField<boolean>;
    // weatherLocation: iField<string>;

    style?: CSSProperties;
}
interface Props extends FormWidgetClockProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    get #clockStyleOptions(): Array<OptionItem<pb.ClockWidget_ClockStyle>> {
        const { formatMessage } = this.props.intl;
        return [
            { value: pb.ClockWidget_ClockStyle.ANALOG_ROUND, label: formatMessage({ defaultMessage: 'Analog 1' }) },
            { value: pb.ClockWidget_ClockStyle.ANALOG_RECT, label: formatMessage({ defaultMessage: 'Analog 2' }) },
            { value: pb.ClockWidget_ClockStyle.DIGITAL, label: formatMessage({ defaultMessage: 'Digital 1' }) },
        ];
    }
    get #fontStyleOptions(): Array<OptionItem<pb.FontStyle>> {
        const { formatMessage } = this.props.intl;
        return [
            { value: pb.FontStyle.LIGHT, label: formatMessage({ defaultMessage: 'Light' }) },
            { value: pb.FontStyle.MEDIUM, label: formatMessage({ defaultMessage: 'Medium' }) },
            { value: pb.FontStyle.BOLD, label: formatMessage({ defaultMessage: 'Bold' }) },
        ];
    }

    #txt = {
        clock: this.props.intl.formatMessage({ defaultMessage: 'Clock' }),
        addScene: this.props.intl.formatMessage({ defaultMessage: 'Add Scene' }),
        editScene: this.props.intl.formatMessage({ defaultMessage: 'Edit Scene' }),
    };

    render() {
        const {
            isOpen,
            isEdit,
            onClose,
            error,

            // Main
            widgetSize,
            clockStyle,
            fontStyle,

            // Additional
            showDate,
            showSeconds,

            showTimezone,
            timezone,

            // showWeather,
            // weatherLocation,

            style,
            intl: { formatMessage },
        } = this.props;

        const form = (
            <Form className={css.form} style={style}>
                <WidgetSizeSelector field={widgetSize} />

                <BoundRadioGroup
                    {...clockStyle}
                    id={$('style')}
                    labelText={formatMessage({ defaultMessage: 'Clock Style' })}
                    items={this.#clockStyleOptions}
                />
                <BoundRadioGroup
                    {...fontStyle}
                    id={$('font')}
                    labelText={formatMessage({ defaultMessage: 'Numbers Font Style' })}
                    items={this.#fontStyleOptions}
                />

                <CheckboxGroup legendText={formatMessage({ defaultMessage: 'Additional Options' })}>
                    <BoundCheckbox
                        {...showDate}
                        id={$('show-date')}
                        labelText={formatMessage({ defaultMessage: 'Show Date' })}
                    />
                    <BoundCheckbox
                        {...showSeconds}
                        id={$('show-seconds')}
                        labelText={formatMessage({ defaultMessage: 'Show Seconds' })}
                    />
                    <BoundCheckbox
                        {...showTimezone}
                        id={$('show-timezone')}
                        labelText={formatMessage({ defaultMessage: 'Show Timezone' })}
                    />
                    {/* <BoundCheckbox {...showWeather} idSuffix="show-weather" labelText={formatMessage({ defaultMessage: 'Show Weather' })} /> */}
                </CheckboxGroup>

                {showTimezone.value === true ? (
                    <BoundComboBox<string>
                        id={$('timezone')}
                        {...timezone}
                        items={timezone.options.map(x => ({ value: x.id, label: `${x.offset} ${x.label}` }))}
                        labelText={formatMessage({ defaultMessage: 'Timezone' })}
                        decorator={<IconEarth size={20} />}
                        helperText={formatMessage({ defaultMessage: 'Location is used for Timezone.' })}
                    />
                ) : null}

                {/* showWeather.value === true ? (
                    <BoundComboBox
                        idSuffix="location"
                        {...weatherLocation}
                        items={[{ label: 'Location', value: 'Location' }]}
                        labelText={formatMessage({ defaultMessage: 'Weather Location' })}
                        decorator={<IconLocation size={20} />}
                    />
                ) : null */}

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

        const verb = isEdit ? this.#txt.editScene : this.#txt.addScene;

        return (
            <ModalCustom
                id={$('dialog')}
                className={css.modal}
                selectorPrimaryFocus="form input,button"
                // State
                size="sm"
                open={isOpen}
                // Heading
                title={this.#txt.clock}
                label={verb}
                // Cancel
                onClose={onClose}
                // Content
                children={form}
            />
        );
    }
}
export function FormWidgetClock(props: FormWidgetClockProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}

export function createClockWidgetKind(data: FormPropsToValuesRec<FormWidgetClockProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'clock',
            value: pb.create(pb.ClockWidgetSchema, {
                clockStyle: data.clockStyle,
                numbersFontStyle: data.fontStyle,
                showDate: data.showDate,
                showSeconds: data.showSeconds,

                showTimezone: data.showTimezone,
                timezone: data.timezone,
            }),
        },
    });
}
export function unpackClockWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetClockProps> {
    if (data.value.case !== 'clock') throw new Error('Invalid widget kind');
    const clock = data.value.value;

    return {
        widgetSize,
        clockStyle: clock.clockStyle,
        fontStyle: clock.numbersFontStyle,
        showDate: clock.showDate,
        showSeconds: clock.showSeconds,
        showTimezone: clock.showTimezone,
        timezone: clock.timezone,
    };
}
