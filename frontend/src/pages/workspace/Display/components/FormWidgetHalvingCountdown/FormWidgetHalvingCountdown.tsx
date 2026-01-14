import { useIntl } from 'react-intl';

import * as pb from '@/proto';
import { getID } from '../const';
import { Form, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { ModalCustom, InlineNotification, Button } from '@/components';
import { WidgetSizeSelector, type WidgetSizeSelectorProps, CheckYourScreenForPreview } from '../shared';

// styles
import css from '../shared.scss';

const $ = getID('halving-countdown-form').get;

export interface FormWidgetHalvingCountdownProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    style?: CSSProperties;
}

export function FormWidgetHalvingCountdown(props: FormWidgetHalvingCountdownProps) {
    const intl = useIntl();
    const { formatMessage } = intl;

    const txt = {
        halvingCountdown: formatMessage({ defaultMessage: 'Halving Countdown' }),
        addWidget: formatMessage({ defaultMessage: 'Add Widget' }),
        editWidget: formatMessage({ defaultMessage: 'Edit Widget' }),
    };

    const {
        isOpen,
        isEdit,
        onClose,
        error,

        // Main
        widgetSize,

        style,
    } = props;

    const form = (
        <Form className={css.form} style={style}>
            <WidgetSizeSelector field={widgetSize} />

            <p
                className={css.note}
                children={intl.formatMessage({
                    defaultMessage:
                        'Displays a countdown to the next Bitcoin halving event. The countdown updates in real-time based on the current block height.',
                })}
            />

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

    const verb = isEdit ? txt.editWidget : txt.addWidget;

    return (
        <ModalCustom
            id={$('dialog')}
            className={css.modal}
            selectorPrimaryFocus="button"
            // State
            size="sm"
            open={isOpen}
            // Heading
            title={txt.halvingCountdown}
            label={verb}
            // Cancel
            onClose={onClose}
            // Content
            children={form}
            footer={
                <Button
                    id={$('done')}
                    kind="primary"
                    children={formatMessage({ defaultMessage: 'Done' })}
                    onClick={onClose}
                />
            }
        />
    );
}

export function createHalvingCountdownWidgetKind(
    _data: FormPropsToValuesRec<FormWidgetHalvingCountdownProps>,
): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'halvingCountdown',
            value: pb.create(pb.HalvingCountdownWidgetSchema, {}),
        },
    });
}

export function unpackHalvingCountdownWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetHalvingCountdownProps> {
    if (data.value?.case !== 'halvingCountdown') throw new Error('Invalid widget kind');

    return {
        widgetSize,
    };
}
