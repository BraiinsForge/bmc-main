import { useMemo } from 'react';
import { useIntl } from 'react-intl';

import * as pb from '@/proto';
import { Form, type iField, getID, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { ModalCustom, InlineNotification } from '@/components';
import {
    BoundToggle,
    WidgetSizeSelector,
    type WidgetSizeSelectorProps,
    CheckYourScreenForPreview,
    BoundRadioGroup,
} from '../shared';

// styles
import css from '../shared.scss';

const $ = getID('settings', 'blockHeight', 'scene').get;

export interface FormWidgetBlockHeightProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    fontStyle: iField<pb.FontStyle>;
    showDate: iField<boolean>;

    style?: CSSProperties;
}

export function FormWidgetBlockHeight(props: FormWidgetBlockHeightProps) {
    const intl = useIntl();
    const { formatMessage } = intl;

    const fontStyles = useMemo(() => {
        return pb.fontStyleOptions.map(x => ({
            value: x,
            label: String(pb.fontStyleToString(intl, x)),
        }));
    }, [intl]);
    const txt = {
        blockHeight: formatMessage({ defaultMessage: 'Block Height' }),
        addScene: formatMessage({ defaultMessage: 'Add Scene' }),
        editScene: formatMessage({ defaultMessage: 'Edit Scene' }),
    };

    const {
        isOpen,
        isEdit,
        onClose,
        error,

        // Main
        widgetSize,
        fontStyle,
        showDate,

        style,
    } = props;

    const form = (
        <Form className={css.form} style={style}>
            <WidgetSizeSelector field={widgetSize} />

            <BoundRadioGroup
                {...fontStyle}
                id={$('font')}
                labelText={formatMessage({ defaultMessage: 'Numbers Font Style' })}
                items={fontStyles}
            />

            <BoundToggle
                {...showDate}
                id={$('show-date')}
                labelText={formatMessage({ defaultMessage: 'Show Time and Date' })}
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

    const verb = isEdit ? txt.editScene : txt.addScene;

    return (
        <ModalCustom
            id={$('dialog')}
            className={css.modal}
            selectorPrimaryFocus="input"
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
        />
    );
}

export function createBlockHeightWidgetKind(data: FormPropsToValuesRec<FormWidgetBlockHeightProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'blockHeight',
            value: pb.create(pb.BlockHeightWidgetSchema, {
                showTimestamp: data.showDate,
                numbersFontStyle: data.fontStyle,
            }),
        },
    });
}
