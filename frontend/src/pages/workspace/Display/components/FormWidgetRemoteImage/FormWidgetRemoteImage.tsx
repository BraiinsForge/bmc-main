import { FormattedMessage, useIntl } from 'react-intl';

// App
import * as pb from '@/proto';
import { getID } from '../const';
import { AutoSelected } from '@/lib/react';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { ModalCustom, InlineNotification } from '@/components';
import { WidgetSizeSelector, type WidgetSizeSelectorProps, CheckYourScreenForPreview, BoundComboBox } from '../shared';
import { TextInput } from '@carbon/react';

// styles
import css from '../shared.scss';

const $ = getID('remote-image-form').get;
type Sizes = Record<Exclude<pb.WidgetSize, 0>, string>;

export interface FormWidgetRemoteImageProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    url: iField<string>;
    refreshDurationSec: iField<number>;

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

        style,
    } = props;

    const txt = {
        remoteImage: formatMessage({ defaultMessage: 'Remote Image' }),
        addScene: formatMessage({ defaultMessage: 'Add Scene' }),
        editScene: formatMessage({ defaultMessage: 'Edit Scene' }),
    };
    const sizes: Sizes = {
        [pb.WidgetSize.SMALL]: '317x238',
        [pb.WidgetSize.MEDIUM]: '638x238',
        [pb.WidgetSize.LARGE]: '638x480',
        [pb.WidgetSize.FULL]: '1280x480',
    };
    const verb = isEdit ? txt.editScene : txt.addScene;

    const refreshIntervalOptions = pb
        .getRemoteImageRefreshIntervalOptions(intl)
        .map(x => ({ value: x.seconds, label: x.label }));

    const form = (
        <Form className={css.form} style={style}>
            <WidgetSizeSelector field={widgetSize} />

            <Rules {...sizes} />

            <TextInput
                id={$('url')}
                labelText={intl.formatMessage({ defaultMessage: 'URL' })}
                placeholder={intl.formatMessage({ defaultMessage: 'URL' })}
                value={url.value || ''}
                onChange={e => url.onChange(e.target.value)}
                invalid={!!url.error}
                invalidText={url.error}
                helperText={
                    <FormattedMessage
                        defaultMessage="JPG or PNG; {size}"
                        values={{ size: widgetSize?.value ? sizes[widgetSize.value] : '' }}
                    />
                }
            />

            <BoundComboBox<number>
                {...refreshDurationSec}
                id={$('refresh-duration')}
                labelText={formatMessage({ defaultMessage: 'Refresh Duration' })}
                placeholderText="---"
                items={refreshIntervalOptions}
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
            title={txt.remoteImage}
            label={verb}
            // Cancel
            onClose={onClose}
            // Content
            children={form}
        />
    );
}

function Rules(props: Sizes) {
    const { formatMessage } = useIntl();
    const b = (chunks: ReactNode) => <strong children={chunks} />;

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
                    defaultMessage="Image has exact resolution for chosen widget size:"
                    values={{ b }}
                />
                <ul>
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="<b>small</b>: 317x238px"
                        values={{ size: props[pb.WidgetSize.SMALL], b }}
                    />
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="<b>medium</b>: 638x238px"
                        values={{ size: props[pb.WidgetSize.MEDIUM], b }}
                    />
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="<b>large</b>: 638x480px"
                        values={{ size: props[pb.WidgetSize.LARGE], b }}
                    />
                    <FormattedMessage
                        tagName="li"
                        defaultMessage="<b>fullscreen</b>: 1280x480px"
                        values={{ size: props[pb.WidgetSize.FULL], b }}
                    />
                </ul>
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
                        'If you have a custom server which generates images dynamically, you can use following query parameters from the request to prepare image with correct resolution:',
                })}
            />
            <ul>
                <li>
                    <AutoSelected kind="code" children="deck_image_width" />
                </li>
                <li>
                    <AutoSelected kind="code" children="deck_image_height" />
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
    };
}
