import { FormattedMessage, useIntl } from 'react-intl';

// App
import * as pb from '@/proto';
import { getID } from '../const';
import { AutoSelected } from '@/lib/react';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { ModalCustom, InlineNotification, Button } from '@/components';
import { WidgetSizeSelector, type WidgetSizeSelectorProps, CheckYourScreenForPreview, BoundComboBox } from '../shared';
import { TextInput, Toggle } from '@carbon/react';

// styles
import css from '../shared.scss';

const $ = getID('remote-image-form').get;
export interface FormWidgetRemoteImageProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    url: iField<string>;
    refreshDurationSec: iField<number>;
    imageScaleMode: iField<number>;

    style?: CSSProperties;
}
export function FormWidgetRemoteImage(props: FormWidgetRemoteImageProps) {
    const intl = useIntl();
    const { formatMessage } = intl;

    const {
        isOpen,
        isEdit,
        onClose,
        error,

        // Main
        widgetSize,
        url,
        refreshDurationSec,
        imageScaleMode,

        style,
    } = props;

    const txt = {
        image: formatMessage({ defaultMessage: 'Image' }),
        addWidget: formatMessage({ defaultMessage: 'Add Widget' }),
        editWidget: formatMessage({ defaultMessage: 'Edit Widget' }),
    };
    const verb = isEdit ? txt.editWidget : txt.addWidget;

    const refreshIntervalOptions = pb
        .getRemoteImageRefreshIntervalOptions(intl)
        .map(x => ({ value: x.seconds, label: x.label }));

    const form = (
        <Form className={css.form} style={style}>
            <WidgetSizeSelector field={widgetSize} />

            <Rules />

            <TextInput
                id={$('url')}
                labelText={intl.formatMessage({ defaultMessage: 'URL' })}
                placeholder={intl.formatMessage({ defaultMessage: 'URL' })}
                value={url.value || ''}
                onChange={e => url.onChange?.(e.target.value)}
                invalid={!!url.error}
                invalidText={url.error}
                helperText={intl.formatMessage({ defaultMessage: 'JPG or PNG' })}
            />

            <BoundComboBox<number>
                {...refreshDurationSec}
                id={$('refresh-duration')}
                labelText={formatMessage({ defaultMessage: 'Refresh Duration' })}
                placeholderText="---"
                items={refreshIntervalOptions}
            />

            <Toggle
                id={$('image-scale-mode')}
                labelText={formatMessage({ defaultMessage: 'Fill mode' })}
                labelA={formatMessage({ defaultMessage: 'Fit (black bars)' })}
                labelB={formatMessage({ defaultMessage: 'Fill (crop edges)' })}
                toggled={imageScaleMode.value === pb.RemoteImageWidget_ImageScaleMode.FILL}
                onToggle={v =>
                    imageScaleMode.onChange?.(
                        v ? pb.RemoteImageWidget_ImageScaleMode.FILL : pb.RemoteImageWidget_ImageScaleMode.FIT,
                    )
                }
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

    return (
        <ModalCustom
            id={$('dialog')}
            className={css.modal}
            selectorPrimaryFocus="[role='combobox']"
            // State
            size="sm"
            open={isOpen}
            // Heading
            title={txt.image}
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

function Rules() {
    const { formatMessage } = useIntl();
    const b = (chunks: ReactNode) => <strong key="b">{chunks}</strong>;

    return (
        <div className={css.rules}>
            <h2 children={formatMessage({ defaultMessage: 'Requirements:' })} />

            <ul>
                <FormattedMessage tagName="li" defaultMessage="Wi-Fi connection is working" />
                <FormattedMessage
                    tagName="li"
                    defaultMessage="HTTP(s) server (specified by URL) is working (e.g. endpoint is accessible, no HTTP errors)"
                />
                <FormattedMessage
                    tagName="li"
                    defaultMessage="Image has <b>PNG</b> or <b>JPEG</b> format"
                    values={{ b }}
                />
                <FormattedMessage
                    tagName="li"
                    defaultMessage="Image is automatically scaled to fit the widget while preserving aspect ratio (up to 4K resolution)"
                />
            </ul>

            <p
                children={formatMessage({
                    defaultMessage:
                        'We suggest you to choose the highest value of refresh duration which makes sense for your usecase.',
                })}
            />
            <p
                children={formatMessage({
                    defaultMessage:
                        'If your image is static, it’s not necessary to refresh it every 10 seconds. This way you can avoid unnecessary server load and network traffic.',
                })}
            />

            <h3 children={formatMessage({ defaultMessage: 'Advanced usage:' })} />

            <p
                children={formatMessage({
                    defaultMessage:
                        'If you have a custom server which generates images dynamically, you can use following placeholders in the URL to insert the widget dimensions:',
                })}
            />
            <ul>
                <li>
                    <AutoSelected kind="code" children="{{width}}" />
                </li>
                <li>
                    <AutoSelected kind="code" children="{{height}}" />
                </li>
            </ul>
        </div>
    );
}

export function createRemoteImageWidgetKind(data: FormPropsToValuesRec<FormWidgetRemoteImageProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'remoteImage',
            value: pb.create(pb.RemoteImageWidgetSchema, {
                url: data.url,
                refreshDurationSec: data.refreshDurationSec,
                imageScaleMode: data.imageScaleMode,
            }),
        },
    });
}
export function unpackRemoteImageWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetRemoteImageProps> {
    if (data.value?.case !== 'remoteImage') throw new Error('Invalid widget kind');
    return {
        widgetSize,
        url: data.value.value.url,
        refreshDurationSec: data.value.value.refreshDurationSec,
        imageScaleMode: data.value.value.imageScaleMode,
    };
}
