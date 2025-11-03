import { useIntl } from 'react-intl';

import * as pb from '@/proto';
import { getID } from '../const';
import { Form, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { ModalCustom, InlineNotification, Button } from '@/components';
import { WidgetSizeSelector, type WidgetSizeSelectorProps, CheckYourScreenForPreview } from '../shared';

// styles
import css from '../shared.scss';

const $ = getID('block-height-form').get;

export interface FormWidgetBlockchainDataProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    style?: CSSProperties;
}
export function FormWidgetBlockchainData(props: FormWidgetBlockchainDataProps) {
    const intl = useIntl();
    const { formatMessage } = intl;

    const txt = {
        blockHeight: formatMessage({ defaultMessage: 'Bitcoin Mining Data' }),
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
                    defaultMessage: 'There are no configuration options for this widget.',
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
            title={txt.blockHeight}
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

export function createBlockchainDataWidgetKind(_: FormPropsToValuesRec<FormWidgetBlockchainDataProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'blockchainData',
            value: pb.create(pb.BlockchainDataWidgetSchema),
        },
    });
}
export function unpackBlockchainDataWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetBlockchainDataProps> {
    if (data.value?.case !== 'blockchainData') throw new Error('Invalid widget kind');
    return { widgetSize };
}
