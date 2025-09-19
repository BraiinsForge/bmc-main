import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

import * as pb from '@/proto';
import { getID } from '../const';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { WidgetSizeSelector, type WidgetSizeSelectorProps, CheckYourScreenForPreview } from '../shared';
import { ModalCustom, InlineNotification } from '@/components';
import { Dropdown } from '@carbon/react';

// styles
import css from '../shared.scss';

const $ = getID('ticker-form').get;

export interface FormWidgetTickerProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];
    timeFrame: iField<pb.TickerBtcWidget_TimeFrame>;

    style?: CSSProperties;
}
interface Props extends FormWidgetTickerProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    get #txt() {
        const { formatMessage } = this.props.intl;

        return {
            ticker: formatMessage({ defaultMessage: 'Ticker' }),
            addScene: formatMessage({ defaultMessage: 'Add Scene' }),
            editScene: formatMessage({ defaultMessage: 'Edit Scene' }),
        };
    }

    #tickerTimeFrameToString = (x: pb.TickerBtcWidget_TimeFrame): string => {
        return pb.tickerTimeFrameToString(this.props.intl, x) ?? 'N/A';
    };

    render() {
        const {
            isOpen,
            isEdit,
            onClose,
            error,

            // Main
            widgetSize,
            timeFrame,

            style,
            intl,
        } = this.props;

        const { formatMessage } = intl;
        const txt = this.#txt;

        const form = (
            <Form className={css.form} style={style}>
                <WidgetSizeSelector field={widgetSize} />

                <Dropdown<pb.TickerBtcWidget_TimeFrame>
                    id={$('time-frame')}
                    label={formatMessage({ defaultMessage: 'Time Frame' })}
                    titleText={formatMessage({ defaultMessage: 'Time Frame' })}
                    items={pb.tickerTimeFrameOptions}
                    selectedItem={timeFrame.value ?? undefined}
                    itemToString={this.#tickerTimeFrameToString}
                    onChange={x => {
                        const v = x.selectedItem;
                        if (v != null) timeFrame.onChange(v);
                    }}
                    invalid={!!timeFrame.error}
                    invalidText={timeFrame.error}
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
                selectorPrimaryFocus="form input,button"
                // State
                size="sm"
                open={isOpen}
                // Heading
                title={txt.ticker}
                label={verb}
                // Cancel
                onClose={onClose}
                // Content
                children={form}
            />
        );
    }
}
export function FormWidgetTicker(props: FormWidgetTickerProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}

export function createTickerWidgetKind(data: FormPropsToValuesRec<FormWidgetTickerProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'tickerBtc',
            value: pb.create(pb.TickerBtcWidgetSchema, {
                timeFrame: data.timeFrame,
            }),
        },
    });
}
export function unpackTicketWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetTickerProps> {
    if (data.value?.case !== 'tickerBtc') throw new Error('Invalid widget kind');
    return {
        widgetSize,
        timeFrame: data.value.value.timeFrame,
    };
}
